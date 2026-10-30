//! Pure, I/O-free logic shared by all Hece crates: vocabulary, hallucination filter,
//! confidence, negation guard and shared types.

pub mod text;
pub mod types;

pub use text::{fold, normalize};
pub use types::{Segment, Transcript};
