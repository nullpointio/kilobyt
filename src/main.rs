//! Kilobyt entry point.
//!
//! `kilobyt` starts the Windows GUI; `-selftest` exercises the engine headlessly
//! (used by CI on every platform); `-version` prints the build version.

#![cfg_attr(all(windows, not(feature = "console")), windows_subsystem = "windows")]

#[cfg(windows)]
mod ui_windows;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "-version" || a == "--version") {
        println!("kilobyt {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    if args.iter().any(|a| a == "-selftest" || a == "--selftest") {
        return match kilobyt::selftest::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("self-test failed: {message}");
                ExitCode::FAILURE
            }
        };
    }

    #[cfg(windows)]
    {
        let _ = ui_windows::run();
        ExitCode::SUCCESS
    }

    #[cfg(not(windows))]
    {
        println!(
            "Kilobyt {} — engine build for {}. The window is Windows-only; \
             run `kilobyt -selftest` to exercise the compressor.",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS
        );
        ExitCode::SUCCESS
    }
}
