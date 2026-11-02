//! Long recordings as chunks cut at natural pauses (see `hece_core::chunker`). Measured on a
//! synthetic 3-minute set: whole-file WER 6.4% with 5 repeated sentences, chunked 0.7% with
//! none, and about half the time. Chunks go to the primary provider only; a chunk that fails
//! twice sends the whole recording through the fallback chain instead, so one dictation is
//! never mixed from two providers.

use std::ops::Range;
use std::sync::Arc;

use hece_core::chunker::{ChunkPolicy, next_cut};
use hece_core::{Transcript, Vocabulary};
use hece_providers::{Outcome, ProviderError, ProviderRequest, Providers};

/// Audio shorter than this is transcribed in one piece.
pub const CHUNK_ABOVE_SAMPLES: usize = 30 * SAMPLE_RATE as usize;
const SAMPLE_RATE: u32 = 16_000;

/// A chunk's place in the recording and, if it was already transcribed, its result.
pub type Part = (Range<usize>, Option<Result<Transcript, ProviderError>>);

/// What every chunk request carries.
#[derive(Debug, Clone)]
pub struct Params {
    pub language: Option<String>,
    pub vocabulary: Option<Arc<Vocabulary>>,
    pub prompt_tokens: usize,
}

impl Params {
    fn request<'a>(&'a self, audio: &'a [i16]) -> ProviderRequest<'a> {
        ProviderRequest {
            audio,
            language: self.language.as_deref(),
            vocabulary: self.vocabulary.as_deref(),
            prompt_prefix: hece_core::DEFAULT_PROMPT_PREFIX,
            prompt_tokens: self.prompt_tokens,
        }
    }
}

/// Splits 16 kHz audio the way live chunking would.
pub fn split(audio: &[i16]) -> Vec<Range<usize>> {
    let policy = ChunkPolicy::default();
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < audio.len() {
        let mono: Vec<f32> = audio[start..]
            .iter()
            .map(|s| f32::from(*s) / 32_768.0)
            .collect();
        let end = next_cut(&mono, SAMPLE_RATE, &policy).map_or(audio.len(), |cut| start + cut);
        ranges.push(start..end);
        start = end;
    }
    ranges
}

/// Joins chunk transcripts; segment times are shifted to the whole recording.
#[expect(
    clippy::cast_precision_loss,
    reason = "sample offsets are far below 2^24 s"
)]
fn combine(parts: Vec<(usize, Transcript)>, provider: &str) -> Transcript {
    let mut text = Vec::new();
    let mut segments: Option<Vec<_>> = Some(Vec::new());
    let mut language = String::new();
    for (start, t) in parts {
        let offset = start as f32 / SAMPLE_RATE as f32;
        if language.is_empty() {
            language.clone_from(&t.language);
        }
        let trimmed = t.text.trim();
        if !trimmed.is_empty() {
            text.push(trimmed.to_owned());
        }
        segments = match (segments, t.segments) {
            (Some(mut all), Some(segs)) => {
                all.extend(segs.into_iter().map(|mut s| {
                    s.start_s += offset;
                    s.end_s += offset;
                    s
                }));
                Some(all)
            }
            _ => None,
        };
    }
    Transcript {
        text: text.join(" "),
        segments,
        language,
        provider: provider.to_owned(),
        low_confidence: false,
    }
}

/// Transcribes `audio` from chunks. `parts` are the chunk ranges in order, each with its result
/// when it was already transcribed (while recording); missing ones are transcribed here. A
/// chunk failing twice falls back to the whole recording through `providers`.
pub async fn assemble(
    providers: &Providers,
    audio: &[i16],
    parts: Vec<Part>,
    params: &Params,
) -> Result<Outcome, ProviderError> {
    let primary = providers.primary();
    let mut done = Vec::with_capacity(parts.len());
    for (range, result) in parts {
        let Some(piece) = audio.get(range.clone()) else {
            continue;
        };
        if piece.is_empty() {
            continue;
        }
        let first = match result {
            Some(r) => r,
            None => primary.transcribe(params.request(piece)).await,
        };
        let result = match first {
            Ok(t) => Ok(t),
            Err(e) => {
                tracing::warn!(code = e.code(), "chunk failed, retrying once");
                primary.transcribe(params.request(piece)).await
            }
        };
        match result {
            Ok(t) => done.push((range.start, t)),
            Err(e) => {
                tracing::warn!(
                    code = e.code(),
                    "chunk failed twice, transcribing the whole recording"
                );
                return providers.transcribe(params.request(audio)).await;
            }
        }
    }
    Ok(Outcome {
        transcript: combine(done, primary.name()),
        fell_back: None,
    })
}

