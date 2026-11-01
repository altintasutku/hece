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
    /// Exit status: 0 ok; 1 some files failed (the table is still printed, and failed files
    /// are listed on stderr as `failed <file> <code>`); 2 usage error; 3 setup or engine
    /// error (bad list or audio, unreadable vocabulary, engine could not start or died).
    ///
    /// With `--long`, the dropped-negation count covers word-level negations only (değil,
    /// yok, hayır, asla, hiç…, not, never, n't…); Turkish verb negation (-me/-ma) is not
    /// detected.
    Bench(BenchArgs),
    /// List microphones (names usable with `--device`).
    Devices,
    /// Record once from the microphone (Enter stops), transcribe and print the text.
    Dictate(DictateArgs),
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
    #[arg(long, requires = "long")]
    pub short: Option<PathBuf>,
}

/// Arguments of `hece dictate`. Without `--api-base` the local engine is used.
#[derive(Debug, Args)]
pub struct DictateArgs {
    /// Microphone name from `hece devices` (default: the system default).
    #[arg(long)]
    pub device: Option<String>,
    /// Spoken language code (default: auto-detect).
    #[arg(long)]
    pub language: Option<String>,
    /// Vocabulary file (one term per line).
    #[arg(long)]
    pub vocab: Option<PathBuf>,
    /// Recording limit in seconds.
    #[arg(long, default_value_t = 600)]
    pub max_secs: u64,
    /// `CTranslate2` model directory for the local engine.
    #[arg(long, conflicts_with = "api_base")]
    pub model: Option<PathBuf>,
    /// Path of the `hece-engine` binary (default: next to this binary, then `PATH`).
    #[arg(long)]
    pub engine: Option<PathBuf>,
    /// Base URL of an OpenAI-compatible API, e.g. `https://api.openai.com/v1`.
    #[arg(long)]
    pub api_base: Option<String>,
    /// Model name for `--api-base`.
    #[arg(long, default_value = "whisper-1", requires = "api_base")]
    pub api_model: String,
    /// Env var holding the API key (none: no authentication, e.g. a local server).
    #[arg(long, requires = "api_base")]
    pub key_env: Option<String>,
    /// Ask the API for segments (`verbose_json`; Whisper models only).
    #[arg(long, requires = "api_base")]
    pub segments: bool,
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
        let Command::Bench(args) = cli.command else {
            panic!("expected bench");
        };
        assert!(args.long);
        assert_eq!(args.short.as_deref(), Some(std::path::Path::new("s")));
        assert_eq!(args.prompt_tokens, 200);
    }

    #[test]
    fn short_requires_long() {
        assert!(
            Cli::try_parse_from(["hece", "bench", "dir", "--model", "m", "--short", "s"]).is_err()
        );
    }

    #[test]
    fn model_is_required() {
        assert!(Cli::try_parse_from(["hece", "bench", "dir"]).is_err());
    }
}
