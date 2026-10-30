//! End-to-end checks of the `hece bench` binary that need no engine.

use std::process::Command;

fn hece() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hece"))
}

#[test]
fn bench_on_missing_dir_fails_before_starting_the_engine() {
    let out = hece()
        .args(["bench", "/nonexistent-hece-bench-dir", "--model", "m"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(out.stdout, Vec::<u8>::new());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("liste.tsv"), "{err}");
}

#[test]
fn no_subcommand_prints_help() {
    let out = hece().output().unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("bench"), "{err}");
}
