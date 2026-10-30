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
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            output::error(&format!("{e:#}"));
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    match cli.command {
        Command::Bench(args) => runtime.block_on(bench::execute(&args)),
    }
}
