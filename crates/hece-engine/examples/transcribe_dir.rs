//! Transcribes a directory of 16 kHz mono 16-bit WAV files for benchmarking.
//!
//! ```text
//! transcribe_dir <model_dir> <audio_dir> <out.tsv> [--prompt-file <file>] [--no-vad] [--language tr]
//! ```
//!
//! `audio_dir/liste.tsv` lists `<file>\t<reference>` lines. Each file is transcribed after one
//! warm-up run and `<file>\t<seconds>\t<text>` is written to `out.tsv`. Only `<file> <seconds>`
//! goes to stderr; transcript text never does.

use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use hece_engine::{ComputeKind, LocalEngine, TranscribeOptions};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct Args {
    model_dir: PathBuf,
    audio_dir: PathBuf,
    out: PathBuf,
    opts: TranscribeOptions,
}

fn parse_args() -> Result<Args> {
    let mut positional = Vec::new();
    let mut opts = TranscribeOptions::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-vad" => opts.vad = false,
            "--prompt-file" => {
                let file = args.next().ok_or("--prompt-file needs a path")?;
                opts.initial_prompt = Some(std::fs::read_to_string(file)?.trim().to_owned());
            }
            "--language" => opts.language = Some(args.next().ok_or("--language needs a code")?),
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    let [model_dir, audio_dir, out]: [PathBuf; 3] = positional.try_into().map_err(|_| {
        "usage: transcribe_dir <model_dir> <audio_dir> <out.tsv> \
         [--prompt-file <file>] [--no-vad] [--language tr]"
    })?;
    Ok(Args {
        model_dir,
        audio_dir,
        out,
        opts,
    })
}

fn read_wav(path: &Path) -> Result<Vec<i16>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    if spec.sample_rate != 16_000 || spec.channels != 1 || spec.bits_per_sample != 16 {
        return Err(format!("{}: expected 16 kHz mono 16-bit PCM", path.display()).into());
    }
    Ok(reader
        .samples::<i16>()
        .collect::<std::result::Result<_, _>>()?)
}

#[expect(
    clippy::print_stderr,
    reason = "benchmark progress output; only file names and timings"
)]
fn progress(file: &str, seconds: f64) {
    eprintln!("{file} {seconds:.3}");
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let mut engine = LocalEngine::load(&args.model_dir, ComputeKind::default(), 0)?;
    engine.warm_up()?;

    let list = std::fs::read_to_string(args.audio_dir.join("liste.tsv"))?;
    let mut out = String::new();
    for line in list.lines().filter(|l| !l.trim().is_empty()) {
        let file = line.split('\t').next().unwrap_or(line);
        let audio = read_wav(&args.audio_dir.join(file))?;
        let started = Instant::now();
        let transcript = engine.transcribe(&audio, &args.opts)?;
        let seconds = started.elapsed().as_secs_f64();
        progress(file, seconds);
        // Keep the TSV one line per file.
        let text = transcript.text.replace(['\t', '\n', '\r'], " ");
        writeln!(out, "{file}\t{seconds:.3}\t{text}")?;
    }
    std::fs::write(&args.out, out)?;
    Ok(())
}
