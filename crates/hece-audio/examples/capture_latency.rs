//! Measures how long a microphone takes to deliver its first samples and whether the recording
//! covers the whole wall-clock span (no lost start, post-roll included).
//!
//! `cargo run -p hece-audio --example capture_latency -- [runs] [device]`
#![allow(clippy::print_stdout, reason = "example output")]
#![allow(clippy::unwrap_used, reason = "example")]

use std::time::{Duration, Instant};

use hece_audio::{Capture, CaptureConfig};

fn main() {
    let mut args = std::env::args().skip(1);
    let runs: u32 = args.next().map_or(5, |r| r.parse().unwrap());
    let cfg = CaptureConfig {
        device: args.next(),
        ..CaptureConfig::default()
    };
    let hold = Duration::from_secs(2);
    for run in 1..=runs {
        let t0 = Instant::now();
        let capture = Capture::start(&cfg, |_| {}).unwrap();
        let opened = t0.elapsed();
        std::thread::sleep(hold);
        let rec = capture.stop().unwrap();
        let recorded =
            Duration::from_millis(u64::try_from(rec.samples.len()).unwrap() * 1000 / 16_000);
        println!(
            "run {run}: open {} ms, first audio {} ms, recorded {} ms of {} ms held + {} ms post-roll",
            opened.as_millis(),
            rec.first_audio_latency
                .map_or_else(|| "none".to_owned(), |d| d.as_millis().to_string()),
            recorded.as_millis(),
            hold.as_millis(),
            cfg.post_roll.as_millis(),
        );
    }
}
