# Kilobyt v1.1.0 — native rewrite

**Make a photo provably fit an upload limit — offline, nothing uploaded.**

Drop photos, type the limit the form gives you, get a JPEG that is at or under it. Or get an honest
message saying the limit is impossible, instead of a file that will be rejected later.

## What changed in 1.1.0

**The exe went from 68 MB to 715 KB.** v1.0.0 shipped a self-contained .NET runtime; this is a native
Rust binary with no runtime, no installer, no DLLs beside it. Same algorithm, same guarantees, one file.

- Rewritten in Rust: the compression engine is a plain library, the GUI is a thin Win32 shell.
- `image` crate does JPEG/PNG/BMP natively — no GDI+, no COM, no .NET.
- Common Controls v6 manifest, so it looks like a normal Windows app rather than Windows 95.
- New `-selftest` mode: CI runs the actual compiled exe on a real Windows runner before any release.
- Fixed-size window, so the layout cannot break at odd screen sizes.

## Downloads

- `Kilobyt.exe` — 715 KB, Windows 10/11 x64. No runtime to install.
- `Kilobyt-v1.1.0-win-x64.zip` — the same exe in a zip, for people who prefer that.
- `SHA256SUMS.txt` — verify your download:
  `powershell -c "(Get-FileHash .\Kilobyt.exe -Algorithm SHA256).Hash"`

## ⚠️ This exe is unsigned

Windows SmartScreen will show "Windows protected your PC" on first run. That is expected: code signing
costs money and this build has no certificate. Choose **More info → Run anyway**, or compare the SHA-256
against `SHA256SUMS.txt` first. The full source is in this repository; if you would rather not trust a
binary, build it yourself — `cargo build --release`, about two minutes.

## What it does

- **Exact byte limit** — ask for 100 KB, get ≤ 100,000 bytes. The written file's length is re-read from
  disk before the app reports success.
- **Optional pixel limit** — fit-within, aspect preserved, never upscales. `600 × 600 max` means that.
- **Batch** — many photos, one limit, per-file status. One bad file never aborts the run.
- **Honest failure** — if a limit can't be reached, no file is written and the smallest achievable size
  is reported.
- **Metadata stripped** — EXIF, GPS, camera serial, timestamps, XMP, IPTC, comments. Rotation is baked
  into the pixels.
- **No network code in the binary** — no update check, no telemetry, no crash reporting.

## Not in 1.1.0

Cropping / exact `W × H` output, PNG or WebP output, HEIC/TIFF/WebP/RAW input, PDF tools, installer,
auto-update, shell integration, remembered settings. JPEG in (and PNG/BMP in), JPEG out.

## Known limitations

- The binary was run end-to-end under Wine (screenshot in the README) and by `-selftest` on
  `windows-latest`, but **not on a physical Windows machine**. If the window misbehaves on yours, please
  open an issue with a screenshot.
- HEIC photos from iPhones are not supported; export to JPEG first.
- Drag-hover highlighting is not implemented (dropping works, the zone just doesn't light up).

## Verification

- 61 tests on Linux: compression search, box resampling, EXIF orientation, naming, error codes.
- The same suite plus `kilobyt.exe -selftest` on `windows-latest` on every push.
- End-to-end run of the shipped binary: 6.18 MB / 4000×3000 in → 98,979 bytes / 1507×1130 out under a
  100 KB target, with no EXIF segment in the result.

MIT licensed. Full spec: `docs/prd.md`.
