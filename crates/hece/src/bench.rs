//! Benchmark runner: list loading, WAV reading, and the transcription loop.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use hece_core::Vocabulary;
use hece_core::metrics::{Scored, score_long, score_short};
use hece_providers::engine_client::{
    EngineClient, EngineClientError, EngineConfig, ProcessSpawner,
};

use crate::cli::BenchArgs;
use crate::output;

/// Name of the list file inside a bench directory.
pub const LIST_FILE: &str = "liste.tsv";
const SAMPLE_RATE: u32 = 16_000;

/// Failures while preparing a bench run. Messages carry file names and line numbers only,
/// never reference text.
#[derive(Debug, thiserror::Error)]
pub enum BenchError {
    /// The list file could not be read.
    #[error("cannot read {0}")]
    ListUnreadable(PathBuf),
    /// A list line has no tab separator or an empty field.
    #[error("{file}:{line}: expected `<wav file><TAB><reference>`")]
    BadLine {
        /// List file.
        file: PathBuf,
        /// 1-based line number.
        line: usize,
    },
    /// An audio file listed in the list does not exist.
    #[error("{file}:{line}: audio file {audio} not found")]
    MissingAudio {
        /// List file.
        file: PathBuf,
        /// 1-based line number.
        line: usize,
        /// Audio file name.
        audio: String,
    },
    /// An audio file cannot be decoded.
    #[error("cannot read audio {}", location(file, *line))]
    BadWav {
        /// Audio file.
        file: PathBuf,
        /// 1-based line of the list that names it, when known.
        line: Option<usize>,
    },
    /// An audio file is not 16 kHz mono 16-bit PCM.
    #[error("{}: audio must be 16 kHz, mono, 16-bit PCM", location(file, *line))]
    WrongFormat {
        /// Audio file.
        file: PathBuf,
        /// 1-based line of the list that names it, when known.
        line: Option<usize>,
    },
    /// The directory holds no entries.
    #[error("{0} lists no recordings")]
    Empty(PathBuf),
}

fn location(file: &Path, line: Option<usize>) -> String {
    line.map_or_else(
        || file.display().to_string(),
        |n| format!("{} (line {n})", file.display()),
    )
}

/// Something that turns audio into text; the error is a stable code.
pub trait Transcriber {
    /// Transcribes 16 kHz mono samples.
    fn transcribe(&self, audio: &[i16]) -> impl Future<Output = Result<String, String>> + Send;
}

/// Transcriber backed by the engine worker.
#[derive(Debug)]
pub struct EngineTranscriber<'a> {
    /// Engine client.
    pub client: &'a EngineClient,
    /// Spoken language.
    pub language: &'a str,
    /// Custom vocabulary, if any.
    pub vocabulary: Option<&'a Vocabulary>,
    /// Prompt prefix.
    pub prefix: &'a str,
    /// Prompt token budget.
    pub prompt_tokens: usize,
}

impl Transcriber for EngineTranscriber<'_> {
    async fn transcribe(&self, audio: &[i16]) -> Result<String, String> {
        self.client
            .transcribe(
                audio,
                Some(self.language),
                self.vocabulary,
                self.prefix,
                self.prompt_tokens,
            )
            .await
            .map(|t| t.text)
            .map_err(|e: EngineClientError| e.code().to_owned())
    }
}

fn check_spec(path: &Path, line: Option<usize>, spec: hound::WavSpec) -> Result<(), BenchError> {
    if spec.sample_rate == SAMPLE_RATE
        && spec.channels == 1
        && spec.bits_per_sample == 16
        && spec.sample_format == hound::SampleFormat::Int
    {
        Ok(())
    } else {
        Err(BenchError::WrongFormat {
            file: path.to_owned(),
            line,
        })
    }
}

/// Reads a 16 kHz mono 16-bit PCM WAV file.
pub fn read_wav(path: &Path) -> Result<Vec<i16>, BenchError> {
    let bad = || BenchError::BadWav {
        file: path.to_owned(),
        line: None,
    };
    let mut reader = hound::WavReader::open(path).map_err(|_| bad())?;
    check_spec(path, None, reader.spec())?;
    reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| bad())
}

