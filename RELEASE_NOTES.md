# Kilobyt v1.0.0

**Make a photo provably fit an upload limit — offline, nothing uploaded.**

Drop photos, type the limit the form gives you, get a JPEG that is at or under it. Or get an honest
message saying the limit is impossible, instead of a file that will be rejected later.

## Downloads

- `Kilobyt.exe` — Windows 10/11 x64, self-contained single file, no installer, no .NET runtime needed.
- `Kilobyt-v1.0.0-win-x64.zip` — the same exe in a zip, for people who prefer that.
- `SHA256SUMS.txt` — verify your download:
  `powershell -c "(Get-FileHash .\Kilobyt.exe -Algorithm SHA256).Hash"`

## ⚠️ This exe is unsigned

Windows SmartScreen will show "Windows protected your PC" on first run. That is expected: code signing
costs money and this build has no certificate. Choose **More info → Run anyway**, or compare the SHA-256
against `SHA256SUMS.txt` first. The full source is in this repository; if you would rather not trust a
binary, build it yourself — the command is in the README.

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

## Not in v1

Cropping / exact `W × H` output, PNG or WebP output, HEIC/TIFF/WebP/RAW input, PDF tools, installer,
auto-update, shell integration, remembered settings. JPEG in (and PNG/BMP in), JPEG out.

## Known limitations

- The window has been compiled but **not yet run on a real Windows machine** in this build environment.
  The algorithm is covered by 116 unit tests on Linux and a Windows integration suite in CI; if the UI
  misbehaves on your machine, please open an issue with a screenshot.
- HEIC photos from iPhones are not supported; export to JPEG first.
- CMYK JPEGs may fail to decode (`E_DECODE`).

## Verification

- 116 unit tests on `ubuntu-latest` (compression search, resampling, naming, cancellation, error codes).
- Windows integration tests on `windows-latest` (real `System.Drawing` encoder, on-disk size invariant,
  EXIF removal, orientation baking, determinism, 40-file batch).
- Build artifacts and hashes produced by the same CI run that publishes this release.

MIT licensed. Full spec: `docs/prd.md`.
