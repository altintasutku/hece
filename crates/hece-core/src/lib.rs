//! Pure, I/O-free logic shared by all Hece crates: vocabulary, hallucination filter,
//! confidence, negation guard and shared types.

pub mod confidence;
pub mod guard;
pub mod hallucination;
pub mod ipc;
pub mod metrics;
pub mod text;
pub mod textnorm;
pub mod types;
pub mod vocab;

pub use confidence::{LOW_CONFIDENCE_LOGPROB, is_low_confidence};
pub use guard::{GuardVerdict, RejectReason, cleanup_guard, is_negation_token, negation_count};
pub use hallucination::{
    Filtered, NO_SPEECH_THRESHOLD, SHORT_SEGMENT_MAX_WORDS, filter_segments, is_known_hallucination,
};
pub use text::{fold, normalize};
pub use types::{Segment, Transcript};
pub use vocab::{
    DEFAULT_PROMPT_PREFIX, DEFAULT_PROMPT_TOKENS, MAX_PROMPT_TOKENS, MAX_TERM_CHARS, Vocabulary,
    clean_term,
};
