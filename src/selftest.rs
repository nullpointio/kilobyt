//! `kilobyt -selftest`: proves the engine end-to-end on this machine, with no GUI.
//!
//! Generates a 4000×3000 deterministic photo in memory, fits it to 100 KB, checks
//! the size and decodability, then commits to a temp directory, re-reads the file
//! length from disk and removes it again. One line is printed per check.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::{commit, fit, Settings};

/// Bytes a 100 KB target may not exceed (1 KB = 1000 bytes).
const TARGET_KB: u32 = 100;
const TARGET_BYTES: usize = 100_000;
const WIDTH: u32 = 4000;
const HEIGHT: u32 = 3000;

/// Run every check. `Err` carries the first failing check's message.
pub fn run() -> Result<(), String> {
    println!("kilobyt {} self-test", env!("CARGO_PKG_VERSION"));

    let jpeg_in = fixture_jpeg(WIDTH, HEIGHT);
    check(
        "fixture",
        jpeg_in.len() > TARGET_BYTES,
        format!("{}×{} JPEG, {} bytes", WIDTH, HEIGHT, jpeg_in.len()),
    )?;

    let settings = Settings {
        target_kb: TARGET_KB,
        max_width: 0,
        max_height: 0,
    };
    let outcome = fit(&jpeg_in, "selftest.jpg", &settings);
    if !outcome.ok {
        return Err(format!(
            "fit failed: {} {}",
            outcome.err_code, outcome.err_detail
        ));
    }
    check(
        "fit",
        outcome.jpeg.len() <= TARGET_BYTES,
        format!(
            "{} bytes <= {} (q{}, {}×{}, {} probes)",
            outcome.jpeg.len(),
            TARGET_BYTES,
            outcome.quality,
            outcome.width,
            outcome.height,
            outcome.probes
        ),
    )?;

    let decoded = image::load_from_memory(&outcome.jpeg)
        .map_err(|e| format!("output does not decode: {e}"))?;
    check(
        "decode",
        decoded.width() == outcome.width && decoded.height() == outcome.height,
        format!("{}×{} output decodes", decoded.width(), decoded.height()),
    )?;

    check(
        "quality",
        (crate::MIN_QUALITY..=crate::MAX_QUALITY).contains(&outcome.quality),
        format!("q{} within 30..=95", outcome.quality),
    )?;
    check(
        "probes",
        outcome.probes <= 200,
        format!("{} probes <= 200", outcome.probes),
    )?;

    let dir = temp_root();
    let mut reserved = HashSet::new();
    let written = commit(&outcome.jpeg, &dir, "selftest", &mut reserved)
        .map_err(|e| format!("commit failed: {e}"))?;
    let on_disk = std::fs::metadata(&written)
        .map(|m| m.len())
        .map_err(|e| format!("cannot stat {}: {e}", written.display()))?;
    check(
        "commit",
        on_disk == outcome.jpeg.len() as u64,
        format!("{} on disk, {} bytes", written.display(), on_disk),
    )?;

    let reread = std::fs::read(&written).map_err(|e| format!("cannot re-read: {e}"))?;
    check(
        "verify",
        reread == outcome.jpeg,
        "re-read bytes match the encoded output".to_string(),
    )?;

    let _ = std::fs::remove_file(&written);
    let _ = std::fs::remove_dir(&dir);
    check(
        "cleanup",
        !written.exists(),
        "temp file removed".to_string(),
    )?;

    println!("self-test passed");
    Ok(())
}

fn check(name: &str, ok: bool, detail: String) -> Result<(), String> {
    if ok {
        println!("  ok   {name:<8} {detail}");
        Ok(())
    } else {
        println!("  FAIL {name:<8} {detail}");
        Err(format!("{name}: {detail}"))
    }
}

fn temp_root() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kilobyt-selftest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// The 12 MP fixture `-mem` measures: 4000×3000, deterministic, generated in RAM.
pub fn twelve_megapixel_jpeg() -> Vec<u8> {
    fixture_jpeg(WIDTH, HEIGHT)
}

/// A deterministic 4000×3000 photo-like JPEG: noise plus a slow gradient so the
/// encoder sees structure, not just entropy. Local PRNG: no fixture files, no deps.
fn fixture_jpeg(w: u32, h: u32) -> Vec<u8> {
    let mut state = 0x1234_5678u32;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };

    let mut px = vec![0u8; (w as usize) * (h as usize) * 3];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let i = (y * w as usize + x) * 3;
            let r = next();
            let base_x = (x * 255 / w as usize) as u8;
            let base_y = (y * 255 / h as usize) as u8;
            px[i] = base_x.wrapping_add((r >> 24) as u8 & 0x3F);
            px[i + 1] = base_y.wrapping_add((r >> 16) as u8 & 0x3F);
            px[i + 2] = (((r >> 8) as u8) & 0x7F).wrapping_add(base_x / 2);
        }
    }

    let mut out = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92);
    encoder
        .encode(&px, w, h, image::ExtendedColorType::Rgb8)
        .expect("fixture encode");
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn fixture_is_a_real_jpeg() {
        let bytes = super::fixture_jpeg(64, 48);
        assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
        let img = image::load_from_memory(&bytes).expect("decodes");
        assert_eq!((img.width(), img.height()), (64, 48));
    }
}
