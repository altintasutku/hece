//! The recording overlay's look, independent of any window system: a phase machine driven by
//! the daemon, a level meter fed with the capture RMS, and drawing into a pixmap.

mod meter;
mod raster;
mod scene;
mod sphere;

pub use meter::{Meter, Voice};
pub use scene::{EXIT, Scene};

/// How a dictation ended, for the overlay's exit animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The text was pasted.
    Inserted,
    /// Nothing was heard.
    NothingHeard,
    /// The text was not pasted (focus changed); it is on the clipboard.
    NotPasted,
    /// Failed, aborted or cancelled.
    Failed,
}

/// What the overlay shows; the daemon sets it after every event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Nothing under way: hidden (after any exit animation).
    Idle,
    /// Recording started, the microphone is not live yet.
    Opening,
    /// The microphone is live.
    Listening,
    /// Recording stopped; a dictation is being transcribed.
    Transcribing,
    /// Every dictation is done; the last one ended like this.
    Finished(Outcome),
}

const PHASES: [Phase; 8] = [
    Phase::Idle,
    Phase::Opening,
    Phase::Listening,
    Phase::Transcribing,
    Phase::Finished(Outcome::Inserted),
    Phase::Finished(Outcome::NothingHeard),
    Phase::Finished(Outcome::NotPasted),
    Phase::Finished(Outcome::Failed),
];

impl Phase {
    /// A number for passing the phase through a window message.
    #[must_use]
    pub fn to_raw(self) -> usize {
        PHASES.iter().position(|p| *p == self).unwrap_or(0)
    }

    /// The phase for [`Phase::to_raw`]'s number; unknown numbers are [`Phase::Idle`].
    #[must_use]
    pub fn from_raw(raw: usize) -> Self {
        PHASES.get(raw).copied().unwrap_or(Self::Idle)
    }
}
