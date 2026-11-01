//! `hecew`: the daemon without a console window (for autostart).
#![windows_subsystem = "windows"]

fn main() -> std::process::ExitCode {
    hece::background_main()
}
