//! Shared transcript types.

use serde::{Deserialize, Serialize};

/// One timed piece of a transcription, with the engine's confidence signals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    /// Transcribed text of the segment.
    pub text: String,
    /// Start time in seconds.
    pub start_s: f32,
    /// End time in seconds.
    pub end_s: f32,
    /// Probability that the segment contains no speech.
    pub no_speech_prob: f32,
    /// Average log probability of the segment's tokens.
    pub avg_logprob: f32,
}

impl Segment {
    /// Segment length in seconds, never negative.
    #[must_use]
    pub fn duration_s(&self) -> f32 {
        (self.end_s - self.start_s).max(0.0)
    }
}

/// A full transcription result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    /// Final text.
    pub text: String,
    /// Per-segment detail, when the provider returns it.
    pub segments: Option<Vec<Segment>>,
    /// Detected or requested language code; empty when no speech was found and no language was
    /// requested.
    pub language: String,
    /// Name of the provider that produced the transcript.
    pub provider: String,
    /// Whether the confidence layer flagged the result as unreliable.
    pub low_confidence: bool,
}

impl Transcript {
    /// Applies the quality layers: with segments, drops hallucinated or silent ones and sets
    /// [`Transcript::low_confidence`]; without segments, empties a text that is a known
    /// hallucination as a whole.
    #[must_use]
    pub fn with_quality_layers(mut self) -> Self {
        match self.segments.take() {
            Some(segs) => {
                let filtered = crate::hallucination::filter_segments(segs);
                self.low_confidence = crate::confidence::is_low_confidence(&filtered.kept);
                self.text = filtered.text;
                self.segments = Some(filtered.kept);
            }
            None if crate::hallucination::is_known_hallucination(&self.text) => self.text.clear(),
            None => {}
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transcript(text: &str, segments: Option<Vec<Segment>>) -> Transcript {
        Transcript {
            text: text.to_owned(),
            segments,
            language: "tr".to_owned(),
            provider: "test".to_owned(),
            low_confidence: false,
        }
    }

    #[test]
    fn quality_layers_drop_silent_segments_and_flag_confidence() {
        let mut silent = segment(1.5, 2.0);
        silent.text = " Altyazı M.K.".to_owned();
        let mut unsure = segment(0.0, 1.5);
        unsure.avg_logprob = -0.9;
        let t = transcript("ignored", Some(vec![unsure, silent])).with_quality_layers();
        assert_eq!(t.text, "merhaba");
        assert!(t.low_confidence);
        assert_eq!(t.segments.map(|s| s.len()), Some(1));
    }

    #[test]
    fn quality_layers_empty_a_segmentless_hallucination() {
        assert_eq!(
            transcript("Altyazı M.K.", None).with_quality_layers().text,
            ""
        );
        assert_eq!(
            transcript("merhaba", None).with_quality_layers().text,
            "merhaba"
        );
    }

    fn segment(start_s: f32, end_s: f32) -> Segment {
        Segment {
            text: "merhaba".to_owned(),
            start_s,
            end_s,
            no_speech_prob: 0.1,
            avg_logprob: -0.2,
        }
    }

    #[test]
    fn segment_json_round_trip() {
        let original = segment(0.0, 1.5);
        let json = serde_json::to_string(&original).unwrap();
        let back: Segment = serde_json::from_str(&json).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn duration_is_never_negative() {
        assert!(segment(1.0, 0.5).duration_s().abs() < f32::EPSILON);
        assert!((segment(0.5, 1.5).duration_s() - 1.0).abs() < f32::EPSILON);
    }
}
