//! Parity of the Rust VAD against faster-whisper 1.2.1, plus edge cases.

#![expect(
    clippy::unwrap_used,
    reason = "clippy.toml only exempts #[test] functions, not helper functions in test files"
)]

use hece_engine::audio::to_f32;
use hece_engine::vad::{
    SileroVad, SpeechSpan, SpeechTimestampsMap, VadOptions, collect_chunks, speech_timestamps,
};
use serde_json::Value;

fn as_usize(v: &Value) -> usize {
    usize::try_from(v.as_u64().unwrap()).unwrap()
}

fn as_f32(v: &Value) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "fixture values are f32 decimals"
    )]
    let x = v.as_f64().unwrap() as f32;
    x
}

fn options(v: &Value) -> VadOptions {
    VadOptions {
        threshold: as_f32(&v["threshold"]),
        neg_threshold: v["neg_threshold"]
            .as_f64()
            .map(|_| as_f32(&v["neg_threshold"])),
        min_speech_duration_ms: u32::try_from(v["min_speech_duration_ms"].as_u64().unwrap())
            .unwrap(),
        max_speech_duration_s: v["max_speech_duration_s"]
            .as_f64()
            .map_or(f32::INFINITY, |_| as_f32(&v["max_speech_duration_s"])),
        min_silence_duration_ms: u32::try_from(v["min_silence_duration_ms"].as_u64().unwrap())
            .unwrap(),
        speech_pad_ms: u32::try_from(v["speech_pad_ms"].as_u64().unwrap()).unwrap(),
    }
}

#[test]
fn speech_timestamps_match_faster_whisper() {
    let cases: Value = serde_json::from_str(include_str!("fixtures/vad_timestamps.json")).unwrap();
    let cases = cases.as_array().unwrap();
    assert!(cases.len() >= 8);
    for case in cases {
        let probs: Vec<f32> = case["probs"]
            .as_array()
            .unwrap()
            .iter()
            .map(as_f32)
            .collect();
        let expected: Vec<SpeechSpan> = case["expected"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| SpeechSpan {
                start: as_usize(&p[0]),
                end: as_usize(&p[1]),
            })
            .collect();
        let got = speech_timestamps(
            &probs,
            as_usize(&case["audio_len"]),
            &options(&case["options"]),
        );
        assert_eq!(got, expected, "case {}", case["name"]);
    }
}

#[test]
fn silero_probs_match_faster_whisper() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/vad_probs.json")).unwrap();
    let audio: Vec<i16> = fixture["audio"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| i16::try_from(v.as_i64().unwrap()).unwrap())
        .collect();
    let expected: Vec<f32> = fixture["probs"]
        .as_array()
        .unwrap()
        .iter()
        .map(as_f32)
        .collect();

    let probs = SileroVad::new()
        .unwrap()
        .speech_probs(&to_f32(&audio))
        .unwrap();

    assert_eq!(probs.len(), expected.len());
    let max_diff = probs
        .iter()
        .zip(&expected)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    assert!(max_diff < 1e-4, "max abs diff {max_diff}");
}

#[test]
fn all_zero_probs_give_no_spans() {
    let spans = speech_timestamps(&[0.0; 50], 25_000, &VadOptions::default());
    assert_eq!(spans, [] as [hece_engine::vad::SpeechSpan; 0]);
}

#[test]
fn short_inputs_give_one_prob_without_panic() {
    let mut vad = SileroVad::new().unwrap();
    assert_eq!(vad.speech_probs(&vec![0.01; 100]).unwrap().len(), 1);
    assert_eq!(vad.speech_probs(&vec![0.01; 512]).unwrap().len(), 1);
    assert_eq!(vad.speech_probs(&[]).unwrap(), [] as [f32; 0]);
}

#[test]
fn speech_spans_on_silence_is_empty() {
    let mut vad = SileroVad::new().unwrap();
    assert_eq!(
        vad.speech_spans(&vec![0.0; 16_000], &VadOptions::default())
            .unwrap(),
        [] as [hece_engine::vad::SpeechSpan; 0]
    );
    assert_eq!(
        vad.speech_spans(&[], &VadOptions::default()).unwrap(),
        [] as [hece_engine::vad::SpeechSpan; 0]
    );
}

#[test]
fn collect_chunks_concatenates_spans() {
    assert_eq!(collect_chunks(&[1.0, 2.0], &[]), [] as [f32; 0]);
    let audio: Vec<f32> = (0..10u8).map(f32::from).collect();
    let spans = [
        SpeechSpan { start: 1, end: 3 },
        SpeechSpan { start: 6, end: 9 },
    ];
    assert_eq!(
        collect_chunks(&audio, &spans),
        vec![1.0, 2.0, 6.0, 7.0, 8.0]
    );
}

#[test]
fn timestamps_map_restores_original_time() {
    // Spans [16000, 32000) and [64000, 80000): 1 s of silence is cut before the first, 2 s more
    // before the second. In the collected audio the chunks end at 16000 and 32000 samples.
    let map = SpeechTimestampsMap::new(&[
        SpeechSpan {
            start: 16_000,
            end: 32_000,
        },
        SpeechSpan {
            start: 64_000,
            end: 80_000,
        },
    ]);
    assert!((map.original_time(0.5, false) - 1.5).abs() < 1e-9);
    // 1.5 s lies in the second chunk: 1.5 + 3 s of removed silence.
    assert!((map.original_time(1.5, false) - 4.5).abs() < 1e-9);
    // An end exactly on the first chunk boundary belongs to the first chunk.
    assert!((map.original_time(1.0, true) - 2.0).abs() < 1e-9);
    // The same time as a start belongs to the second chunk.
    assert!((map.original_time(1.0, false) - 4.0).abs() < 1e-9);
}

#[test]
fn timestamps_map_boundary_times_use_f64_sample_math() {
    // Spans [0, 320) and [16000, 16320): the collected chunks end at samples 320 and 640.
    // 0.02 s is exactly 320 samples in f64 math (values checked against faster-whisper).
    let map = SpeechTimestampsMap::new(&[
        SpeechSpan { start: 0, end: 320 },
        SpeechSpan {
            start: 16_000,
            end: 16_320,
        },
    ]);
    // End exactly on the first chunk boundary: stays in chunk 0 (no gap added).
    assert!((map.original_time(0.02, true) - 0.02).abs() < 1e-9);
    // The same time as a start belongs to chunk 1: + (16000 - 320) / 16000 = 0.98 s of silence.
    assert!((map.original_time(0.02, false) - 1.0).abs() < 1e-9);
}
