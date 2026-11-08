//! PRD §5.2 pipeline: decode → orient → plan → resample → quality search →
//! downscale loop → encode. Nothing here touches the file system.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::exif;
use crate::{
    Outcome, Settings, E_BUDGET, E_CANCELLED, E_DECODE, E_EMPTY, E_FORMAT, E_TOOBIG,
    E_TOO_MANY_PIXELS, E_TOO_SMALL, MAX_PIXELS, MAX_QUALITY, MIN_QUALITY,
};

/// Hard cap on encode probes per file (PRD §5.3).
pub(crate) const PROBE_BUDGET: u32 = 200;
/// Downscale iterations after q=30 still misses the target (PRD §5.2.7).
pub(crate) const MAX_DOWNSCALES: u32 = 8;
/// Never downscale below this on either side (PRD §5.2.7).
pub(crate) const MIN_DIMENSION: u32 = 64;
const DOWNSCALE_FACTOR: f64 = 0.85;

pub(crate) const MAX_FILE_BYTES: u64 = 200_000_000;
// The pixel cap itself lives in lib.rs (`crate::MAX_PIXELS`) so the public surface
// and the docs can state it; it is enforced here before any buffer is allocated.

/// A cancellation flag that is never set, for the non-cancellable API.
pub(crate) static NEVER_CANCEL: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// Messages (PRD §7, verbatim)
// ---------------------------------------------------------------------------

pub(crate) fn msg_empty() -> String {
    "Empty file.".to_string()
}

pub(crate) fn msg_decode() -> String {
    "Cannot read this image — the file may be damaged or use an unsupported format.".to_string()
}

pub(crate) fn msg_format() -> String {
    "Only JPG, PNG and BMP are supported. Convert HEIC/TIFF/WebP first.".to_string()
}

pub(crate) fn msg_unstable() -> String {
    "Encoder produced an oversized file 3 times; skipped.".to_string()
}

pub(crate) fn msg_budget() -> String {
    "Internal error (E_BUDGET) — please report this file.".to_string()
}

pub(crate) fn msg_cancelled() -> String {
    "Cancelled.".to_string()
}

pub(crate) fn msg_too_big_bytes(bytes: u64) -> String {
    format!(
        "Too large: {} MB. Limit is 200 MB.",
        format_1dp(bytes, 1_000_000)
    )
}

pub(crate) fn msg_too_big_pixels(pixels: u64) -> String {
    format!(
        "Too large: {} MP. Limit is {} MP.",
        format_1dp(pixels, 1_000_000),
        MAX_PIXELS / 1_000_000
    )
}

pub(crate) fn msg_too_small(target_bytes: u64, best_bytes: u64, w: u32, h: u32) -> String {
    format!(
        "Cannot reach {} KB. The smallest this photo can be is {} KB at {w}×{h}.",
        format_kb(target_bytes),
        format_kb(best_bytes)
    )
}

pub(crate) fn msg_access(dir: &std::path::Path) -> String {
    format!(
        "Cannot write to {} — folder is read-only or missing.",
        dir.display()
    )
}

pub(crate) fn msg_name(stem: &str) -> String {
    format!("Too many files named {stem}_fit — rename the source files.")
}

/// `0.#` formatting, culture-invariant, as in the C# reference.
fn format_1dp(value: u64, unit: u64) -> String {
    let tenths = (value as f64 * 10.0 / unit as f64).round() as u64;
    let whole = tenths / 10;
    let frac = tenths % 10;
    if frac == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{frac}")
    }
}

fn format_kb(bytes: u64) -> String {
    format_1dp(bytes, 1000)
}

// ---------------------------------------------------------------------------
// Extension rule
// ---------------------------------------------------------------------------

