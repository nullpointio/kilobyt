//! Kilobyt engine — makes a photo provably fit a hard upload limit.
//!
//! This crate is the whole product logic (PRD §5): decode JPEG/PNG/BMP, bake EXIF
//! orientation into the pixels, plan dimensions, resample with a box filter, search
//! JPEG quality against a byte target, and commit without ever overwriting a file.
//!
//! Public surface (the GUI codes against exactly this):
//!
//! ```
//! use kilobyt::{fit, Settings};
//! let s = Settings { target_kb: 100, max_width: 0, max_height: 0 };
//! let out = fit(&[], "IMG_2031.JPG", &s);
//! assert_eq!(out.err_code, kilobyt::E_EMPTY);
//! ```
#![forbid(unsafe_code)]

mod exif;
mod fit;
mod naming;
pub mod selftest;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Error codes, exactly as named in PRD §7.
pub const E_EMPTY: &str = "E_EMPTY";
pub const E_FORMAT: &str = "E_FORMAT";
pub const E_TOOBIG: &str = "E_TOOBIG";
pub const E_DECODE: &str = "E_DECODE";
pub const E_TOO_SMALL: &str = "E_TOO_SMALL";
pub const E_ACCESS: &str = "E_ACCESS";
pub const E_NAME: &str = "E_NAME";
pub const E_UNSTABLE: &str = "E_UNSTABLE";
pub const E_BUDGET: &str = "E_BUDGET";
pub const E_CANCELLED: &str = "E_CANCELLED";

/// Lowest JPEG quality the engine will ever encode (PRD §5.1).
pub const MIN_QUALITY: u8 = 30;
/// Highest JPEG quality the engine will ever encode (PRD §5.1).
pub const MAX_QUALITY: u8 = 95;

/// Per-file settings. `0` means "no limit" for the dimension fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Target size in KB, 1 KB = 1000 bytes (PRD §5.1). Range 1..=99999.
    pub target_kb: u32,
    /// Maximum output width in pixels, 0 = no limit.
    pub max_width: u32,
    /// Maximum output height in pixels, 0 = no limit.
    pub max_height: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            target_kb: 100,
            max_width: 0,
            max_height: 0,
        }
    }
}

/// Outcome of one file.
///
/// On failure `ok == false`, `jpeg` is empty and **no file may be written**
/// (PRD §5.2.8): a file that misses the limit is worse than no file.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub ok: bool,
    pub jpeg: Vec<u8>,
    pub quality: u8,
    pub width: u32,
    pub height: u32,
    pub probes: u32,
    pub best_bytes: u64,
    /// `""` on success, otherwise one of the `E_*` constants.
    pub err_code: &'static str,
    /// User-facing wording from PRD §7; `""` on success.
    pub err_detail: String,
}

impl Default for Outcome {
    fn default() -> Self {
        Self::fail("", String::new())
    }
}

impl Outcome {
    pub fn success(jpeg: Vec<u8>, quality: u8, width: u32, height: u32, probes: u32) -> Self {
        let best_bytes = jpeg.len() as u64;
        Self {
            ok: true,
            jpeg,
            quality,
            width,
            height,
            probes,
            best_bytes,
            err_code: "",
            err_detail: String::new(),
        }
    }

    pub fn fail(code: &'static str, detail: String) -> Self {
        Self {
            ok: false,
            jpeg: Vec::new(),
            quality: 0,
            width: 0,
            height: 0,
            probes: 0,
            best_bytes: 0,
            err_code: code,
            err_detail: detail,
        }
    }
}

/// Fit one in-memory file. `name` is used only for the extension rule (E_FORMAT);
/// pass the source file name so `IMG_2031.HEIC` is rejected before decoding.
pub fn fit(file: &[u8], name: &str, s: &Settings) -> Outcome {
    fit::run(file, name, s, &fit::NEVER_CANCEL, fit::PROBE_BUDGET)
}

