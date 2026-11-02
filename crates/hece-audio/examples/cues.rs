//! Plays the start, stop and cancel cues once on the default output device.
//!
//! `cargo run -p hece-audio --example cues -- [volume 0..1]`
#![allow(clippy::print_stdout, reason = "example output")]
#![allow(clippy::unwrap_used, reason = "example")]

use hece_audio::Cue;
use hece_audio::cue::play_blocking;

fn main() {
    let volume: f32 = std::env::args().nth(1).map_or(0.5, |v| v.parse().unwrap());
    for cue in [Cue::Start, Cue::Stop, Cue::Cancel] {
        let played = play_blocking(cue, volume, None).unwrap();
        println!(
            "{cue:?}: {} ms tone, played in {} ms",
            cue.duration().as_millis(),
            played.as_millis()
        );
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
}