fn extension(name: &str) -> Option<String> {
    let dot = name.rfind('.')?;
    let ext = &name[dot + 1..];
    if ext.is_empty() || ext.contains(['/', '\\']) {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

pub(crate) fn supported_extension(name: &str) -> bool {
    matches!(
        extension(name).as_deref(),
        Some("jpg" | "jpeg" | "png" | "bmp")
    )
}

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

struct Decoded {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    dpi: u16,
    orientation: u8,
}

/// Decode by content, flattening alpha over white (PRD §5.2.2).
fn decode(file: &[u8]) -> Option<Decoded> {
    let img = image::load_from_memory(file).ok()?;
    let (w, h) = (img.width(), img.height());
    if w < 1 || h < 1 {
        return None;
    }

    let pixels = if img.color().has_alpha() {
        // Composite in place: packing RGBA down to RGB in the same buffer keeps
        // one allocation (4 bytes/px) instead of holding RGBA and RGB at once.
        let mut raw = img.into_rgba8().into_raw();
        let (mut src, mut dst) = (0usize, 0usize);
        while src + 3 < raw.len() {
            let a = u32::from(raw[src + 3]);
            let (r, g, b) = if a == 255 {
                (raw[src], raw[src + 1], raw[src + 2])
            } else {
                // Composite over white, round-half-up (§5.2.2).
                let over = |c: u8| {
                    let c = u32::from(c);
                    ((c * a + 255 * (255 - a) + 127) / 255) as u8
                };
                (over(raw[src]), over(raw[src + 1]), over(raw[src + 2]))
            };
            raw[dst] = r;
            raw[dst + 1] = g;
            raw[dst + 2] = b;
            src += 4;
            dst += 3;
        }
        raw.truncate(dst);
        raw
    } else {
        img.into_rgb8().into_raw()
    };

    if pixels.len() != (w as usize) * (h as usize) * 3 {
        return None;
    }

    Some(Decoded {
        pixels,
        width: w,
        height: h,
        dpi: exif::choose_dpi(exif::jfif_density(file)),
        orientation: exif::orientation(file),
    })
}

/// Width×height read from the container header alone — no pixel buffer allocated.
pub(crate) fn header_dimensions(file: &[u8]) -> Option<(u32, u32)> {
    let reader = image::ImageReader::new(std::io::Cursor::new(file))
        .with_guessed_format()
        .ok()?;
    reader.into_dimensions().ok()
}

// ---------------------------------------------------------------------------
// Sizing and resampling
// ---------------------------------------------------------------------------

/// PRD §5.2.4. `scale` is the minimum of every provided limit and 1.0.
pub(crate) fn plan_size(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let (w, h) = (w.max(1), h.max(1));
    let mut scale = 1.0f64;
    if max_w > 0 {
        scale = scale.min(max_w as f64 / w as f64);
    }
    if max_h > 0 {
        scale = scale.min(max_h as f64 / h as f64);
    }
    scale = scale.min(1.0);

    let new_w = clamp_dim(round_half_away(w as f64 * scale), w);
    let new_h = clamp_dim(round_half_away(h as f64 * scale), h);
    (new_w, new_h)
}

fn round_half_away(v: f64) -> u32 {
    // Values here are always positive and well inside u32 for real inputs.
    (v + 0.5).floor().max(0.0) as u32
}

fn clamp_dim(value: u32, max: u32) -> u32 {
    value.clamp(1, max.max(1))
}

/// PRD §5.2.5 — area-average (box) filter, integer accumulation, round-half-up.
/// Each destination pixel averages the source rectangle it covers.
pub(crate) fn box_resize(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    if dw == sw && dh == sh {
        return src.to_vec();
    }
    let (sw, sh, dw, dh) = (sw as usize, sh as usize, dw as usize, dh as usize);
    debug_assert!(dw <= sw && dh <= sh && dw >= 1 && dh >= 1);
    debug_assert_eq!(src.len(), sw * sh * 3);

    let xs: Vec<usize> = (0..=dw).map(|i| i * sw / dw).collect();
    let ys: Vec<usize> = (0..=dh).map(|j| j * sh / dh).collect();

    let mut out = vec![0u8; dw * dh * 3];
    for y in 0..dh {
        let y0 = ys[y];
        let y1 = (ys[y + 1]).max(y0 + 1).min(sh);
        for x in 0..dw {
            let x0 = xs[x];
            let x1 = (xs[x + 1]).max(x0 + 1).min(sw);

            let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
            for sy in y0..y1 {
                let row = sy * sw * 3;
                for sx in x0..x1 {
                    let i = row + sx * 3;
                    r += u64::from(src[i]);
                    g += u64::from(src[i + 1]);
                    b += u64::from(src[i + 2]);
                    n += 1;
                }
            }
            if n == 0 {
                continue;
            }
            let o = (y * dw + x) * 3;
            out[o] = ((r + n / 2) / n) as u8;
            out[o + 1] = ((g + n / 2) / n) as u8;
            out[o + 2] = ((b + n / 2) / n) as u8;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Encode
// ---------------------------------------------------------------------------

/// JPEG output buffer, allocated once per file and reused by every quality probe.
///
/// The quality search encodes the same picture at up to ~40 different qualities;
/// each output can be tens of MB at high q, so a fresh `Vec` per probe meant the
/// allocator held (and zeroed, and grew) a new multi-MB buffer each time. One
/// retained buffer keeps the run's encode footprint flat at the largest single
/// output seen, and lets the final encode hand its buffer straight to `Outcome`.
pub(crate) struct JpegSink {
    buf: Vec<u8>,
    /// Buffers the allocator handed out (growth events), for byte accounting.
    allocations: u32,
}

/// Write adapter that grows the retained buffer in tight steps instead of
/// letting `Vec` double it. A doubled buffer costs up to twice the size of the
/// JPEG we just produced, and the copy at the reallocation moment keeps the old
/// and new allocations alive together. Growing to ~1.25x the bytes needed caps
/// both the resident buffer and that transient.
struct RetainedBuf<'a> {
    buf: &'a mut Vec<u8>,
}

impl std::io::Write for RetainedBuf<'_> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let need = self.buf.len() + data.len();
        if need > self.buf.capacity() {
            // Round up to 64 KiB so the header and per-scan writes do not each
            // trigger a reallocation.
            let target = need.saturating_add(need / 4).saturating_add(65_535) & !65_535;
            self.buf.reserve_exact(target - self.buf.len());
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl JpegSink {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            allocations: 0,
        }
    }

    /// Encode RGB24 as baseline JPEG at `q` with the JFIF density patched in.
    /// The encoder writes no EXIF/XMP/GPS/comment segments (PRD §5.2.13).
    ///
    /// Returns the encoded length; the bytes stay in the sink's buffer.
    fn encode(&mut self, pixels: &[u8], w: u32, h: u32, q: u8, dpi: u16) -> Result<usize, String> {
        if w == 0 || h == 0 || w > 65_535 || h > 65_535 {
            return Err("unsupported dimensions".to_string());
        }
        if pixels.len() != (w as usize) * (h as usize) * 3 {
            return Err("pixel buffer does not match dimensions".to_string());
        }
        let quality = q.clamp(MIN_QUALITY, MAX_QUALITY);
        let before = self.buf.capacity();
        self.buf.clear();
        {
            let mut out = RetainedBuf { buf: &mut self.buf };
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
            enc.encode(pixels, w, h, image::ExtendedColorType::Rgb8)
                .map_err(|e| e.to_string())?;
        }
        if self.buf.capacity() != before {
            self.allocations += 1;
        }
        patch_jfif_density(&mut self.buf, dpi);
        Ok(self.buf.len())
    }

    fn bytes(&self) -> &[u8] {
        &self.buf
    }

    /// Hand the finished JPEG to the caller (final encode only).
    fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buf)
    }

    /// Drop the output buffer without producing anything.
    ///
    /// Called at a downscale boundary: the next probe encodes a smaller image, and
    /// the resample needs its destination buffer at the same moment, so keeping
    /// the old (larger) output alive would add both to the peak.
    fn release(&mut self) {
        self.buf = Vec::new();
    }
}

/// Rewrite the JFIF APP0 density (units = inches). No-op when the encoder did not
/// emit a JFIF APP0 segment.
fn patch_jfif_density(jpeg: &mut [u8], dpi: u16) {
    if jpeg.len() < 18 || jpeg[0] != 0xFF || jpeg[1] != 0xD8 {
        return;
    }
    if jpeg[2] != 0xFF || jpeg[3] != 0xE0 || &jpeg[6..11] != b"JFIF\0" {
        return;
    }
    let d = dpi.clamp(1, 1200).to_be_bytes();
    jpeg[13] = 1; // units: dots per inch
    jpeg[14] = d[0];
    jpeg[15] = d[1];
    jpeg[16] = d[0];
    jpeg[17] = d[1];
}

// ---------------------------------------------------------------------------
// Quality search
// ---------------------------------------------------------------------------

/// Why a probe sequence stopped early.
enum Stop {
    Budget,
    Cancelled,
    Encode,
}

struct Prober<'a> {
    target: u64,
    budget: u32,
    probes: u32,
    best_bytes: u64,
    best_quality: u8,
    best_w: u32,
    best_h: u32,
    dpi: u16,
    cancel: &'a AtomicBool,
    /// One output buffer shared by every probe and the final encode.
    enc: JpegSink,
}

