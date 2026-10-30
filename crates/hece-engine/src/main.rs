//! Hece engine worker binary. The worker IPC server is not implemented yet, so the binary only
//! explains why it cannot run and exits with code 2.

use std::process::ExitCode;

#[cfg(not(feature = "cuda"))]
fn main() -> ExitCode {
    let err = hece_engine::error::EngineError::BuiltWithoutCuda;
    output::error(&format!("hece-engine: {err} ({})", err.code()));
    ExitCode::from(2)
}

#[cfg(feature = "cuda")]
fn main() -> ExitCode {
    output::error("hece-engine: the worker server is not implemented yet (not_implemented)");
    ExitCode::from(2)
}

/// The only place this binary writes user-facing text.
mod output {
    /// Writes one error line to stderr.
    #[expect(
        clippy::print_stderr,
        reason = "user-facing CLI message; this module is the binary's only output path"
    )]
    pub fn error(message: &str) {
        eprintln!("{message}");
    }
}
