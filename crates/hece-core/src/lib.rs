//! Pure, I/O-free logic shared by all Hece crates: vocabulary, hallucination filter,
//! confidence, negation guard and shared types.

pub mod text;
pub mod types;
pub mod vocab;

pub use text::{fold, normalize};
pub use types::{Segment, Transcript};
pub use vocab::{
    DEFAULT_PROMPT_PREFIX, DEFAULT_PROMPT_TOKENS, MAX_PROMPT_TOKENS, MAX_TERM_CHARS, Vocabulary,
    clean_term,
};