impl Prober<'_> {
    /// Encode one probe at `q`, tracking the smallest output seen so far.
    fn probe(&mut self, px: &[u8], w: u32, h: u32, q: u8) -> Result<u64, Stop> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Stop::Cancelled);
        }
        if self.probes >= self.budget {
            return Err(Stop::Budget);
        }
        self.probes += 1;
        let len = self
            .enc
            .encode(px, w, h, q, self.dpi)
            .map_err(|_| Stop::Encode)? as u64;
        if len < self.best_bytes {
            self.best_bytes = len;
            self.best_quality = q;
            self.best_w = w;
            self.best_h = h;
        }
        Ok(len)
    }

    /// PRD §5.2.6 — descending ladder 95..30 step 5, then integer bisect in
    /// `(q_hi, q_hi + 5]` for the highest passing quality (≤ 4 extra probes).
    fn find(&mut self, px: &[u8], w: u32, h: u32) -> Result<Option<u8>, Stop> {
        let mut q_hi: Option<u8> = None;
        let mut q = MAX_QUALITY;
        loop {
            if self.probe(px, w, h, q)? <= self.target {
                q_hi = Some(q);
                break;
            }
            if q == MIN_QUALITY {
                break;
            }
            q -= 5;
        }

        let Some(q_hi) = q_hi else { return Ok(None) };

        let mut best = q_hi;
        let mut lo = u32::from(q_hi) + 1;
        let mut hi = (u32::from(q_hi) + 4).min(u32::from(MAX_QUALITY));
        while lo <= hi {
            let mid = (lo + hi) / 2;
            if self.probe(px, w, h, mid as u8)? <= self.target {
                best = mid as u8;
                lo = mid + 1;
            } else {
                hi = mid - 1;
            }
        }
        Ok(Some(best))
    }
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

/// The encoded source file, either borrowed from the caller or owned by the engine.
///
/// The bytes are only needed for the extension rule, the EXIF/JFIF probes and the
/// decode. Once the pixels exist they are dead weight — for a 12 MP JPEG that is
/// ~17 MB, and for a 200 MB input it is most of the run's footprint — so an owned
/// input is released before the quality search starts.
pub(crate) enum Input<'a> {
    Borrowed(&'a [u8]),
    Owned(Vec<u8>),
}

impl Input<'_> {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Borrowed(b) => b,
            Self::Owned(v) => v,
        }
    }

    /// Free the file bytes if the engine owns them; a borrowed input is untouched.
    fn release(&mut self) {
        if matches!(self, Self::Owned(_)) {
            *self = Self::Borrowed(&[]);
        }
    }
}

pub(crate) fn run(
    file: &[u8],
    name: &str,
    s: &Settings,
    cancel: &AtomicBool,
    budget: u32,
) -> Outcome {
    run_input(Input::Borrowed(file), name, s, cancel, budget)
}

pub(crate) fn run_owned(
    file: Vec<u8>,
    name: &str,
    s: &Settings,
    cancel: &AtomicBool,
    budget: u32,
) -> Outcome {
    run_input(Input::Owned(file), name, s, cancel, budget)
}