/// Loads `<dir>/liste.tsv` into `(audio path, reference)` rows. Every audio file must exist
/// and have the right format, so a bad set aborts before any transcription starts.
pub fn load_list(dir: &Path) -> Result<Vec<(PathBuf, String)>, BenchError> {
    let list = dir.join(LIST_FILE);
    let text =
        std::fs::read_to_string(&list).map_err(|_| BenchError::ListUnreadable(list.clone()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let mut rows = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = idx + 1;
        if raw.trim().is_empty() {
            continue;
        }
        let bad = || BenchError::BadLine {
            file: list.clone(),
            line,
        };
        let (name, reference) = raw.split_once('\t').ok_or_else(bad)?;
        let (name, reference) = (name.trim(), reference.trim());
        if name.is_empty() || reference.is_empty() {
            return Err(bad());
        }
        let path = dir.join(name);
        if !path.is_file() {
            return Err(BenchError::MissingAudio {
                file: list.clone(),
                line,
                audio: name.to_owned(),
            });
        }
        let reader = hound::WavReader::open(&path).map_err(|_| BenchError::BadWav {
            file: path.clone(),
            line: Some(line),
        })?;
        check_spec(&path, Some(line), reader.spec())?;
        rows.push((path, reference.to_owned()));
    }
    if rows.is_empty() {
        return Err(BenchError::Empty(list));
    }
    Ok(rows)
}

/// Transcribes every row in order; a failing file is recorded as `(file name, code)` and
/// skipped. Returns the scored rows and the failures. Warm-up is the caller's job.
pub async fn run<T: Transcriber>(
    rows: &[(PathBuf, String)],
    transcriber: &T,
) -> (Vec<Scored>, Vec<(String, String)>) {
    let mut scored = Vec::with_capacity(rows.len());
    let mut failures = Vec::new();
    for (path, reference) in rows {
        let file = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let Ok(audio) = read_wav(path) else {
            failures.push((file, "bad_audio".to_owned()));
            continue;
        };
        let started = Instant::now();
        match transcriber.transcribe(&audio).await {
            Ok(hypothesis) => scored.push(Scored {
                file,
                reference: reference.clone(),
                hypothesis,
                seconds: started.elapsed().as_secs_f64(),
            }),
            Err(code) => failures.push((file, code)),
        }
    }
    (scored, failures)
}

/// Runs `hece bench`: prepares both sets, warms the engine, scores, prints, shuts the worker down.
/// Returns whether any file failed.
pub async fn execute(args: &BenchArgs) -> anyhow::Result<bool> {
    let terms = match &args.vocab {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| {
                anyhow::anyhow!(
                    "cannot read vocabulary file {}: {:?}",
                    path.display(),
                    e.kind()
                )
            })?;
            Some(Vocabulary::parse(&text))
        }
        None => None,
    };
    let term_list = terms.as_ref().map_or_else(Vec::new, |v| v.terms.clone());

    // Validate everything before the engine starts.
    let main_rows = load_list(&args.dir)?;
    let short_rows = args.short.as_deref().map(load_list).transpose()?;

    let mut cfg = EngineConfig::default_for(args.model.clone())?;
    cfg.engine_path.clone_from(&args.engine);
    let client = EngineClient::new(cfg, Arc::new(ProcessSpawner));
    let outcome = measure(
        args,
        &client,
        terms.as_ref(),
        &term_list,
        &main_rows,
        short_rows.as_deref(),
    )
    .await;
    // Do not leave a loaded GPU worker behind for its idle TTL.
    let shutdown = client.shutdown().await;
    let any_failed = outcome?;
    shutdown?;
    Ok(any_failed)
}

