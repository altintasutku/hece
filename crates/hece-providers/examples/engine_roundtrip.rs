//! Dev tool: prewarm the engine worker, transcribe synthetic audio and print timings.
//!
//! `cargo run -p hece-providers --example engine_roundtrip -- <model_dir> <engine_bin> [idle_ttl_secs]`
//!
//! Only timings, sizes and error codes are printed, never transcript text.

#![expect(clippy::print_stdout, reason = "dev tool reports timings on stdout")]

use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hece_providers::engine_client::{EngineClient, EngineConfig, ProcessSpawner};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: engine_roundtrip <model_dir> <engine_bin> [idle_ttl_secs]";
    let model_dir = PathBuf::from(args.next().ok_or(usage)?);
    let engine = PathBuf::from(args.next().ok_or(usage)?);
    let ttl: u64 = args.next().map_or(Ok(1800), |s| s.parse())?;

    let mut cfg = EngineConfig::default_for(model_dir)?;
    cfg.engine_path = Some(engine);
    cfg.idle_ttl = Duration::from_secs(ttl);
    let client = EngineClient::new(cfg, Arc::new(ProcessSpawner));

    let t = Instant::now();
    client.prewarm().await?;
    println!("prewarm_ms={}", t.elapsed().as_millis());

    // 3 s of a synthetic tone burst and 1 s of silence.
    let tone: Vec<i16> = (0..48_000)
        .map(|i| {
            let x = f64::from(i) / 16_000.0;
            #[expect(clippy::cast_possible_truncation, reason = "bounded by amplitude")]
            let v = (8_000.0 * (2.0 * std::f64::consts::PI * 220.0 * x).sin()) as i16;
            v
        })
        .collect();
    let silence = vec![0_i16; 16_000];
    for (name, audio) in [("tone", &tone), ("silence", &silence)] {
        let t = Instant::now();
        match client.transcribe(audio, Some("tr"), None, "", 200).await {
            Ok(tr) => println!(
                "{name}: ms={} chars={} segments={}",
                t.elapsed().as_millis(),
                tr.text.chars().count(),
                tr.segments.map_or(0, |s| s.len())
            ),
            Err(e) => println!("{name}: error={}", e.code()),
        }
    }
    Ok(())
}