/// Same as [`fit`], but observes a cancellation flag between encode probes
/// (PRD §5.7). Returns `E_CANCELLED` and no output when set.
pub fn fit_cancellable(
    file: &[u8],
    name: &str,
    s: &Settings,
    cancel: &std::sync::atomic::AtomicBool,
) -> Outcome {
    fit::run(file, name, s, cancel, fit::PROBE_BUDGET)
}

/// Read `path` and fit it. A vanished file reports `E_ACCESS` / `File is gone.`
pub fn fit_file(path: &Path, s: &Settings) -> Outcome {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match std::fs::read(path) {
        Ok(bytes) => fit(&bytes, &name, s),
        Err(_) => Outcome::fail(E_ACCESS, "File is gone.".to_string()),
    }
}

/// Plan output dimensions: fit within the limits, never upscale, aspect error ≤ 1 px.
pub fn plan_size(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    fit::plan_size(w, h, max_w, max_h)
}

/// Trim trailing dots/spaces (Windows rule); empty becomes `image`.
pub fn sanitise_stem(name: &str) -> String {
    naming::sanitise_stem(name)
}

/// Derive a sanitised stem from a source file name (`IMG_2031.JPG` → `IMG_2031`).
pub fn stem_of(name: &str) -> String {
    let stem = Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    naming::sanitise_stem(&stem)
}