async fn measure(
    args: &BenchArgs,
    client: &EngineClient,
    vocabulary: Option<&Vocabulary>,
    terms: &[String],
    main_rows: &[(PathBuf, String)],
    short_rows: Option<&[(PathBuf, String)]>,
) -> anyhow::Result<bool> {
    let transcriber = EngineTranscriber {
        client,
        language: &args.language,
        vocabulary,
        prefix: &args.prefix,
        prompt_tokens: args.prompt_tokens,
    };
    client.prewarm().await?;
    let mut any_failed = false;
    let mut report_failures = |failures: &[(String, String)]| {
        any_failed |= !failures.is_empty();
        for (file, code) in failures {
            output::failure(file, code);
        }
    };

    output::line(&output::header_line());
    if args.long {
        let short_report = match short_rows {
            Some(rows) => {
                let (scored, failures) = run(rows, &transcriber).await;
                report_failures(&failures);
                Some(score_short(&scored, failures.len(), terms))
            }
            None => None,
        };
        let (scored, failures) = run(main_rows, &transcriber).await;
        report_failures(&failures);
        let report = score_long(&scored, failures.len(), terms);
        if let Some(s) = &short_report {
            output::line(&output::format_row("short", s));
        }
        output::line(&output::format_row("long", &report.short));
        for l in output::format_long_lines(
            &report,
            short_report.as_ref().filter(|s| s.files > 0).map(|s| s.wer),
        ) {
            output::line(&l);
        }
    } else {
        let (scored, failures) = run(main_rows, &transcriber).await;
        report_failures(&failures);
        let report = score_short(&scored, failures.len(), terms);
        output::line(&output::format_row("short", &report));
    }
    Ok(any_failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hece-bench-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_wav(path: &Path, rate: u32, channels: u16) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for _ in 0..(160 * usize::from(channels)) {
            w.write_sample(0i16).unwrap();
        }
        w.finalize().unwrap();
    }

    #[test]
    fn line_without_tab_names_line_not_text() {
        let dir = temp_dir();
        write_wav(&dir.join("a.wav"), 16_000, 1);
        std::fs::write(
            dir.join(LIST_FILE),
            "a.wav\tfirst secret sentence\nno tab SECRETTEXT here\n",
        )
        .unwrap();
        let err = load_list(&dir).unwrap_err().to_string();
        assert!(err.contains(":2:"), "{err}");
        assert!(!err.contains("SECRETTEXT"), "{err}");
    }

    #[test]
    fn missing_list_and_missing_audio_are_errors() {
        let dir = temp_dir();
        assert!(matches!(
            load_list(&dir),
            Err(BenchError::ListUnreadable(_))
        ));
        std::fs::write(dir.join(LIST_FILE), "gone.wav\tsome text\n").unwrap();
        let err = load_list(&dir).unwrap_err().to_string();
        assert!(err.contains("gone.wav") && err.contains(":1:"), "{err}");
        assert!(!err.contains("some text"));
    }

    #[test]
    fn load_list_rejects_bad_format_before_run() {
        let dir = temp_dir();
        write_wav(&dir.join("hi.wav"), 44_100, 1);
        std::fs::write(dir.join(LIST_FILE), "hi.wav\ttext\n").unwrap();
        let err = load_list(&dir).unwrap_err();
        assert!(matches!(err, BenchError::WrongFormat { line: Some(1), .. }));
        let msg = err.to_string();
        assert!(msg.contains("hi.wav") && msg.contains("line 1"), "{msg}");
        assert!(!msg.contains("text"), "{msg}");
    }

    #[test]
    fn read_wav_rejects_float_and_24_bit() {
        let dir = temp_dir();
        let float = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut w = hound::WavWriter::create(dir.join("f.wav"), float).unwrap();
        w.write_sample(0.0f32).unwrap();
        w.finalize().unwrap();
        let deep = hound::WavSpec {
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
            ..float
        };
        let mut w = hound::WavWriter::create(dir.join("d.wav"), deep).unwrap();
        w.write_sample(0i32).unwrap();
        w.finalize().unwrap();
        for name in ["f.wav", "d.wav"] {
            assert!(matches!(
                read_wav(&dir.join(name)),
                Err(BenchError::WrongFormat { .. })
            ));
        }
    }

    #[test]
    fn bom_in_list_is_ignored() {
        let dir = temp_dir();
        write_wav(&dir.join("a.wav"), 16_000, 1);
        std::fs::write(dir.join(LIST_FILE), "\u{feff}a.wav\tsome text\n").unwrap();
        assert_eq!(load_list(&dir).unwrap().len(), 1);
    }

    #[test]
    fn read_wav_accepts_16k_mono_and_rejects_others() {
        let dir = temp_dir();
        write_wav(&dir.join("ok.wav"), 16_000, 1);
        write_wav(&dir.join("rate.wav"), 44_100, 1);
        write_wav(&dir.join("stereo.wav"), 16_000, 2);
        assert_eq!(read_wav(&dir.join("ok.wav")).unwrap().len(), 160);
        assert!(matches!(
            read_wav(&dir.join("rate.wav")),
            Err(BenchError::WrongFormat { .. })
        ));
        assert!(matches!(
            read_wav(&dir.join("stereo.wav")),
            Err(BenchError::WrongFormat { .. })
        ));
    }

    struct FailSecond(AtomicUsize);

    impl Transcriber for FailSecond {
        fn transcribe(
            &self,
            _audio: &[i16],
        ) -> impl Future<Output = Result<String, String>> + Send {
            std::future::ready(if self.0.fetch_add(1, Ordering::SeqCst) == 1 {
                Err("engine_crashed".to_owned())
            } else {
                Ok("merhaba".to_owned())
            })
        }
    }

    #[tokio::test]
    async fn run_counts_failures_and_continues() {
        let dir = temp_dir();
        let mut rows = Vec::new();
        for n in ["a", "b", "c"] {
            let path = dir.join(format!("{n}.wav"));
            write_wav(&path, 16_000, 1);
            rows.push((path, "merhaba".to_owned()));
        }
        let (scored, failures) = run(&rows, &FailSecond(AtomicUsize::new(0))).await;
        assert_eq!(
            failures,
            vec![("b.wav".to_owned(), "engine_crashed".to_owned())]
        );
        assert_eq!(scored.len(), 2);
        assert_eq!(scored[1].file, "c.wav");
    }
}
