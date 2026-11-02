//! Short sound cues for recording start, stop and cancel/error. The tones are synthesized here
//! (no audio assets) at the output device's own rate, so nothing is resampled, and played on a
//! throwaway thread through cpal, so neither the hotkey nor the capture path ever waits for them.
//!
//! The microphone hears the speaker: the start cue plays while capture runs, so
//! [`play`] mutes the capture (see [`Mute`]) for the cue plus an echo margin. The cue is the
//! "speak now" signal, so the muted span is one the user does not speak into.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use crate::AudioError;
use crate::capture::Mute;

/// Peak amplitude at volume 1.0 (-6 dBFS): cues stay well below anything clipping.
const MAX_PEAK: f32 = 0.5;
/// Fade in at each note start (raised cosine), long enough to avoid a click.
const ATTACK: f32 = 0.004;
/// Fade out at each note end.
const RELEASE: f32 = 0.012;
/// Weight of the second harmonic: a little warmth, still soft.
const OVERTONE: f32 = 0.15;
/// After the cue: output latency plus room echo reaching the microphone.
const ECHO_MARGIN: Duration = Duration::from_millis(80);
/// Time for the device buffer to play out before the stream is closed.
const DRAIN: Duration = Duration::from_millis(120);

/// Which cue to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    /// Recording started (the microphone delivers audio): two notes going up.
    Start,
    /// Recording stopped: the same two notes going down.
    Stop,
    /// Cancelled or failed: one low note.
    Cancel,
}

/// E5, A5 and E4 (Hz).
const E5: f32 = 659.25;
const A5: f32 = 880.0;
const E4: f32 = 329.63;

impl Cue {
    /// The notes as (frequency in Hz, length in ms).
    fn notes(self) -> &'static [(f32, u32)] {
        match self {
            Self::Start => &[(E5, 35), (A5, 50)],
            Self::Stop => &[(A5, 35), (E5, 50)],
            Self::Cancel => &[(E4, 110)],
        }
    }

    /// How long the cue sounds.
    #[must_use]
    pub fn duration(self) -> Duration {
        Duration::from_millis(self.notes().iter().map(|(_, ms)| u64::from(*ms)).sum())
    }
}

/// The cue as mono samples at `rate` Hz; `volume` (0..=1) scales the peak.
#[must_use]
pub fn tone(cue: Cue, rate: u32, volume: f32) -> Vec<f32> {
    let peak = MAX_PEAK * volume.clamp(0.0, 1.0) / (1.0 + OVERTONE);
    let rate_f = f64::from(rate);
    let mut out = Vec::new();
    for &(freq, ms) in cue.notes() {
        let n = rate.saturating_mul(ms) / 1000;
        let len = f64::from(n) / rate_f;
        for i in 0..n {
            let t = f64::from(i) / rate_f;
            #[allow(
                clippy::cast_possible_truncation,
                reason = "a time within one short note fits f32"
            )]
            let (t, len) = (t as f32, len as f32);
            let phase = std::f32::consts::TAU * freq * t;
            let wave = phase.sin() + OVERTONE * (2.0 * phase).sin();
            // A gentle decay so the note rings rather than buzzes.
            let decay = (-3.0 * t / len).exp();
            out.push(peak * wave * decay * fade(t, ATTACK) * fade(len - t, RELEASE));
        }
    }
    out
}

/// Raised-cosine ramp from 0 (at `x` = 0) to 1 (at `x` >= `width`).
fn fade(x: f32, width: f32) -> f32 {
    if x >= width {
        1.0
    } else {
        0.5 - 0.5 * (std::f32::consts::PI * x.max(0.0) / width).cos()
    }
}

/// Plays `cue` on a background thread and returns at once; failures (no speaker, busy device)
/// are logged and otherwise ignored. With `mute`, the capture is muted while the cue sounds.
pub fn play(cue: Cue, volume: f32, mute: Option<Mute>) {
    let spawned = std::thread::Builder::new()
        .name("hece-cue".to_owned())
        .spawn(move || {
            if let Err(e) = play_blocking(cue, volume, mute.as_ref()) {
                tracing::warn!(code = e.code(), "sound cue failed");
            }
        });
    if spawned.is_err() {
        tracing::warn!(code = AudioError::Thread.code(), "sound cue failed");
    }
}