fn run_input(
    mut input: Input<'_>,
    name: &str,
    s: &Settings,
    cancel: &AtomicBool,
    budget: u32,
) -> Outcome {
    let file = input.bytes();
    // 1. Validate. Empty beats everything else, then the extension rule.
    if file.is_empty() {
        return Outcome::fail(E_EMPTY, msg_empty());
    }
    if !supported_extension(name) {
        return Outcome::fail(E_FORMAT, msg_format());
    }
    if file.len() as u64 > MAX_FILE_BYTES {
        return Outcome::fail(E_TOOBIG, msg_too_big_bytes(file.len() as u64));
    }
    if cancel.load(Ordering::Relaxed) {
        return Outcome::fail(E_CANCELLED, msg_cancelled());
    }

    // 2. Decode. Refuse an oversized image from its header first: the pixel cap
    //    is only worth anything if it fires before the pixel buffer is allocated.
    if let Some((w, h)) = header_dimensions(file) {
        let pixels = u64::from(w) * u64::from(h);
        if pixels > MAX_PIXELS {
            return Outcome::fail(E_TOO_MANY_PIXELS, msg_too_big_pixels(pixels));
        }
    }
    let Some(decoded) = decode(file) else {
        return Outcome::fail(E_DECODE, msg_decode());
    };
    // The pixels exist now: an owned file buffer is dead weight from here on.
    input.release();
    // Backstop: a format whose header under-reported its real size.
    let pixels = decoded.pixels.len() as u64 / 3;
    if pixels > MAX_PIXELS {
        return Outcome::fail(E_TOO_MANY_PIXELS, msg_too_big_pixels(pixels));
    }

    // 3. Bake orientation so the output needs no tag.
    let (pixels, src_w, src_h) = exif::apply(
        decoded.pixels,
        decoded.width,
        decoded.height,
        decoded.orientation,
    );
    if src_w < 1 || src_h < 1 || pixels.len() != (src_w as usize) * (src_h as usize) * 3 {
        return Outcome::fail(E_DECODE, msg_decode());
    }

    // 4. Plan dimensions (never upscale).
    let (plan_w, plan_h) = plan_size(src_w, src_h, s.max_width, s.max_height);

    // 5. Resample once; the downscale loop re-samples from the oriented original.
    let mut buf_w = plan_w;
    let mut buf_h = plan_h;
    let mut buf = if plan_w == src_w && plan_h == src_h {
        Vec::new() // empty means "the oriented source is already the working buffer"
    } else {
        box_resize(&pixels, src_w, src_h, plan_w, plan_h)
    };

    let target_kb = s.target_kb.clamp(1, 99_999);
    let target = u64::from(target_kb) * 1000;
    let mut prober = Prober {
        target,
        budget,
        probes: 0,
        best_bytes: u64::MAX,
        best_quality: 0,
        best_w: plan_w,
        best_h: plan_h,
        dpi: decoded.dpi,
        cancel,
        enc: JpegSink::new(),
    };

    let mut downscales = 0u32;
    loop {
        let current: &[u8] = if buf.is_empty() { &pixels } else { &buf };
        match prober.find(current, buf_w, buf_h) {
            Err(Stop::Cancelled) => {
                return cancelled(prober.probes, prober.best_bytes, buf_w, buf_h);
            }
            Err(Stop::Budget) => {
                return budget_fail(prober.probes, prober.best_bytes, buf_w, buf_h);
            }
            Err(Stop::Encode) => return Outcome::fail(E_DECODE, msg_decode()),
            Ok(Some(q)) => {
                // 10-11. Encode the final image at the winning quality and verify its
                // length. This encode is not a probe and reuses the probe buffer. If
                // the encoder did not reproduce the probed size, step q down by one,
                // at most 3 attempts (§5.2.11).
                let mut quality = q;
                if prober
                    .enc
                    .encode(current, buf_w, buf_h, quality, decoded.dpi)
                    .is_err()
                {
                    return Outcome::fail(E_DECODE, msg_decode());
                }
                let mut attempts = 0;
                while prober.enc.bytes().len() as u64 > target {
                    if attempts == 3 || quality <= MIN_QUALITY {
                        return Outcome::fail(crate::E_UNSTABLE, msg_unstable());
                    }
                    attempts += 1;
                    quality -= 1;
                    if prober
                        .enc
                        .encode(current, buf_w, buf_h, quality, decoded.dpi)
                        .is_err()
                    {
                        return Outcome::fail(E_DECODE, msg_decode());
                    }
                }
                return Outcome::success(prober.enc.take(), quality, buf_w, buf_h, prober.probes);
            }
            Ok(None) => {}
        }

        // 7. Downscale loop, height from the ORIGINAL aspect ratio.
        if downscales >= MAX_DOWNSCALES {
            break;
        }
        if buf_w <= MIN_DIMENSION || buf_h <= MIN_DIMENSION {
            break;
        }
        let next_w = ((buf_w as f64 * DOWNSCALE_FACTOR).floor() as u32).max(MIN_DIMENSION);
        let next_h = round_half_away(next_w as f64 * src_h as f64 / src_w as f64);
        if next_w < MIN_DIMENSION || next_h < MIN_DIMENSION {
            break;
        }
        if next_w >= buf_w || next_h >= buf_h {
            break;
        }
        if cancel.load(Ordering::Relaxed) {
            return cancelled(prober.probes, prober.best_bytes, buf_w, buf_h);
        }

        downscales += 1;
        // The current image is about to be replaced by a smaller one, and the
        // probe buffer is sized for the old one: drop it before the resize so the
        // source, the destination and the encoder output never coexist.
        prober.enc.release();
        // Free the outgoing scaled buffer before the replacement is allocated:
        // the peak for a step is source + next, not source + previous + next.
        drop(std::mem::take(&mut buf));
        buf = box_resize(&pixels, src_w, src_h, next_w, next_h);
        buf_w = next_w;
        buf_h = next_h;
    }

    let best = if prober.best_bytes == u64::MAX {
        0
    } else {
        prober.best_bytes
    };
    let mut out = Outcome::fail(
        E_TOO_SMALL,
        msg_too_small(target, best, prober.best_w, prober.best_h),
    );
    out.quality = prober.best_quality;
    out.probes = prober.probes;
    out.best_bytes = best;
    out.width = prober.best_w;
    out.height = prober.best_h;
    out
}

