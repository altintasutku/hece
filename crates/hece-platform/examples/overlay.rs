//! Manual check of the recording overlay: cycles through a whole dictation (opening, speech with
//! pauses, transcribing, each ending) with a synthetic level. Keep another window focused and
//! type while it runs: the caret must never leave it. Ctrl+C ends it.

#[cfg(windows)]
fn main() {
    use std::time::{Duration, Instant};

    use hece_platform::Overlay;
    use hece_platform::overlay::{Outcome, Phase};

    let started = Instant::now();
    // About -20 dBFS while "speaking", in syllable-like bursts; silent in the pauses.
    let level = move || {
        let t = started.elapsed().as_secs_f32();
        let speaking = (t % 6.0) < 4.0;
        let syllable = ((t * 9.0).sin() * 0.5 + (t * 3.7).sin() * 0.4 + 0.35).max(0.0);
        if speaking { 0.1 * syllable } else { 0.0 }
    };
    let Ok(overlay) = Overlay::start(level) else {
        return;
    };
    let endings = [
        Outcome::Inserted,
        Outcome::NothingHeard,
        Outcome::NotPasted,
        Outcome::Failed,
    ];
    for ending in endings.iter().cycle() {
        for (phase, secs) in [
            (Phase::Opening, 1),
            (Phase::Listening, 8),
            (Phase::Transcribing, 2),
            (Phase::Finished(*ending), 1),
            (Phase::Idle, 2),
        ] {
            overlay.set_phase(phase);
            std::thread::sleep(Duration::from_secs(secs));
        }
    }
}

#[cfg(not(windows))]
fn main() {}
