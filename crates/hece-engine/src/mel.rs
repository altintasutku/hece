//! Whisper log-mel features.
//!
//! Ported from faster-whisper 1.2.1 (MIT), `feature_extractor.py` (`FeatureExtractor`).

use std::f64::consts::PI;
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

/// FFT size in samples.
const N_FFT: usize = 400;
/// Hop between frames in samples.
const HOP: usize = 160;
/// Zeros appended to the waveform before the STFT (faster-whisper `padding=160`).
const PADDING: usize = 160;
/// Frames in one 30 s Whisper window.
pub const N_FRAMES: usize = 3000;
/// Bins in the one-sided spectrum.
const N_FREQS: usize = N_FFT / 2 + 1;
/// Sample rate used for the filter bank centre frequencies.
const SR: f64 = 16_000.0;

/// Log-mel spectrogram, row-major `[n_mels][frames]`.
#[derive(Debug, Clone)]
pub struct Mel {
    /// Number of mel bands (rows).
    pub n_mels: usize,
    /// Number of time frames (columns).
    pub frames: usize,
    /// Row-major values.
    pub data: Vec<f32>,
}

impl Mel {
    /// Returns `data[:, start..start + len]` as an `[n_mels][3000]` buffer, zero-padded
    /// (faster-whisper `pad_or_trim`). Columns past the end of the spectrogram are zero too.
    #[must_use]
    pub fn window(&self, start: usize, len: usize) -> Vec<f32> {
        let mut out = vec![0.0; self.n_mels * N_FRAMES];
        let start = start.min(self.frames);
        let take = len.min(N_FRAMES).min(self.frames - start);
        for m in 0..self.n_mels {
            let src = &self.data[m * self.frames + start..m * self.frames + start + take];
            out[m * N_FRAMES..m * N_FRAMES + take].copy_from_slice(src);
        }
        out
    }
}

/// Log-mel extractor with precomputed filter bank, window and FFT plan.
pub struct MelExtractor {
    n_mels: usize,
    /// Row-major `[n_mels][N_FREQS]`.
    filters: Vec<f32>,
    window: Vec<f32>,
    fft: Arc<dyn Fft<f32>>,
}

impl std::fmt::Debug for MelExtractor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MelExtractor")
            .field("n_mels", &self.n_mels)
            .finish_non_exhaustive()
    }
}

impl MelExtractor {
    /// Builds the extractor for `n_mels` bands.
    #[must_use]
    pub fn new(n_mels: usize) -> Self {
        // Periodic Hann: `np.hanning(n_fft + 1)[:-1]`.
        #[expect(
            clippy::cast_precision_loss,
            reason = "indices below 400 are exact in f64"
        )]
        #[expect(
            clippy::cast_possible_truncation,
            reason = "window values are stored as f32 like the reference"
        )]
        let window = (0..N_FFT)
            .map(|n| (0.5 - 0.5 * (2.0 * PI * n as f64 / N_FFT as f64).cos()) as f32)
            .collect();
        Self {
            n_mels,
            filters: mel_filters(n_mels),
            window,
            fft: FftPlanner::<f32>::new().plan_fft_forward(N_FFT),
        }
    }

    /// Computes the log-mel spectrogram of 16 kHz mono audio; yields `(N + 160) / 160` frames.
    #[must_use]
    pub fn log_mel(&self, audio: &[f32]) -> Mel {
        let mut x = Vec::with_capacity(audio.len() + PADDING);
        x.extend_from_slice(audio);
        x.resize(audio.len() + PADDING, 0.0);

        // Centre reflect padding by n_fft / 2 on both sides.
        let pad = N_FFT / 2;
        let mut padded = Vec::with_capacity(x.len() + 2 * pad);
        padded.extend((0..pad).map(|i| x[reflect(i.cast_signed() - pad.cast_signed(), x.len())]));
        padded.extend_from_slice(&x);
        padded.extend((0..pad).map(|i| x[reflect((x.len() + i).cast_signed(), x.len())]));

        // The reference drops the last STFT frame.
        let frames = (padded.len() - N_FFT) / HOP;

        // Power spectrum per frame, row-major `[frames][N_FREQS]`.
        let mut power = vec![0.0_f32; frames * N_FREQS];
        let mut buf = vec![Complex::new(0.0_f32, 0.0); N_FFT];
        for (t, row) in power.as_chunks_mut::<N_FREQS>().0.iter_mut().enumerate() {
            for (i, b) in buf.iter_mut().enumerate() {
                *b = Complex::new(padded[t * HOP + i] * self.window[i], 0.0);
            }
            self.fft.process(&mut buf);
            for (p, c) in row.iter_mut().zip(&buf) {
                *p = c.norm_sqr();
            }
        }

        let mut data = vec![0.0_f32; self.n_mels * frames];
        let mut max = f32::MIN;
        for (m, filt) in self.filters.as_chunks::<N_FREQS>().0.iter().enumerate() {
            for t in 0..frames {
                let spec = &power[t * N_FREQS..(t + 1) * N_FREQS];
                let v: f32 = filt.iter().zip(spec).map(|(a, b)| a * b).sum();
                let l = v.max(1e-10).log10();
                data[m * frames + t] = l;
                max = max.max(l);
            }
        }
        for v in &mut data {
            *v = (v.max(max - 8.0) + 4.0) / 4.0;
        }
        Mel {
            n_mels: self.n_mels,
            frames,
            data,
        }
    }
}

