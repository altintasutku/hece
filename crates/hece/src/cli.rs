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
    /// Run the dictation daemon (default: Ctrl+Shift+Space records, Ctrl+Shift+Backspace cancels).
    ///
    /// Settings come from the config file; a commented template is written on first run.
    /// `hecew.exe` (no arguments) runs the same daemon without a console window.
    #[cfg(windows)]
    Run(RunArgs),
    /// Show what the running daemon is doing (idle, recording, processing N).
    #[cfg(windows)]
    Status,
    /// Start or stop a recording in the running daemon.
    #[cfg(windows)]
    Toggle,
    /// Show recent dictations.
    History(HistoryArgs),
    /// Copy the last dictation to the clipboard (kept out of clipboard history).
    #[cfg(windows)]
    Repaste {
        /// The raw transcript instead of the cleaned text.
        #[arg(long)]
        raw: bool,
    },
    /// Manage API keys in the OS keyring.
    #[command(subcommand)]
    Key(KeyCommand),
    /// List or download local models.
    #[command(subcommand)]
    Model(ModelCommand),
    /// Start hece at login.
    #[cfg(windows)]
    Autostart {
        /// on, off or status.
        #[arg(value_enum)]
        action: Toggle,
    },
    /// Print the config file path (creating the template if missing).
    Config,
}

/// Arguments of `hece bench`.
#[derive(Debug, Args)]
pub struct BenchArgs {
    /// Directory holding `liste.tsv` (`<wav file><TAB><reference text>` per line) and the WAVs.
    pub dir: PathBuf,
    /// `CTranslate2` model directory.
    #[arg(long)]
    pub model: PathBuf,
    /// Path of the `hece-engine` binary (default: next to this binary).
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
    /// With `--long`: split each file with the live-dictation chunker (pauses after 15 s, cap
    /// 30 s) and join the chunk transcripts, to compare against whole-file transcription.
    #[arg(long, requires = "long")]
    pub chunked: bool,
    /// Beam width (1 = greedy; default: the engine's 5).
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=10))]
    pub beam_size: Option<u32>,
}

/// Which provider transcribes, shared by `dictate` and `run`. Without `--api-base` the local
/// engine is used.
#[derive(Debug, Args)]
pub struct ProviderArgs {
    /// Spoken language code (default: auto-detect).
    #[arg(long)]
    pub language: Option<String>,
    /// Vocabulary file (one term per line).
    #[arg(long)]
    pub vocab: Option<PathBuf>,
    /// `CTranslate2` model directory for the local engine.
    #[arg(long, conflicts_with = "api_base")]
    pub model: Option<PathBuf>,
    /// Path of the `hece-engine` binary (default: next to this binary).
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

/// Arguments of `hece dictate`.
#[derive(Debug, Args)]
pub struct DictateArgs {
    /// Microphone name from `hece devices` (default: the system default).
    #[arg(long)]
    pub device: Option<String>,
    /// Recording limit in seconds.
    #[arg(long, default_value_t = 600)]
    pub max_secs: u64,
    /// Transcription provider.
    #[command(flatten)]
    pub provider: ProviderArgs,
}

/// Arguments of `hece run`.
#[derive(Debug, Args)]
pub struct RunArgs {
    /// Config file (default: `%APPDATA%\hece\config.toml`).
    #[arg(long)]
    pub config: Option<PathBuf>,
}

/// Arguments of `hece history`.
#[derive(Debug, Args)]
pub struct HistoryArgs {
    /// How many recent entries to show.
    #[arg(long, short = 'n', default_value_t = 10)]
    pub count: usize,
    /// Delete the history.
    #[arg(long, conflicts_with = "count")]
    pub clear: bool,
}

/// What to do with a provider's API key.
#[derive(Debug, Subcommand)]
pub enum KeyCommand {
    /// Store a key in the OS keyring (typed without echo).
    Set {
        /// Provider name (`name` in the config, or its kind).
        provider: String,
    },
    /// Remove a key from the OS keyring.
    Delete {
        /// Provider name.
        provider: String,
    },
}

/// Local models.
#[derive(Debug, Subcommand)]
pub enum ModelCommand {
    /// Show the models Hece can download and which are installed.
    List,
    /// Download a model (resumes an interrupted download).
    Download {
        /// Model id from `hece model list`; default: the first one.
        id: Option<String>,
    },
}

/// Autostart at login.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Toggle {
    /// Start hecew.exe at login.
    On,
    /// Do not start at login.
    Off,
    /// Show whether it is on.
    Status,
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
