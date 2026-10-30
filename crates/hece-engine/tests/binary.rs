//! The worker binary is a placeholder until the IPC server lands: it explains why it cannot run
//! and exits with code 2.

use std::process::Command;

#[test]
fn exits_with_code_2_and_one_line_message() {
    let out = Command::new(env!("CARGO_BIN_EXE_hece-engine"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(stderr.trim_end().lines().count(), 1);
    #[cfg(not(feature = "cuda"))]
    assert!(stderr.contains("built_without_cuda"));
}
