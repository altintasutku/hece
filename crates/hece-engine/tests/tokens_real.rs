//! Parity of the Rust token helpers against faster-whisper 1.2.1 on the real tokenizer.
//! Needs `HECE_TEST_MODEL_DIR` pointing at a Whisper large-v3 model directory.

#![expect(
    clippy::unwrap_used,
    reason = "clippy.toml only exempts #[test] functions, not helper functions in test files"
)]

use std::path::PathBuf;

use hece_engine::tokens::{
    SpecialTokens, TextTokenizer, language_token, sot_sequence, suppressed_tokens,
};
use serde_json::Value;

fn id(v: &Value) -> u32 {
    u32::try_from(v.as_u64().unwrap()).unwrap()
}

#[test]
#[ignore = "needs HECE_TEST_MODEL_DIR with a Whisper tokenizer.json"]
fn matches_faster_whisper() {
    let dir = PathBuf::from(std::env::var("HECE_TEST_MODEL_DIR").unwrap());
    let tok = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).unwrap();
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/tokens_expected.json")).unwrap();

    let sp = SpecialTokens::resolve(&tok).unwrap();
    assert_eq!(sp.sot, id(&expected["sot"]));
    assert_eq!(sp.sot_prev, id(&expected["sot_prev"]));
    assert_eq!(sp.sot_lm, id(&expected["sot_lm"]));
    assert_eq!(sp.eot, id(&expected["eot"]));
    assert_eq!(sp.transcribe, id(&expected["transcribe"]));
    assert_eq!(sp.translate, id(&expected["translate"]));
    assert_eq!(sp.no_timestamps, id(&expected["no_timestamps"]));
    assert_eq!(sp.no_speech, id(&expected["no_speech"]));
    assert_eq!(sp.timestamp_begin, id(&expected["timestamp_begin"]));

    let tr = language_token(&tok, "tr").unwrap();
    assert_eq!(tr, id(&expected["language_tr"]));
    let seq: Vec<u32> = expected["sot_sequence"]
        .as_array()
        .unwrap()
        .iter()
        .map(id)
        .collect();
    assert_eq!(sot_sequence(&sp, tr).to_vec(), seq);

    let suppressed: Vec<i32> = expected["suppressed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| i32::try_from(v.as_i64().unwrap()).unwrap())
        .collect();
    assert_eq!(suppressed_tokens(&tok, &sp).unwrap(), suppressed);

    let text = expected["text"].as_str().unwrap();
    let ids: Vec<u32> = expected["text_tokens"]
        .as_array()
        .unwrap()
        .iter()
        .map(id)
        .collect();
    assert_eq!(TextTokenizer::encode(&tok, text).unwrap(), ids);
}
