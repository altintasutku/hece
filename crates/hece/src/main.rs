//! Hece daemon and CLI entry point.

mod bench;
mod cli;
mod output;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command};

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(any_failed) => ExitCode::from(exit_status(any_failed)),
        Err(e) => {
            output::error(&format!("{e:#}"));
            ExitCode::from(EXIT_SETUP)
        }
    }
}

/// Some files failed (the report was still printed).
const EXIT_FILES_FAILED: u8 = 1;
/// Setup or engine error outside per-file work. Clap usage errors exit 2 on their own.
const EXIT_SETUP: u8 = 3;

fn exit_status(any_failed: bool) -> u8 {
    if any_failed { EXIT_FILES_FAILED } else { 0 }
}

/// Returns whether any file failed.
fn run(cli: Cli) -> anyhow::Result<bool> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    match cli.command {
        Command::Bench(args) => runtime.block_on(bench::execute(&args)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_distinct() {
        assert_eq!(exit_status(false), 0);
        assert_eq!(exit_status(true), 1);
        assert_eq!(EXIT_SETUP, 3);
    }
}
