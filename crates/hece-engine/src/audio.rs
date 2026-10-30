//! Audio constants and sample conversion.

/// Sample rate the whole pipeline works at.
pub const SAMPLE_RATE: usize = 16_000;

/// Converts 16-bit PCM to `f32` in `[-1, 1)`.
#[must_use]
pub fn to_f32(audio: &[i16]) -> Vec<f32> {
    audio.iter().map(|&x| f32::from(x) / 32768.0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_by_32768() {
        assert_eq!(to_f32(&[0, 16384, -32768]), vec![0.0, 0.5, -1.0]);
    }
}
