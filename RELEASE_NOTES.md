# Kilobyt v1.1.3 — version info in the exe

**Make a photo provably fit an upload limit — offline, nothing uploaded.**

Drop photos, type the limit the form gives you, get a JPEG that is at or under it. Or get an honest
message saying the limit is impossible, instead of a file that will be rejected later.

## What changed in 1.1.3

- The exe now carries a PE `VERSIONINFO` resource, so Task Manager's **Description** column and
  Explorer's Details tab show a product name and version instead of `Kilobyt.ex…`.
- Fields: `CompanyName=nullpointio`, `ProductName=Kilobyt`, `FileDescription=Compress a photo to fit a
  hard upload limit, offline`, `FileVersion`/`ProductVersion=1.1.3.0`, `OriginalFilename=Kilobyt.exe`,
  `LegalCopyright=MIT License — nullpointio`.
- It is compiled into the same resource object as the icon and manifest, so there is no second file to
  keep in sync. `-version` still prints the crate version, which now matches. No behaviour change.
- **Peak memory is lower and now measurable.** The JPEG output buffer is allocated once per file and
  reused by every quality probe (grown to ~1.25× the bytes needed instead of doubling, and dropped at
  each downscale boundary); the source file's bytes are freed as soon as the pixels are decoded; alpha
  is flattened in place. A 12 MP photo that needs 126 probes to reach 100 KB drops from 96.2 MiB to
  78.9 MiB peak RSS (4:4:4 source) and from 87.7 MiB to 71.1 MiB (4:2:0 source). Measure it yourself
  with `kilobyt -mem path\to\photo.jpg`.
- **The 80 MPix cap now fires before decoding.** Dimensions come from the container header, so an
  oversized image is refused with `E_TOO_MANY_PIXELS` instead of allocating its RGB24 buffer first
  (240 MB at the cap).

## What changed in 1.1.0

**The exe went from 68 MB to 583 KB.** v1.0.0 shipped a self-contained .NET runtime; this is a native
Rust binary with no runtime, no installer, no DLLs beside it. Same algorithm, same guarantees, one file.

- Rewritten in Rust: the compression engine is a plain library, the GUI is a thin Win32 shell.
- `image` crate does JPEG/PNG/BMP natively — no GDI+, no COM, no .NET.
- Common Controls v6 manifest, so it looks like a normal Windows app rather than Windows 95.
- New `-selftest` mode: CI runs the actual compiled exe on a real Windows runner before any release.
- Fixed-size window, so the layout cannot break at odd screen sizes.

## Downloads

- `Kilobyt.exe` — 583 KB, Windows 10/11 x64. No runtime to install.
- `Kilobyt-v1.1.1-win-x64.zip` — the same exe in a zip, for people who prefer that.
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