/// Write `jpeg` as `{stem}_fit.jpg` (then `-2` … `-999`) in `dir`, never overwriting.
///
/// `reserved` holds names already handed out by the batch; the returned path is added
/// to it. On success the file exists on disk with exactly `jpeg.len()` bytes.
///
/// On failure the `Err` string is `"<CODE>: <user message>"`, e.g.
/// `"E_NAME: Too many files named IMG_fit — rename the source files."`.
pub fn commit(
    jpeg: &[u8],
    dir: &Path,
    stem: &str,
    reserved: &mut HashSet<String>,
) -> Result<PathBuf, String> {
    naming::commit(jpeg, dir, stem, reserved)
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::io::Cursor;

    /// Deterministic, photo-like noise: a smooth 64-pixel lattice with fine grain on
    /// top (a coarse lattice stays smooth after a 5× downscale; an 8-pixel one would
    /// alias into near-Nyquist noise and no encoder could then meet a small target). Compressibility is realistic (uniform per-pixel entropy is not a photo and
    /// no encoder can hit 100 KB from it at any sane size). PRD fixtures are generated
    /// in code, never checked in.
    pub fn noise_rgb(w: u32, h: u32, seed: u32) -> Vec<u8> {
        let mut state = seed | 1;
        let mut rnd = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };

        const CELL: usize = 96;
        let cw = (w as usize / CELL) + 2;
        let ch = (h as usize / CELL) + 2;
        let mut lattice = vec![0u8; cw * ch * 3];
        for b in lattice.iter_mut() {
            *b = (rnd() >> 24) as u8;
        }

        let mut px = vec![0u8; (w as usize) * (h as usize) * 3];
        for y in 0..h as usize {
            let gy = y / CELL;
            let fy = (y % CELL) as f64 / CELL as f64;
            for x in 0..w as usize {
                let gx = x / CELL;
                let fx = (x % CELL) as f64 / CELL as f64;
                let grain = ((rnd() >> 30) as i32) - 1; // -1..2, sensor-grain scale
                let o = (y * w as usize + x) * 3;
                for c in 0..3 {
                    let at = |gx: usize, gy: usize| i32::from(lattice[(gy * cw + gx) * 3 + c]);
                    let top = f64::from(at(gx, gy)) * (1.0 - fx) + f64::from(at(gx + 1, gy)) * fx;
                    let bottom =
                        f64::from(at(gx, gy + 1)) * (1.0 - fx) + f64::from(at(gx + 1, gy + 1)) * fx;
                    let v = top * (1.0 - fy) + bottom * fy;
                    px[o + c] = (v as i32 + grain).clamp(0, 255) as u8;
                }
            }
        }
        px
    }

    /// Pure per-pixel entropy: incompressible by design, for "target unreachable" tests.
    pub fn flat_noise_rgb(w: u32, h: u32, seed: u32) -> Vec<u8> {
        let mut state = seed | 1;
        let mut px = vec![0u8; (w as usize) * (h as usize) * 3];
        for b in px.iter_mut() {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *b = (state >> 24) as u8;
        }
        px
    }

    /// Smooth horizontal+vertical gradient, handy for exact averaging checks.
    pub fn gradient_rgb(w: u32, h: u32) -> Vec<u8> {
        let mut px = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                px.push((x * 255 / w.max(1)) as u8);
                px.push((y * 255 / h.max(1)) as u8);
                px.push(((x + y) * 255 / (w + h).max(1)) as u8);
            }
        }
        px
    }

    pub fn encode_jpeg(px: &[u8], w: u32, h: u32, quality: u8) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
        enc.encode(px, w, h, image::ExtendedColorType::Rgb8)
            .expect("encode fixture");
        out
    }

    pub fn decode_jpeg(bytes: &[u8]) -> (Vec<u8>, u32, u32) {
        let img = image::load_from_memory(bytes)
            .expect("decode fixture")
            .to_rgb8();
        (img.as_raw().clone(), img.width(), img.height())
    }

    pub fn encode_png_rgba(px: &[u8], w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_raw(w, h, px.to_vec()).expect("rgba fixture");
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("png fixture");
        out
    }

    pub fn encode_bmp(px: &[u8], w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_raw(w, h, px.to_vec()).expect("bmp fixture");
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Bmp)
            .expect("bmp fixture");
        out
    }

    /// Minimal EXIF APP1 carrying only IFD0 Orientation. JPEG segment lengths are
    /// BIG-endian; the TIFF header may be either byte order.
    pub fn with_exif_orientation(jpeg: &[u8], orientation: u16, little_endian: bool) -> Vec<u8> {
        assert!(
            jpeg.len() > 2 && jpeg[0] == 0xFF && jpeg[1] == 0xD8,
            "need SOI"
        );
        let mut tiff = Vec::new();
        if little_endian {
            tiff.extend_from_slice(b"II");
            tiff.extend_from_slice(&42u16.to_le_bytes());
            tiff.extend_from_slice(&8u32.to_le_bytes());
            tiff.extend_from_slice(&1u16.to_le_bytes());
            tiff.extend_from_slice(&0x0112u16.to_le_bytes());
            tiff.extend_from_slice(&3u16.to_le_bytes());
            tiff.extend_from_slice(&1u32.to_le_bytes());
            tiff.extend_from_slice(&orientation.to_le_bytes());
            tiff.extend_from_slice(&[0, 0]);
            tiff.extend_from_slice(&0u32.to_le_bytes());
        } else {
            tiff.extend_from_slice(b"MM");
            tiff.extend_from_slice(&42u16.to_be_bytes());
            tiff.extend_from_slice(&8u32.to_be_bytes());
            tiff.extend_from_slice(&1u16.to_be_bytes());
            tiff.extend_from_slice(&0x0112u16.to_be_bytes());
            tiff.extend_from_slice(&3u16.to_be_bytes());
            tiff.extend_from_slice(&1u32.to_be_bytes());
            tiff.extend_from_slice(&orientation.to_be_bytes());
            tiff.extend_from_slice(&[0, 0]);
            tiff.extend_from_slice(&0u32.to_be_bytes());
        }

        let mut payload = Vec::new();
        payload.extend_from_slice(b"Exif\0\0");
        payload.extend_from_slice(&tiff);

        let mut out = Vec::with_capacity(jpeg.len() + payload.len() + 4);
        out.extend_from_slice(&jpeg[..2]); // SOI
        out.push(0xFF);
        out.push(0xE1);
        let seg_len = (payload.len() + 2) as u16; // length includes itself, big-endian
        out.extend_from_slice(&seg_len.to_be_bytes());
        out.extend_from_slice(&payload);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    pub fn temp_dir(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("kilobyt-test-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }
}
