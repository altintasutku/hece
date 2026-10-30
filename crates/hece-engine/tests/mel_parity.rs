//! Parity of the Rust log-mel against faster-whisper 1.2.1.

use hece_engine::audio::to_f32;
use hece_engine::mel::MelExtractor;

fn read_i16(bytes: &[u8]) -> Vec<i16> {
    bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect()
}

fn read_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

#[test]
fn log_mel_matches_faster_whisper() {
    let audio = read_i16(include_bytes!("fixtures/mel_audio.i16"));
    let expected = read_f32(include_bytes!("fixtures/mel_expected.f32"));

    let mel = MelExtractor::new(128).log_mel(&to_f32(&audio));

    assert_eq!(mel.n_mels, 128);
    assert_eq!(mel.frames, expected.len() / 128);
    assert_eq!(mel.frames, (audio.len() + 160) / 160);
    let max_diff = mel
        .data
        .iter()
        .zip(&expected)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    assert!(max_diff < 1e-4, "max abs diff {max_diff}");
}
