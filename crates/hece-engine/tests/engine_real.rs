//! `LocalEngine` on a real model with synthetic audio. The model tests need a CUDA GPU and
//! `HECE_TEST_MODEL_DIR` pointing at a converted Whisper large-v3 model directory.

#![expect(
    clippy::unwrap_used,
    reason = "clippy.toml only exempts #[test] functions, not helper functions in test files"
)]

use std::path::PathBuf;

use hece_engine::{ComputeKind, LocalEngine, TranscribeOptions};

fn engine() -> LocalEngine {
    let dir = PathBuf::from(std::env::var("HECE_TEST_MODEL_DIR").unwrap());
    LocalEngine::load(&dir, ComputeKind::default(), 0).unwrap()
}

#[test]
fn directory_without_model_files_is_invalid() {
    let dir = std::env::temp_dir().join(format!("hece-empty-model-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let err = LocalEngine::load(&dir, ComputeKind::default(), 0).unwrap_err();
    std::fs::remove_dir(&dir).unwrap();
    assert_eq!(err.code(), "model_dir_invalid");
}

#[test]
#[ignore = "needs a CUDA GPU and HECE_TEST_MODEL_DIR"]
fn silence_gives_empty_text() {
    let mut engine = engine();
    engine.warm_up().unwrap();
    let opts = TranscribeOptions {
        language: Some("tr".to_owned()),
        ..TranscribeOptions::default()
    };
    let t = engine.transcribe(&vec![0; 32_000], &opts).unwrap();
    assert_eq!(t.text, "");
    assert_eq!(t.segments, Some(Vec::new()));
    assert_eq!(t.provider, "local-ct2");
    assert_eq!(t.language, "tr");
}

#[test]
#[ignore = "needs a CUDA GPU and HECE_TEST_MODEL_DIR"]
fn synthetic_audio_decodes_without_vad_and_detects_a_language() {
    let audio: Vec<i16> = include_bytes!("fixtures/mel_audio.i16")
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    assert_eq!(audio.len(), 24_000);
    let mut engine = engine();
    let opts = TranscribeOptions {
        vad: false,
        ..TranscribeOptions::default()
    };
    let t = engine.transcribe(&audio, &opts).unwrap();
    assert!(!t.language.is_empty() && !t.language.contains('|'));
    let segments = t.segments.unwrap();
    assert!(segments.iter().all(|s| s.start_s <= s.end_s));
    // With VAD on, the same audio must not fail either.
    engine
        .transcribe(&audio, &TranscribeOptions::default())
        .unwrap();
}
