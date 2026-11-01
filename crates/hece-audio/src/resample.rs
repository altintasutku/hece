//! Downmix to mono and resample to 16 kHz `i16`, the format every provider takes.

use rubato::audioadapter_buffers::direct::SequentialSliceOfSlices;
use rubato::{Fft, FixedSync, Resampler};

use crate::AudioError;

/// Output sample rate.
pub const TARGET_RATE: u32 = 16_000;
/// Frames per FFT chunk; the resampler's latency does not matter for a whole clip.
const CHUNK: usize = 1024;

/// Averages interleaved frames of `channels` channels into mono.
#[must_use]
pub fn downmix(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let channels = usize::from(channels.max(1));
    if channels == 1 {
        return interleaved.to_vec();
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "channel counts are tiny, far below f32's exact integer range"
    )]
    let scale = 1.0 / channels as f32;
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() * scale)
        .collect()
}

/// Resamples mono `samples` at `rate` Hz to [`TARGET_RATE`] and converts to `i16`.
pub fn to_16k(samples: &[f32], rate: u32) -> Result<Vec<i16>, AudioError> {
    if samples.is_empty() {
        return Ok(Vec::new());
    }
    if rate == TARGET_RATE {
        return Ok(samples.iter().copied().map(to_i16).collect());
    }
    let in_rate = usize::try_from(rate).map_err(|_| AudioError::Resample)?;
    let out_rate = usize::try_from(TARGET_RATE).map_err(|_| AudioError::Resample)?;
    let mut resampler = Fft::<f32>::new(in_rate, out_rate, CHUNK, 1, FixedSync::Input)
        .map_err(|_| AudioError::Resample)?;
    let input = [samples];
    let adapter =
        SequentialSliceOfSlices::new(&input, 1, samples.len()).map_err(|_| AudioError::Resample)?;
    let output = resampler
        .process_all(&adapter, samples.len(), None)
        .map_err(|_| AudioError::Resample)?;
    Ok(output.take_data().into_iter().map(to_i16).collect())
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "clamped to [-1, 1] and scaled into i16 range before the cast"
)]
fn to_i16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[expect(clippy::cast_precision_loss, reason = "test signal generation")]
    /// One second of a sine at half scale.
    fn sine(rate: u32, freq: f32) -> Vec<f32> {
        let n = usize::try_from(rate).unwrap();
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * 0.5)
            .collect()
    }

    fn zero_crossings(s: &[i16]) -> usize {
        s.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count()
    }

    #[test]
    fn downmix_averages_channels() {
        assert_eq!(downmix(&[1.0, 0.0, 0.5, 0.5], 2), [0.5, 0.5]);
        assert_eq!(downmix(&[0.25, -0.25], 1), [0.25, -0.25]);
    }

    #[test]
    fn same_rate_only_converts() {
        assert_eq!(
            to_16k(&[0.0, 1.0, -1.0, 2.0], 16_000).unwrap(),
            [0, 32767, -32767, 32767]
        );
    }

    #[test]
    fn resampling_keeps_length_and_pitch() {
        for rate in [44_100, 48_000] {
            let out = to_16k(&sine(rate, 440.0), rate).unwrap();
            assert!(
                out.len().abs_diff(16_000) <= 2,
                "{rate}: got {} samples",
                out.len()
            );
            // 440 Hz crosses zero 880 times per second.
            let zc = zero_crossings(&out[400..out.len() - 400]);
            let expected = 880 * (out.len() - 800) / 16_000;
            assert!(zc.abs_diff(expected) <= 4, "{rate}: {zc} crossings");
        }
    }
}