/// Transcribes `audio`, in chunks when it is long.
pub async fn transcribe(
    providers: &Providers,
    audio: &[i16],
    params: &Params,
) -> Result<Outcome, ProviderError> {
    if audio.len() <= CHUNK_ABOVE_SAMPLES {
        return providers.transcribe(params.request(audio)).await;
    }
    let parts = split(audio).into_iter().map(|r| (r, None)).collect();
    assemble(providers, audio, parts, params).await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use hece_core::Segment;
    use hece_core::ipc::MAX_AUDIO_SAMPLES;
    use hece_providers::{BoxFuture, Capabilities, Provider, VocabularyStyle};

    use super::*;

    /// Fails the first `fail` calls, then returns the chunk length.
    #[derive(Debug)]
    struct Flaky {
        name: &'static str,
        fail: usize,
        calls: AtomicUsize,
        caps: Capabilities,
    }

    fn flaky(name: &'static str, fail: usize) -> Arc<Flaky> {
        Arc::new(Flaky {
            name,
            fail,
            calls: AtomicUsize::new(0),
            caps: Capabilities {
                local: false,
                vocabulary: VocabularyStyle::Unsupported,
                segments: true,
                max_audio_samples: MAX_AUDIO_SAMPLES,
                language_detection: true,
            },
        })
    }

    impl Provider for Flaky {
        fn name(&self) -> &'static str {
            self.name
        }
        fn capabilities(&self) -> &Capabilities {
            &self.caps
        }
        fn transcribe<'a>(
            &'a self,
            req: ProviderRequest<'a>,
        ) -> BoxFuture<'a, Result<Transcript, ProviderError>> {
            Box::pin(async move {
                if self.calls.fetch_add(1, Ordering::SeqCst) < self.fail {
                    return Err(ProviderError::Network {
                        provider: self.name.to_owned(),
                    });
                }
                Ok(Transcript {
                    text: format!("{}:{}", self.name, req.audio.len()),
                    segments: Some(vec![Segment {
                        text: String::new(),
                        start_s: 0.0,
                        end_s: 1.0,
                        no_speech_prob: 0.0,
                        avg_logprob: 0.0,
                    }]),
                    language: "tr".to_owned(),
                    provider: self.name.to_owned(),
                    low_confidence: false,
                })
            })
        }
    }

    fn params() -> Params {
        Params {
            language: None,
            vocabulary: None,
            prompt_tokens: 0,
        }
    }

    #[tokio::test]
    async fn chunks_are_joined_in_order_with_shifted_segments() {
        let providers = Providers::new(flaky("a", 0), None, false).unwrap();
        let audio = vec![0i16; 48_000];
        let parts = vec![(0..16_000, None), (16_000..48_000, None)];
        let out = assemble(&providers, &audio, parts, &params())
            .await
            .unwrap();
        assert_eq!(out.transcript.text, "a:16000 a:32000");
        let segs = out.transcript.segments.unwrap();
        assert!((segs[1].start_s - 1.0).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn a_chunk_is_retried_once() {
        let primary = flaky("a", 1);
        let providers = Providers::new(primary.clone(), None, false).unwrap();
        let audio = vec![0i16; 32_000];
        let out = assemble(&providers, &audio, vec![(0..32_000, None)], &params())
            .await
            .unwrap();
        assert_eq!(out.transcript.text, "a:32000");
        assert_eq!(primary.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_chunk_failing_twice_sends_the_whole_recording_to_the_fallback() {
        let providers = Providers::new(flaky("a", usize::MAX), Some(flaky("b", 0)), false).unwrap();
        let audio = vec![0i16; 48_000];
        let parts = vec![(0..16_000, None), (16_000..48_000, None)];
        let out = assemble(&providers, &audio, parts, &params())
            .await
            .unwrap();
        assert_eq!(out.transcript.text, "b:48000");
        assert_eq!(out.fell_back.unwrap().to, "b");
    }
}