/// Plays `cue` on the default output device and returns once it has been played, with the
/// time from start of playback until the last sample was handed to the device.
pub fn play_blocking(cue: Cue, volume: f32, mute: Option<&Mute>) -> Result<Duration, AudioError> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or(AudioError::NoOutputDevice)?;
    let supported = device
        .default_output_config()
        .map_err(|e| AudioError::from_cpal(&e))?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let samples = tone(cue, config.sample_rate, volume);
    let (done_tx, done) = mpsc::sync_channel(1);
    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, &config, samples, done_tx),
        SampleFormat::I16 => build::<i16>(&device, &config, samples, done_tx),
        SampleFormat::U16 => build::<u16>(&device, &config, samples, done_tx),
        SampleFormat::I32 => build::<i32>(&device, &config, samples, done_tx),
        _ => Err(AudioError::UnsupportedFormat),
    }?;
    if let Some(mute) = mute {
        mute.until(Instant::now() + cue.duration() + ECHO_MARGIN);
    }
    let started = Instant::now();
    stream.play().map_err(|e| AudioError::from_cpal(&e))?;
    done.recv_timeout(cue.duration() + Duration::from_secs(2))
        .map_err(|_| AudioError::Backend)?;
    let played = started.elapsed();
    std::thread::sleep(DRAIN);
    Ok(played)
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    samples: Vec<f32>,
    done: mpsc::SyncSender<()>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels.max(1));
    let mut pos = 0;
    device
        .build_output_stream::<T, _, _>(
            *config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                for frame in data.chunks_mut(channels) {
                    let s = T::from_sample(samples.get(pos).copied().unwrap_or(0.0));
                    frame.fill(s);
                    pos += 1;
                }
                if pos >= samples.len() {
                    // Full after the first send; later callbacks just play silence.
                    let _ = done.try_send(());
                }
            },
            |e: cpal::Error| tracing::debug!(kind = ?e.kind(), "sound cue stream event"),
            None,
        )
        .map_err(|e| AudioError::from_cpal(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tones_have_the_cue_length_and_stay_soft() {
        for cue in [Cue::Start, Cue::Stop, Cue::Cancel] {
            let samples = tone(cue, 48_000, 1.0);
            let expected = usize::try_from(cue.duration().as_millis() * 48).unwrap();
            assert_eq!(samples.len(), expected, "{cue:?}");
            let peak = samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.1 && peak <= MAX_PEAK, "{cue:?} peak {peak}");
            assert!(samples.iter().all(|s| s.is_finite()));
        }
        assert!(Cue::Start.duration() <= Duration::from_millis(120));
    }

    #[test]
    fn notes_fade_in_and_out_without_clicks() {
        let rate = 48_000;
        let samples = tone(Cue::Start, rate, 1.0);
        // Both note boundaries: the first sample of each note and the last of each.
        let first_len = usize::try_from(rate * 35 / 1000).unwrap();
        for i in [0, first_len - 1, first_len, samples.len() - 1] {
            assert!(samples[i].abs() < 1e-3, "sample {i} = {}", samples[i]);
        }
        // No jump beyond the steepest slope of the waves themselves (~0.065 here; a click
        // would be close to the 0.43 peak).
        let step = samples
            .windows(2)
            .fold(0.0_f32, |m, w| m.max((w[1] - w[0]).abs()));
        assert!(step < 0.07, "max step {step}");
    }

    #[test]
    fn volume_scales_and_zero_is_silent() {
        let loud = tone(Cue::Cancel, 44_100, 1.0);
        let soft = tone(Cue::Cancel, 44_100, 0.25);
        assert!((soft[200] * 4.0 - loud[200]).abs() < 1e-6);
        assert!(tone(Cue::Cancel, 44_100, 0.0).iter().all(|s| *s == 0.0));
    }
}