/// Index of `i` under numpy `mode="reflect"` padding for a signal of length `len`; stays
/// valid when the pad is longer than the signal (numpy reflects repeatedly).
#[expect(
    clippy::cast_possible_wrap,
    reason = "signal lengths are far below isize::MAX"
)]
#[expect(clippy::cast_sign_loss, reason = "rem_euclid result is non-negative")]
fn reflect(i: isize, len: usize) -> usize {
    if len <= 1 {
        return 0;
    }
    let period = 2 * (len as isize - 1);
    let k = i.rem_euclid(period);
    (if k < len as isize { k } else { period - k }) as usize
}

/// Slaney-scale mel filter bank, row-major `[n_mels][N_FREQS]`
/// (`FeatureExtractor.get_mel_filters`).
#[expect(
    clippy::cast_precision_loss,
    reason = "small integer indices are exact in f64"
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "filter weights are stored as f32 like the reference"
)]
fn mel_filters(n_mels: usize) -> Vec<f32> {
    let fftfreqs: Vec<f64> = (0..N_FREQS).map(|i| i as f64 * SR / N_FFT as f64).collect();
    let (min_mel, max_mel) = (0.0_f64, 45.245_640_471_924_965_f64);
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0_f64;
    let min_log_mel = min_log_hz / f_sp;
    let logstep = 6.4_f64.ln() / 27.0;
    let freqs: Vec<f64> = (0..n_mels + 2)
        .map(|i| min_mel + (max_mel - min_mel) * i as f64 / (n_mels + 1) as f64)
        .map(|m| {
            if m >= min_log_mel {
                min_log_hz * (logstep * (m - min_log_mel)).exp()
            } else {
                f_sp * m
            }
        })
        .collect();
    let mut out = Vec::with_capacity(n_mels * N_FREQS);
    for m in 0..n_mels {
        let enorm = 2.0 / (freqs[m + 2] - freqs[m]);
        let d_lo = freqs[m + 1] - freqs[m];
        let d_hi = freqs[m + 2] - freqs[m + 1];
        out.extend(fftfreqs.iter().map(|&f| {
            let lower = (f - freqs[m]) / d_lo;
            let upper = (freqs[m + 2] - f) / d_hi;
            (lower.min(upper).max(0.0) * enorm) as f32
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_zero_pads_to_3000_frames() {
        let mel = Mel {
            n_mels: 128,
            frames: 2,
            data: vec![1.0; 128 * 2],
        };
        let w = mel.window(0, 10);
        assert_eq!(w.len(), 128 * N_FRAMES);
        for m in 0..128 {
            let row = &w[m * N_FRAMES..(m + 1) * N_FRAMES];
            assert!(row[..2].iter().all(|&v| (v - 1.0).abs() < f32::EPSILON));
            assert!(row[2..].iter().all(|&v| v == 0.0));
        }
    }

    #[test]
    fn empty_audio_does_not_panic() {
        let mel = MelExtractor::new(128).log_mel(&[]);
        assert!(mel.frames >= 1);
        assert_eq!(mel.data.len(), 128 * mel.frames);
    }

    #[test]
    fn frame_count_follows_reference_formula() {
        let ex = MelExtractor::new(128);
        for n in [1, 159, 160, 161, 16_000] {
            assert_eq!(ex.log_mel(&vec![0.1; n]).frames, (n + 160) / 160);
        }
    }
}
