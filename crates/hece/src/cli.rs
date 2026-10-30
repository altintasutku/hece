//! Command-line definition.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use hece_core::DEFAULT_PROMPT_TOKENS;

/// Hece voice dictation.
#[derive(Debug, Parser)]
#[command(name = "hece", version, about, arg_required_else_help = true)]
pub struct Cli {
    /// Subcommand to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Measure transcription quality on a directory of recordings with references.
    ///
    /// Exits with status 1 if any file failed (after printing the table); failed files are
    /// listed on stderr as `failed <file> <code>`.
    Bench(BenchArgs),
}

/// Arguments of `hece bench`.
#[derive(Debug, Args)]
pub struct BenchArgs {
    /// Directory holding `liste.tsv` (`<wav file><TAB><reference text>` per line) and the WAVs.
    pub dir: PathBuf,
    /// `CTranslate2` model directory.
    #[arg(long)]
    pub model: PathBuf,
    /// Path of the `hece-engine` binary (default: next to this binary, then `PATH`).
    #[arg(long)]
    pub engine: Option<PathBuf>,
    /// Vocabulary file (one term per line).
    #[arg(long)]
    pub vocab: Option<PathBuf>,
    /// Text placed before the vocabulary terms in the Whisper prompt.
    #[arg(long, default_value = hece_core::DEFAULT_PROMPT_PREFIX)]
    pub prefix: String,
    /// Token budget of the vocabulary prompt.
    #[arg(long, default_value_t = DEFAULT_PROMPT_TOKENS)]
    pub prompt_tokens: usize,
    /// Spoken language code.
    #[arg(long, default_value = "tr")]
    pub language: String,
    /// Treat `dir` as the long-recording set and report long-form checks.
    #[arg(long)]
    pub long: bool,
    /// Short-utterance set to run in the same invocation (for the WER delta of `--long`).
    #[arg(long)]
    pub short: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_bench_flags() {
        let cli = Cli::try_parse_from([
            "hece", "bench", "dir", "--model", "m", "--long", "--short", "s",
        ])
        .unwrap();
        let Command::Bench(args) = cli.command;
        assert!(args.long);
        assert_eq!(args.short.as_deref(), Some(std::path::Path::new("s")));
        assert_eq!(args.prompt_tokens, 200);
    }

    #[test]
    fn model_is_required() {
        assert!(Cli::try_parse_from(["hece", "bench", "dir"]).is_err());
    }
}
