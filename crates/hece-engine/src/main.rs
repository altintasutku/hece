//! Hece engine worker binary: `hece-engine serve` runs the unix socket worker.
//!
//! Exit codes: 0 normal, 2 built without CUDA, 3 another worker is running, 4 socket or
//! runtime-dir error, 64 bad command line.

use std::process::ExitCode;

#[cfg(not(feature = "cuda"))]
fn main() -> ExitCode {
    let err = hece_engine::error::EngineError::BuiltWithoutCuda;
    output::error(&format!("hece-engine: {err} ({})", err.code()));
    ExitCode::from(2)
}

#[cfg(feature = "cuda")]
fn main() -> ExitCode {
    use hece_engine::server::{ArgsError, CudaBackend, ServeArgs, serve};
    use hece_engine::socket::{BindError, bind, socket_path};

    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = args.split_first() else {
        output::error("hece-engine: usage: hece-engine serve (bad_args)");
        return ExitCode::from(64);
    };
    if command != "serve" {
        output::error("hece-engine: unknown command (bad_args)");
        return ExitCode::from(64);
    }
    let serve_args = match ServeArgs::parse(rest) {
        Ok(a) => a,
        Err(e @ ArgsError) => {
            output::error(&format!("hece-engine: {e} (bad_args)"));
            return ExitCode::from(64);
        }
    };

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let path = match serve_args.socket {
        Some(p) => p,
        None => match socket_path() {
            Ok(p) => p,
            Err(e) => {
                output::error(&format!("hece-engine: {e} ({})", e.code()));
                return ExitCode::from(4);
            }
        },
    };
    let bound = match bind(&path) {
        Ok(b) => b,
        Err(BindError::AlreadyRunning) => {
            output::error("hece-engine: another worker is already running (already_running)");
            return ExitCode::from(3);
        }
        Err(e @ BindError::Io(_)) => {
            tracing::error!(detail = %e, "engine.error");
            output::error("hece-engine: cannot bind the worker socket (bind_failed)");
            return ExitCode::from(4);
        }
    };
    tracing::info!(
        socket = %path.display(),
        idle_ttl_secs = serve_args.idle_ttl.as_secs(),
        "engine.start"
    );
    let mut backend = CudaBackend::new();
    let result = serve(bound.listener, &mut backend, serve_args.idle_ttl);
    // Leave no stale file behind on a normal exit.
    let _ = std::fs::remove_file(&path);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            output::error(&format!("hece-engine: {e} ({})", e.code()));
            ExitCode::from(4)
        }
    }
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