fn cancelled(probes: u32, best: u64, w: u32, h: u32) -> Outcome {
    let mut out = Outcome::fail(E_CANCELLED, msg_cancelled());
    out.probes = probes;
    out.best_bytes = if best == u64::MAX { 0 } else { best };
    out.width = w;
    out.height = h;
    out
}

fn budget_fail(probes: u32, best: u64, w: u32, h: u32) -> Outcome {
    let mut out = Outcome::fail(E_BUDGET, msg_budget());
    out.probes = probes;
    out.best_bytes = if best == u64::MAX { 0 } else { best };
    out.width = w;
    out.height = h;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use crate::{fit, fit_cancellable, E_ACCESS, E_DECODE, E_TOO_SMALL};

    // --- plan_size -------------------------------------------------------

    #[test]
    fn plan_size_matches_ac_2_3_table() {
        assert_eq!(plan_size(4000, 3000, 600, 0), (600, 450));
        assert_eq!(plan_size(4000, 3000, 0, 0), (4000, 3000));
        assert_eq!(plan_size(4000, 3000, 10_000, 10_000), (4000, 3000));
    }

    #[test]
    fn plan_size_never_upscales() {
        assert_eq!(plan_size(400, 300, 600, 600), (400, 300));
        assert_eq!(plan_size(1, 1, 800, 800), (1, 1));
    }

    #[test]
    fn plan_size_both_limits_pick_the_tighter() {
        assert_eq!(plan_size(4000, 3000, 600, 600), (600, 450));
        assert_eq!(plan_size(3000, 4000, 600, 600), (450, 600));
        assert_eq!(plan_size(4000, 3000, 2000, 900), (1200, 900));
    }

    #[test]
    fn plan_size_zero_means_no_limit() {
        assert_eq!(plan_size(4000, 3000, 0, 900), (1200, 900));
        assert_eq!(plan_size(4000, 3000, 600, 0), (600, 450));
    }

    #[test]
    fn plan_size_keeps_dimensions_at_least_one() {
        assert_eq!(plan_size(10_000, 3, 1, 1), (1, 1));
        assert_eq!(plan_size(3, 10_000, 1, 1), (1, 1));
    }

    #[test]
    fn plan_size_aspect_error_within_one_pixel() {
        let source = (4000f64, 3000f64);
        for max_w in [101, 333, 600, 777, 1024, 1999] {
            let (w, h) = plan_size(4000, 3000, max_w, 0);
            let expected_h = w as f64 * source.1 / source.0;
            assert!(
                (h as f64 - expected_h).abs() <= 1.0,
                "max_w={max_w} -> {w}x{h}, expected h≈{expected_h:.2}"
            );
        }
    }

    #[test]
    fn plan_size_rounds_half_away_from_zero() {
        // 3 * (5/3) = 5 exactly; 1 * 0.5 = 0.5 must round up to 1.
        assert_eq!(plan_size(1, 1, 0, 0), (1, 1));
        assert_eq!(plan_size(2, 2, 1, 1), (1, 1));
        assert_eq!(plan_size(5, 5, 3, 3), (3, 3));
    }

    // --- resampler -------------------------------------------------------

    #[test]
    fn box_resize_identity_is_byte_copy() {
        let px = noise_rgb(7, 5, 11);
        assert_eq!(box_resize(&px, 7, 5, 7, 5), px);
    }

    #[test]
    fn box_resize_averages_2x2_exactly() {
        // 2x2, grey levels 0, 255, 128, 64 -> mean 111.75 -> 112 (round-half-up)
        let px = vec![
            0u8, 0, 0, 255, 255, 255, //
            128, 128, 128, 64, 64, 64,
        ];
        let out = box_resize(&px, 2, 2, 1, 1);
        assert_eq!(out, vec![112, 112, 112]);
    }

    #[test]
    fn box_resize_gradient_stays_monotonic() {
        let px = gradient_rgb(64, 64);
        let out = box_resize(&px, 64, 64, 16, 16);
        for y in 0..16u32 {
            for x in 1..16u32 {
                let prev = out[((y * 16 + x - 1) * 3) as usize] as i32;
                let cur = out[((y * 16 + x) * 3) as usize] as i32;
                assert!(cur >= prev, "red must not decrease along x");
            }
        }
        // Red is x*255/64, so a 4x4 block centred on x=33 averages to 33*255/64 ≈ 131.
        let mid = out[((8 * 16 + 8) * 3) as usize];
        assert!((mid as i32 - 131).abs() <= 2, "centre red {mid}");
    }

    #[test]
    fn box_resize_down_to_one_pixel_is_the_mean() {
        let px = vec![10u8, 20, 30, 40, 50, 60];
        let out = box_resize(&px, 2, 1, 1, 1);
        assert_eq!(out, vec![25, 35, 45]);
    }

    #[test]
    fn box_resize_rejects_upward_dimensions_in_debug() {
        // Never called with dw > sw by the pipeline; the guard is a debug assert.
        let px = noise_rgb(4, 4, 1);
        let out = box_resize(&px, 4, 4, 4, 4);
        assert_eq!(out.len(), 4 * 4 * 3);
    }

    #[test]
    fn box_resize_rounds_half_up() {
        // mean = 1.5 -> 2
        let px = vec![1u8, 1, 1, 2, 2, 2];
        assert_eq!(box_resize(&px, 2, 1, 1, 1), vec![2, 2, 2]);
    }

    // --- memory ----------------------------------------------------------

    #[test]
    fn jpeg_sink_keeps_one_buffer_for_every_probe() {
        let px = noise_rgb(600, 400, 3);
        let mut sink = JpegSink::new();
        let mut capacity_after_first = 0usize;
        let mut sizes = Vec::new();

        // Descending quality, like the real ladder: the first probe is the largest.
        for (index, q) in (30..=MAX_QUALITY).rev().step_by(5).enumerate() {
            sizes.push(sink.encode(&px, 600, 400, q, 96).unwrap());
            if index == 0 {
                capacity_after_first = sink.buf.capacity();
                assert!(
                    capacity_after_first >= sizes[0],
                    "buffer must hold the largest output"
                );
            }
        }

        assert!(
            sizes.len() >= 13,
            "expected the whole ladder, got {}",
            sizes.len()
        );
        assert!(
            sizes[0] > *sizes.last().unwrap(),
            "q95 must be larger than q30"
        );
        assert_eq!(
            sink.buf.capacity(),
            capacity_after_first,
            "capacity must not grow once the largest output has been written"
        );
        assert_eq!(
            sink.allocations, 1,
            "14 probes must allocate one output buffer, not one per probe"
        );
    }

    #[test]
    fn jpeg_sink_encodes_identically_on_reuse() {
        let px = noise_rgb(320, 240, 12);
        let mut sink = JpegSink::new();
        sink.encode(&px, 320, 240, 60, 96).unwrap();
        let first = sink.bytes().to_vec();
        assert_eq!(sink.encode(&px, 320, 240, 60, 96).unwrap(), first.len());
        assert_eq!(
            sink.bytes(),
            &first[..],
            "a reused buffer must not shift bytes"
        );

        // Same bytes as a virgin sink, and the JFIF density is patched either way.
        let mut fresh = JpegSink::new();
        fresh.encode(&px, 320, 240, 60, 96).unwrap();
        assert_eq!(fresh.bytes(), &first[..]);
        assert_eq!(crate::exif::jfif_density(&first), Some((96, 96)));
    }

    #[test]
    fn jpeg_sink_hands_its_buffer_out_once() {
        let px = noise_rgb(64, 48, 4);
        let mut sink = JpegSink::new();
        sink.encode(&px, 64, 48, 70, 96).unwrap();
        let out = sink.take();
        assert_eq!(&out[..2], &[0xFF, 0xD8]);
        assert!(sink.bytes().is_empty(), "take must leave the sink empty");
    }

    #[test]
    fn jpeg_sink_releases_its_buffer_on_demand() {
        let px = noise_rgb(64, 48, 4);
        let mut sink = JpegSink::new();
        sink.encode(&px, 64, 48, 90, 96).unwrap();
        assert!(!sink.bytes().is_empty());
        sink.release();
        assert!(sink.bytes().is_empty());
        assert_eq!(sink.buf.capacity(), 0, "a released sink holds no memory");
        // Still usable afterwards: the next encode reallocates as needed.
        assert!(sink.encode(&px, 64, 48, 90, 96).unwrap() > 0);
    }

    #[test]
    fn pixel_cap_is_enforced_from_the_header_before_decoding() {
        let px = noise_rgb(64, 48, 4);
        let small = encode_jpeg(&px, 64, 48, 90);
        // A few KB on disk that claims 30000×30000 = 900 MP. Decoding it would mean
        // a 2.7 GB buffer; the cap must reject it from the header instead.
        let huge = with_dimensions(&small, 30_000, 30_000);
        assert_eq!(header_dimensions(&huge), Some((30_000, 30_000)));

        let out = fit(&huge, "huge.jpg", &Settings::default());
        assert!(!out.ok);
        assert_eq!(out.err_code, E_TOO_MANY_PIXELS);
        assert_eq!(out.err_detail, "Too large: 900 MP. Limit is 80 MP.");
        assert!(out.jpeg.is_empty());
    }

    #[test]
    fn pixel_cap_is_a_real_bound_on_the_largest_allocation() {
        // The documented ceiling: RGB24 at the cap is the biggest single buffer.
        assert_eq!(MAX_PIXELS * 3, 240_000_000);
        assert_eq!(crate::MAX_PIXELS, MAX_PIXELS);
    }

    #[test]
    fn header_dimensions_agree_with_the_decoded_image() {
        let px = noise_rgb(37, 23, 6);
        let jpeg = encode_jpeg(&px, 37, 23, 85);
        let mut rgba = Vec::with_capacity(px.len() / 3 * 4);
        for pixel in px.chunks_exact(3) {
            rgba.extend_from_slice(pixel);
            rgba.push(255);
        }
        let png = encode_png_rgba(&rgba, 37, 23);
        let bmp = encode_bmp(&px, 37, 23);
        for file in [jpeg, png, bmp] {
            assert_eq!(header_dimensions(&file), Some((37, 23)));
        }
        assert_eq!(header_dimensions(b"not an image"), None);
    }

    // --- end-to-end ------------------------------------------------------

    #[test]
    fn fit_4000x3000_noise_to_100kb_succeeds() {
        let px = noise_rgb(4000, 3000, 7);
        let jpeg = encode_jpeg(&px, 4000, 3000, 92);
        let out = fit(
            &jpeg,
            "IMG_2031.JPG",
            &Settings {
                target_kb: 100,
                max_width: 0,
                max_height: 0,
            },
        );

        assert!(
            out.ok,
            "expected success, got {} {}",
            out.err_code, out.err_detail
        );
        assert!(out.jpeg.len() <= 100_000, "{} bytes", out.jpeg.len());
        assert!(
            (MIN_QUALITY..=MAX_QUALITY).contains(&out.quality),
            "q={}",
            out.quality
        );
        assert!(out.probes <= 200);
        assert!(out.width <= 4000 && out.height <= 3000);

        let (_, w, h) = decode_jpeg(&out.jpeg);
        assert_eq!((w, h), (out.width, out.height));
        let expected_h = out.width as f64 * 3000.0 / 4000.0;
        assert!((out.height as f64 - expected_h).abs() <= 1.0);
    }

    #[test]
    fn fit_small_image_already_under_target_uses_q95_and_keeps_size() {
        let px = noise_rgb(300, 200, 3);
        let jpeg = encode_jpeg(&px, 300, 200, 40);
        let out = fit(
            &jpeg,
            "small.jpg",
            &Settings {
                target_kb: 500,
                max_width: 0,
                max_height: 0,
            },
        );
        assert!(out.ok);
        assert_eq!(out.quality, MAX_QUALITY);
        assert_eq!((out.width, out.height), (300, 200));
        assert!(out.jpeg.len() <= 500_000);
    }

    #[test]
    fn fit_is_byte_identical_across_runs() {
        let px = noise_rgb(640, 480, 99);
        let jpeg = encode_jpeg(&px, 640, 480, 90);
        let s = Settings {
            target_kb: 40,
            max_width: 0,
            max_height: 0,
        };
        let a = fit(&jpeg, "a.jpg", &s);
        let b = fit(&jpeg, "a.jpg", &s);
        assert!(a.ok && b.ok);
        assert_eq!(a.jpeg, b.jpeg);
        assert_eq!(a.quality, b.quality);
        assert_eq!((a.width, a.height), (b.width, b.height));
    }

    #[test]
    fn fit_respects_max_dimensions() {
        let px = noise_rgb(1200, 900, 5);
        let jpeg = encode_jpeg(&px, 1200, 900, 90);
        let out = fit(
            &jpeg,
            "big.jpg",
            &Settings {
                target_kb: 50,
                max_width: 600,
                max_height: 600,
            },
        );
        assert!(out.ok);
        assert!(
            out.width <= 600 && out.height <= 600,
            "{}x{}",
            out.width,
            out.height
        );
        let expected_h = out.width as f64 * 900.0 / 1200.0;
        assert!((out.height as f64 - expected_h).abs() <= 1.0);
    }

    #[test]
    fn fit_unreachable_target_reports_truth_and_emits_nothing() {
        let px = flat_noise_rgb(900, 600, 13);
        let jpeg = encode_jpeg(&px, 900, 600, 90);
        let out = fit(
            &jpeg,
            "noise.jpg",
            &Settings {
                target_kb: 3,
                max_width: 0,
                max_height: 0,
            },
        );
        assert!(!out.ok);
        assert_eq!(out.err_code, E_TOO_SMALL);
        assert!(out.jpeg.is_empty(), "no output may be produced");
        assert!(
            out.err_detail.starts_with("Cannot reach 3 KB"),
            "{}",
            out.err_detail
        );
        assert!(out.err_detail.contains('×'), "{}", out.err_detail);
        assert!(out.best_bytes > 3000);
        assert!(out.width >= 64 && out.height >= 64);
    }

    #[test]
    fn fit_downscale_loop_stops_at_64_pixels() {
        // 200x200 with an impossible target: every probed dimension must be >= 64.
        let px = flat_noise_rgb(200, 200, 17);
        let jpeg = encode_jpeg(&px, 200, 200, 90);
        let out = fit(
            &jpeg,
            "tiny-target.jpg",
            &Settings {
                target_kb: 1,
                max_width: 0,
                max_height: 0,
            },
        );
        assert!(!out.ok);
        assert_eq!(out.err_code, E_TOO_SMALL);
        assert!(out.width >= 64 && out.height >= 64);
    }

    #[test]
    fn fit_probe_cap_reports_e_budget() {
        let px = noise_rgb(320, 240, 21);
        let jpeg = encode_jpeg(&px, 320, 240, 90);
        let out = run(
            &jpeg,
            "budget.jpg",
            &Settings {
                target_kb: 1,
                max_width: 0,
                max_height: 0,
            },
            &NEVER_CANCEL,
            3,
        );
        assert!(!out.ok);
        assert_eq!(out.err_code, E_BUDGET);
        assert_eq!(out.probes, 3);
        assert!(out.jpeg.is_empty());
    }

    #[test]
    fn too_big_messages_use_one_decimal() {
        assert_eq!(
            msg_too_big_pixels(210_000_000),
            "Too large: 210 MP. Limit is 80 MP."
        );
        assert_eq!(
            msg_too_big_pixels(80_500_000),
            "Too large: 80.5 MP. Limit is 80 MP."
        );
        assert_eq!(
            msg_too_big_bytes(210_000_000),
            "Too large: 210 MB. Limit is 200 MB."
        );
        assert_eq!(
            msg_too_small(5_000, 41_200, 96, 72),
            "Cannot reach 5 KB. The smallest this photo can be is 41.2 KB at 96×72."
        );
        assert_eq!(
            msg_name("IMG_1"),
            "Too many files named IMG_1_fit — rename the source files."
        );
    }

    #[test]
    fn output_jfif_density_defaults_to_96() {
        let px = noise_rgb(120, 90, 31);
        let jpeg = encode_jpeg(&px, 120, 90, 90);
        let out = fit(
            &jpeg,
            "dpi.jpg",
            &Settings {
                target_kb: 40,
                max_width: 0,
                max_height: 0,
            },
        );
        assert!(out.ok);
        assert_eq!(crate::exif::jfif_density(&out.jpeg), Some((96, 96)));
    }

    #[test]
    fn fit_empty_file_is_e_empty() {
        let out = fit(&[], "empty.jpg", &Settings::default());
        assert_eq!(out.err_code, crate::E_EMPTY);
        assert_eq!(out.err_detail, "Empty file.");
    }

    #[test]
    fn fit_unsupported_extension_is_e_format() {
        let px = noise_rgb(32, 32, 1);
        let jpeg = encode_jpeg(&px, 32, 32, 90);
        for name in [
            "photo.heic",
            "photo.tiff",
            "photo.webp",
            "photo",
            "photo.x.jpg.txt",
        ] {
            let out = fit(&jpeg, name, &Settings::default());
            assert_eq!(out.err_code, crate::E_FORMAT, "{name}");
            assert!(out.err_detail.starts_with("Only JPG, PNG and BMP"));
        }
    }

    #[test]
    fn fit_accepts_uppercase_and_mixed_case_extensions() {
        let px = noise_rgb(48, 48, 2);
        let jpeg = encode_jpeg(&px, 48, 48, 90);
        for name in ["A.JPG", "A.Jpeg", "A.PNG", "A.BMP"] {
            let out = fit(
                &jpeg,
                name,
                &Settings {
                    target_kb: 200,
                    max_width: 0,
                    max_height: 0,
                },
            );
            assert!(out.ok, "{name}: {} {}", out.err_code, out.err_detail);
        }
    }

    #[test]
    fn fit_corrupt_bytes_is_e_decode() {
        let out = fit(b"not an image at all", "broken.jpg", &Settings::default());
        assert_eq!(out.err_code, E_DECODE);
        assert!(out.err_detail.starts_with("Cannot read this image"));
    }

    #[test]
    fn fit_header_without_payload_is_e_decode() {
        let mut junk = vec![0xFFu8, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        junk.extend_from_slice(&[0x42; 512]);
        let out = fit(&junk, "cut.jpg", &Settings::default());
        assert_eq!(out.err_code, E_DECODE);
    }

    #[test]
    fn fit_png_alpha_flattens_to_near_white() {
        // Left half fully transparent, right half near-black.
        let (w, h) = (64u32, 48u32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        for y in 0..h as usize {
            for x in 0..w as usize {
                let i = (y * w as usize + x) * 4;
                let transparent = x < (w as usize) / 2;
                px[i] = 10;
                px[i + 1] = 20;
                px[i + 2] = 30;
                px[i + 3] = if transparent { 0 } else { 255 };
            }
        }
        let png = encode_png_rgba(&px, w, h);
        let out = fit(
            &png,
            "alpha.png",
            &Settings {
                target_kb: 300,
                max_width: 0,
                max_height: 0,
            },
        );
        assert!(out.ok, "{} {}", out.err_code, out.err_detail);
        let (rgb, dw, _) = decode_jpeg(&out.jpeg);
        // Sampled well inside the transparent half, away from JPEG block edges.
        for y in 4..(h as usize - 4) {
            for x in 4..(dw as usize / 4) {
                let p = &rgb[(y * dw as usize + x) * 3..][..3];
                for c in p {
                    assert!(
                        *c >= 253,
                        "transparent pixel {x},{y} must decode near white, got {p:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn fit_bmp_input_succeeds() {
        let px = gradient_rgb(120, 80);
        let bmp = encode_bmp(&px, 120, 80);
        let out = fit(
            &bmp,
            "scan.BMP",
            &Settings {
                target_kb: 40,
                max_width: 0,
                max_height: 0,
            },
        );
        assert!(out.ok, "{} {}", out.err_code, out.err_detail);
        let (_, w, h) = decode_jpeg(&out.jpeg);
        assert_eq!((w, h), (120, 80));
    }

    #[test]
    fn fit_one_pixel_image_succeeds() {
        let png = encode_png_rgba(&[12, 34, 56, 255], 1, 1);
        let out = fit(
            &png,
            "dot.png",
            &Settings {
                target_kb: 1,
                max_width: 0,
                max_height: 0,
            },
        );
        assert!(out.ok, "{} {}", out.err_code, out.err_detail);
        assert_eq!((out.width, out.height), (1, 1));
    }

    #[test]
    fn fit_output_carries_no_exif() {
        let px = noise_rgb(300, 200, 8);
        let jpeg = encode_jpeg(&px, 300, 200, 90);
        let with_exif = with_exif_orientation(&jpeg, 6, true);
        let out = fit(
            &with_exif,
            "phone.jpg",
            &Settings {
                target_kb: 60,
                max_width: 0,
                max_height: 0,
            },
        );
        assert!(out.ok, "{} {}", out.err_code, out.err_detail);
        assert!(
            !out.jpeg.windows(4).any(|w| w == b"Exif"),
            "EXIF must be stripped"
        );
        assert!(
            !out.jpeg.windows(5).any(|w| w == b"Exif\0"),
            "EXIF must be stripped"
        );
        // Dimensions swapped: the rotation is baked in.
        assert_eq!((out.width, out.height), (200, 300));
    }

    #[test]
    fn fit_cancelled_before_start_reports_e_cancelled() {
        let px = noise_rgb(64, 64, 9);
        let jpeg = encode_jpeg(&px, 64, 64, 90);
        let cancel = AtomicBool::new(true);
        let out = fit_cancellable(&jpeg, "c.jpg", &Settings::default(), &cancel);
        assert_eq!(out.err_code, crate::E_CANCELLED);
        assert!(out.jpeg.is_empty());
    }

    #[test]
    fn fit_file_reads_from_disk() {
        let dir = temp_dir("fitfile");
        let path = dir.join("photo.jpg");
        let px = noise_rgb(160, 120, 10);
        std::fs::write(&path, encode_jpeg(&px, 160, 120, 90)).unwrap();
        let out = crate::fit_file(
            &path,
            &Settings {
                target_kb: 30,
                max_width: 0,
                max_height: 0,
            },
        );
        assert!(out.ok, "{} {}", out.err_code, out.err_detail);
        let missing = crate::fit_file(&dir.join("gone.jpg"), &Settings::default());
        assert_eq!(missing.err_code, E_ACCESS);
        assert_eq!(missing.err_detail, "File is gone.");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
