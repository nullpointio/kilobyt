//! Kilobyt entry point.
//!
//! `kilobyt` starts the Windows GUI; `-selftest` exercises the engine headlessly
//! (used by CI on every platform); `-mem` reports peak RSS for a 12 MP run;
//! `-version` prints the build version.

#![cfg_attr(all(windows, not(feature = "console")), windows_subsystem = "windows")]

#[cfg(windows)]
mod ui_windows;

use std::process::ExitCode;

use kilobyt::{fit_owned, Settings};

const MEM_WIDTH: u32 = 4000;
const MEM_HEIGHT: u32 = 3000;

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

    if let Some(index) = args.iter().position(|a| a == "-mem" || a == "--mem") {
        let path = args[index + 1..].iter().find(|a| !a.starts_with('-'));
        return mem_mode(path.map(String::as_str));
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

/// `-mem [path]`: fit one 12 MP image (the built-in fixture, or `path`) and print
/// the peak resident set size observed for that fit. Numbers are the process'
/// own, read from the OS — never estimated from buffer arithmetic.
fn mem_mode(path: Option<&str>) -> ExitCode {
    let settings = Settings {
        target_kb: 100,
        max_width: 0,
        max_height: 0,
    };

    let (label, jpeg) = match path {
        Some(p) => match std::fs::read(p) {
            Ok(bytes) => (p.to_string(), bytes),
            Err(e) => {
                eprintln!("cannot read {p}: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => (
            format!("{MEM_WIDTH}×{MEM_HEIGHT} fixture"),
            kilobyt::selftest::twelve_megapixel_jpeg(),
        ),
    };

    let megapixels = match image_dimensions(&jpeg) {
        Some((w, h)) => format!(
            "{:.1} MP",
            (u64::from(w) * u64::from(h)) as f64 / 1_000_000.0
        ),
        None => "unknown".to_string(),
    };

    println!("kilobyt {} -mem", env!("CARGO_PKG_VERSION"));
    println!(
        "  input    {label}, {megapixels}, {} bytes, target {} KB",
        jpeg.len(),
        settings.target_kb
    );
    if path.is_none() {
        println!("  note     the generated fixture's own bytes are resident; pass a path for a clean number");
    }

    // Reset the peak so the number covers the fit and nothing else. Platforms
    // without a reset report the whole-process peak since launch.
    reset_peak_rss();

    let outcome = fit_owned(jpeg, "mem.jpg", &settings);
    let peak = peak_rss_bytes();

    if outcome.ok {
        println!(
            "  fit      {} bytes, q{}, {}×{}, {} probes",
            outcome.jpeg.len(),
            outcome.quality,
            outcome.width,
            outcome.height,
            outcome.probes
        );
    } else {
        println!(
            "  fit      failed: {} {} ({} probes)",
            outcome.err_code, outcome.err_detail, outcome.probes
        );
    }

    match peak {
        Some(bytes) => {
            println!(
                "  peak RSS {:.1} MiB ({} bytes)",
                bytes as f64 / 1_048_576.0,
                bytes
            );
            // The measurement is the product here; a fit failure is reported above.
            ExitCode::SUCCESS
        }
        None => {
            println!("  peak RSS unavailable on this platform");
            ExitCode::FAILURE
        }
    }
}

/// Width/height from the header only (no pixel buffer is allocated).
fn image_dimensions(file: &[u8]) -> Option<(u32, u32)> {
    kilobyt::image_dimensions(file)
}

#[cfg(target_os = "linux")]
fn peak_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let rest = line.strip_prefix("VmHWM:")?;
        let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
        Some(kb * 1024)
    })
}

#[cfg(target_os = "linux")]
fn reset_peak_rss() {
    // clear_refs=5 resets VmHWM to the current RSS (Linux ≥ 4.0).
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}

#[cfg(all(windows, not(target_os = "linux")))]
fn peak_rss_bytes() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    // SAFETY: both calls take a handle this process owns and a struct we pass by
    // pointer with its exact size; the API only writes into that struct.
    unsafe {
        let mut counters: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        let ok = GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        );
        (ok != 0).then_some(counters.PeakWorkingSetSize as u64)
    }
}

#[cfg(all(windows, not(target_os = "linux")))]
fn reset_peak_rss() {
    // Windows has no portable way to lower the working-set peak; the reported
    // number therefore covers everything since launch, fixture included.
}

#[cfg(not(any(target_os = "linux", windows)))]
fn peak_rss_bytes() -> Option<u64> {
    None
}

#[cfg(not(any(target_os = "linux", windows)))]
fn reset_peak_rss() {}
