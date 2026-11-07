# PRD-001 · Kilobyt v1 — photo files that provably fit an upload limit

| Field | Value |
|---|---|
| PRD ID | PRD-001 |
| Status | **Approved** (v1.0, 2026-11-07) — hand off to Eng |
| Author | CPO (PM-1 unfilled) |
| Approver | CPO · CTO feasibility **ASSUMPTION, not yet given** (see §12 Q1) |
| Version | 1.0 |
| Target release | v1.0.0, one GitHub Release, unsigned exe |
| Launch tier | T3 (repo + Release notes; no announcement — public launch is Founder-A) |
| Scorecard PS | 14.2 (C-3), band "Next"; rank 1 by 2.6× — see `decision.md` §1 |
| Evidence tier | E2 (qualitative; `research.md` §2) |

**Architecture constraint (binding on Eng, not a suggestion):** all logic that can be tested without a display lives in `Kilobyt.Core` (`net8.0`, no native dependencies, no `System.Drawing`). `Kilobyt.App` (`net8.0-windows`, WinForms) is a thin shell that supplies an `IJpegCodec` implementation and draws the window. If a rule in §5 cannot be unit-tested with a fake codec on Linux, the rule is specified wrong — come back before building it.

---

## 1. Problem

A person filling a government, visa, exam or HR form cannot submit their photo because the portal enforces a hard byte limit (often with a pixel limit), the free tools that fix it require uploading an identity photo to an unknown server, and the desktop tools that do not require understanding JPEG quality sliders and guessing numbers — so they retry blindly at the last step of a form they have already completed, sometimes past a deadline.

**Falsifiable:** if ≥3 users report that a Kilobyt output was rejected by the portal for *size* while the app reported success, the byte-limit model in §5.2 is wrong and this PRD is wrong.

---

## 2. Who it's for

| Segment | Why they care | v1 served? |
|---|---|---|
| Applicant on a gov/visa/exam/ID form (primary) | One photo, stated limit, deadline, no image skills | yes |
| HR / school / clinic admin (secondary) | 20–200 photos, same limit, currently one at a time | yes (batch) |
| Privacy-motivated professional | Will not upload client images anywhere | yes (offline, metadata stripped) |
| Photographers / designers | Colour management, exact crop, print | **no** |

---

## 3. Goals

| # | Metric | Baseline | Target | Measured |
|---|---|---|---|---|
| M1 | Outputs whose byte length exceeds the requested limit | — | **0** (hard invariant, not a rate) | every run, in-app |
| M2 | Successful completions per download | unknown (offline, no telemetry — **ASSUMPTION**) | ≥40% proxy at day 30 | download counts + issue reports |
| M3 | "Portal rejected the file" reports | 0 | ≤2 in 30 days | issue tracker |

---

## 4. Scope

**In scope:** single window; drag-drop + file picker; JPEG/PNG/BMP in; JPEG out; target KB; optional max width/height; batch with per-file status; cancel; output naming; metadata stripping.

**Out of scope (v1) — do not build, do not "just add":**
1. Cropping, or exact W×H output (fit-within only, aspect always preserved).
2. PNG/WebP output, transparency preservation (alpha is flattened to white).
3. HEIC, TIFF, GIF, RAW, PDF input.
4. OCR, PDF assembly, EXIF scrubbing as a standalone feature, renaming by date.
5. Any network call: no update check, no telemetry, no crash reporting, no licence check, no "share".
6. Installer, MSIX, code signing, auto-update, shell/Explorer context-menu integration.
7. Presets for specific portals (we cannot verify any portal's rules).
8. Command-line mode, settings persistence across runs, themes, localisation, dark mode.
9. Multi-threading the batch (sequential is deterministic and keeps memory bounded).

---

## 5. Functional specification — the core algorithm

This section is the contract. Numbers are exact; do not "improve" them without a PRD version bump.

### 5.1 Units and definitions

- `targetBytes = TargetKb × 1000`. **1 KB = 1000 bytes in v1.** Rationale: a portal that means 1024 bytes/KB still accepts the smaller file, and a portal that means 1000 is satisfied exactly. Documented in the UI tooltip on the KB field. Not configurable in v1.
- `q` = JPEG quality, integer, **30 ≤ q ≤ 95**. q=100 and q<30 are never encoded.
- Input limits: file ≤ 200 MB; decoded pixels ≤ 80,000,000; either dimension ≥ 1. Violations are per-file errors, never crashes.

### 5.2 Pipeline, per file (exact order)

1. **Read** the whole file into memory. Zero-length → error `E_EMPTY`.
2. **Decode** to `Rgb24Image` (interleaved RGB, 3 bytes/px, row-major, no padding) plus `ImageMeta { DpiX, DpiY, Orientation }`. Alpha is composited over **white** during decode. Unsupported/corrupt → `E_DECODE`.
3. **Orient**: apply EXIF orientation 1–8 to the pixels so the stored image is upright; the output carries **no** orientation tag.
4. **Plan dimensions** (`SizePlanner.Fit`): `scale = 1`; for each provided limit `scale = min(scale, limit / dimension)`; `scale = min(scale, 1.0)` — **never upscale**. `newW = clamp(round(W·scale), 1, W)`, `newH = clamp(round(H·scale), 1, H)`. Aspect error must stay ≤ 1 px versus the source ratio.
5. **Resample** only if `newW != W || newH != H`: area-average (box) filter over the source rectangle each destination pixel covers, integer accumulation, round-half-up. No bicubic, no sharpening.
6. **Quality search** on the current buffer (`QualitySearch.Find`):
   - Coarse ladder, descending: `q = 95, 90, 85, 80, 75, 70, 65, 60, 55, 50, 45, 40, 35, 30`. Probe = encode at q, take `byte[].Length`.
   - First `q_hi` whose size ≤ `targetBytes` becomes the candidate. Then **bisect integers** in `(q_hi, q_hi + 5)` for the highest passing q: at most 4 further probes. Return the highest passing q found.
   - If **no** ladder q passes, go to step 7.
7. **Downscale loop** (only reached when q=30 still exceeds target): `newW' = max(64, floor(newW × 0.85))`; recompute `newH'` from the **original** aspect ratio (`round(newW' × H/W)`), not from the previous step, to avoid ratio drift. Re-run step 6 from the top of the ladder. Stop when either dimension would fall below 64, or after **8** downscale iterations.
8. **Unreachable target** → return failure with the best achieved byte count, the dimensions and the q used. **No output file is written.** (A file that misses the limit is worse than no file: the user believes they are done.)
9. **DPI**: preserve source `DpiX/DpiY` when each is within `[1, 1200]` and non-zero; otherwise use **96**. Written as JFIF density (units = inches).
10. **Encode final** at the chosen q and that DPI.
11. **Verify**: write to `{final}.part` in the destination directory, `Flush`, re-read the file length from disk. If length > `targetBytes` (encoder did not reproduce the probe size), decrement q by 1 and retry, max 3 attempts; after that → `E_UNSTABLE`, delete the `.part`.
12. **Commit**: `File.Move(part, final)` (never overwrite an existing file — the name is pre-resolved by `OutputNamer`). Delete `.part` on any exception or cancellation.
13. **Strip**: the output contains no EXIF, no XMP, no GPS, no IPTC, no comment segment. Only JFIF APP0 (density) and the frame tables. No ICC profile is written; sRGB is assumed.

### 5.3 Termination and budget

- Hard cap **200 encode probes per file** (ladder + bisect + downscale iterations). Exceeding it is a spec violation, not a user error → `E_BUDGET`.
- Typical probe count: 1–5 for small files, up to ~40 for a 12 MP photo at a very small target.
- The search is **greedy-maximal**: the chosen q is the highest that fit among probes actually performed. Do not claim global optimality against a non-monotonic encoder; do not add a second search strategy in v1.

### 5.4 Determinism

Same input bytes + same `FitSettings` + same machine/encoder → **byte-identical output** (verified by SHA-256 in tests). No timestamps, no random names, no culture-dependent formatting in file names.

### 5.5 Batch behaviour

- Files are processed **sequentially**, in list order.
- The list is a snapshot taken when "Compress" is pressed; files added or dragged during a run are appended to the list as *Queued* and are **not** part of the running batch (v1 simplicity; the UI says "added — press Compress again").
- A per-file failure never aborts the batch. Status becomes `Failed: <message>`; the run continues.
- Duplicate paths are de-duplicated on add (case-insensitive, full path).
- Output directory: same as the source file by default, or one chosen folder (flat, not mirrored). If two sources in different folders share a stem, `OutputNamer` resolves the collision inside the destination.

### 5.6 Output naming (`OutputNamer.NextFree`)

- `{stem}_fit.jpg`, where `stem` is the source filename without extension, sanitised only by trimming trailing dots/spaces (Windows rule).
- If that path exists on disk **or** is already reserved by this batch: `{stem}_fit-2.jpg`, then `-3`, … first free integer, max 999 then `E_NAME`.
- Never overwrite. Never write into a read-only directory (`E_ACCESS`).
- Example: `IMG_2031.JPG` + target 100 → `IMG_2031_fit.jpg`.

### 5.7 Cancellation

- `CancellationToken` is checked before each encode probe and before each downscale step, and between files.
- On cancel: current `.part` is deleted, sources are untouched, completed outputs are kept (they are valid files), remaining items return to `Cancelled` status, the summary reads `Stopped — 4 of 14 done`.
- Cancellation must complete within 2 s of the click for inputs ≤ 12 MP.

### 5.8 Library surface (minimum, Eng may add)

```csharp
namespace Kilobyt.Core;

public readonly record struct Rgb24Image(int Width, int Height, byte[] Pixels);
public sealed record ImageMeta(int DpiX, int DpiY, int Orientation);

public interface IJpegCodec {                      // implemented in Kilobyt.App (System.Drawing)
    Rgb24Image Decode(byte[] file, out ImageMeta meta);   // throws UnsupportedImageException
    byte[] Encode(Rgb24Image image, int quality, int dpi); // quality 30..95
}

public sealed record FitSettings(int TargetKb, int? MaxWidth, int? MaxHeight);
public sealed record FitResult(bool Success, byte[]? Jpeg, int Quality, int Width, int Height,
                               int Probes, long BestBytes, string? ErrorCode, string? ErrorDetail);

public static class Compressor { public static FitResult Fit(byte[] input, IJpegCodec codec,
                                   FitSettings settings, CancellationToken ct); }
public static class SizePlanner { public static (int W, int H) Fit(int w, int h, int? maxW, int? maxH); }
public static class Resampler   { public static Rgb24Image Downscale(Rgb24Image src, int w, int h); }
public static class QualitySearch { public static int? Find(Func<int, long> probe, long targetBytes, CancellationToken ct); }
public static class OutputNamer { public static string NextFree(string dir, string stem, string ext, ISet<string> reserved); }
```

`Compressor`, `SizePlanner`, `Resampler`, `QualitySearch`, `OutputNamer` must be testable with no file system and no real codec (fake `IJpegCodec` returning `Pixels.Length / k` bytes).

### 5.9 Requirement index

M = release blocked without it · S = should · C = cut first.

| ID | Requirement | Pri | Acceptance criteria |
|---|---|---|---|
| R-01 | Add files by drag-drop and by multi-select picker; de-duplicate by full path (case-insensitive) | M | US-1, US-3.3 |
| R-02 | Target size in KB with 1 KB = 1000 bytes, range 1–99999 | M | AC-1.1 |
| R-03 | Optional max width/height, fit-within, never upscale, aspect within 1 px | M | AC-2.1–2.3 |
| R-04 | Quality ladder 95→30 step 5 + integer bisect, q ∈ [30,95], ≤200 probes | M | AC-1.2, §5.2.6 |
| R-05 | Downscale loop ≤8 iterations, floor 64 px, explicit `E_TOO_SMALL`, no file written | M | AC-6.1–6.2 |
| R-06 | Re-verify output length on disk; retry q−1 ×3; then `E_UNSTABLE` | M | §5.2.11 |
| R-07 | Bake EXIF orientation, strip all metadata, DPI preserved in [1,1200] else 96 | M | AC-4.2–4.3 |
| R-08 | Output `{stem}_fit.jpg`, `-2`…`-999` collision suffix, never overwrite | M | AC-3.3 |
| R-09 | Batch continues past failures; per-row status; summary `N of M done` | M | AC-3.1–3.2 |
| R-10 | Cancel with no `.part` left behind, sources untouched | M | AC-5.1–5.2 |
| R-11 | PNG and BMP input (alpha flattened to white), JPEG output only | M | AC-7.1 |
| R-12 | Byte-identical output for identical input + settings | M | AC-8.1 |
| R-13 | Responsive UI during a run, controls disabled, determinate progress | M | §8, AC-3.1 |
| R-14 | Zero network capability in the shipped binary | M | AC-4.1 |
| R-15 | Message per error code, per-file, non-blocking | M | AC-3.1, §7 |
| R-16 | Per-monitor DPI aware, every control labelled and tab-reachable | S | §8 |

**Additional acceptance criteria referenced above**

- **AC-7.1** Given a 1200×800 PNG with an alpha channel, when compressed, then the output is a decodable JPEG, and a pixel that was fully transparent in the source decodes to within 2/255 of white.
- **AC-8.1** Given the same fixture and settings, run `Compressor.Fit` twice in one process, then `SHA256(a.Jpeg) == SHA256(b.Jpeg)`; run it again in a fresh process on the CI image, the hash is unchanged.

---

## 6. User stories and acceptance criteria

Checkable by someone who did not write the code. "Fixture" = `TestImages.Noise(w, h, seed)` generated in test code (deterministic PRNG), so results do not depend on a checked-in binary.

**US-1 — one photo, one limit.** As an applicant, I drop one photo and set 100 KB, and get a file I can upload.
- **AC-1.1** Given a 4000×3000 JPEG fixture, when compressed with target 100 KB and no dimension limits, then a file exists at `{stem}_fit.jpg`, its length on disk is ≤ 100,000 bytes, it decodes, and its dimensions are ≤ source and preserve the source aspect ratio within 1 px.
- **AC-1.2** Given the same run, then `FitResult.Quality ∈ [30,95]` and `Probes ≤ 200`.
- **AC-1.3** Given a fixture already smaller than the target (e.g. a 300×200 image at q40 ≈ 8 KB) with target 500 KB, then output ≤ 500,000 bytes and `Quality = 95` (the app never exceeds q95 and does not upscale: output dimensions equal source dimensions).

**US-2 — two limits at once.** As an applicant on a form that says "600×600 px, max 50 KB", I set both and get something the form accepts.
- **AC-2.1** Given 4000×3000 and max 600×600, target 50 KB, then output width ≤ 600 and height ≤ 600, and aspect is preserved within 1 px.
- **AC-2.2** Given 400×300 and max 600×600, then output stays 400×300 (no upscaling) — `Fit(400,300,600,600) == (400,300)`.
- **AC-2.3** `SizePlanner.Fit(4000,3000,600,null) == (600,450)`; `Fit(4000,3000,null,null) == (4000,3000)`; `Fit(4000,3000,10000,10000) == (4000,3000)`.

**US-3 — batch that partly fails.** As an HR admin, I drop 40 photos; the three that cannot reach the limit are named, the other 37 are done.
- **AC-3.1** Given 40 files of which 3 are unreadable/corrupt, when the batch runs, then 37 outputs exist, 3 rows read `Failed: …` with a specific reason, no dialog blocks the run, and the summary reads `37 of 40 done · 3 failed`.
- **AC-3.2** Given 14 acceptable files, each output is ≤ the target and none of the sources is modified (source SHA-256 unchanged).
- **AC-3.3** Given a batch whose files include `a.jpg` in two different folders writing to one output folder, then the second becomes `a_fit-2.jpg` and both files exist.

**US-4 — nothing leaves, nothing sticks.** As a privacy-motivated user, I know the app is offline and the output carries no metadata.
- **AC-4.1** The solution contains no `System.Net.Http`, no `HttpClient`, no socket usage outside test code; the exe runs with the network adapter disabled (manual check, recorded in the release checklist).
- **AC-4.2** Given an input JPEG with EXIF GPS, camera make, serial, and a DateTimeOriginal, then the output has no APP1/EXIF segment, and the byte sequence of the source GPS coordinates does not appear in the output.
- **AC-4.3** Given a portrait photo with EXIF orientation = 6, then the output's pixel dimensions are swapped (width < height after rotation applied) — i.e. the rotation is baked in, not tagged.

**US-5 — stop a run.** As any user, I can stop a long batch and nothing is left half-written.
- **AC-5.1** Given a 40-file batch, when Cancel is pressed after the 3rd file completes, then no `.part` file remains in any output folder, sources are unchanged, and status reads `Stopped — 3 of 40 done`.
- **AC-5.2** Cancellation is observed by `Compressor.Fit` within one encode probe (unit test: a fake codec that cancels after the 2nd probe returns without throwing `OperationCanceledException` past the boundary; `Compressor` converts it to a cancelled result).

**US-6 — unreachable target is stated, not faked.** As an applicant with a 20 KB limit on a 12 MP photo, I am told the truth.
- **AC-6.1** Given a 12 MP noise fixture and target 5 KB, then `Success == false`, `ErrorCode == "E_TOO_SMALL"`, **no file is written**, and the message names the best achievable size and the final dimensions.
- **AC-6.2** The loop stops at 64 px minimum dimension or after 8 downscales, whichever is first (unit test asserts the iteration count and that no probe used a dimension < 64).

---

## 7. Errors and edge cases

Every message is shown in the row's Status cell; the full text goes into the end-of-run summary. All are per-file and non-fatal.

| Code | Trigger | Message shown to the user |
|---|---|---|
| `E_EMPTY` | 0-byte file | `Empty file.` |
| `E_DECODE` | corrupt/truncated/unsupported codec data | `Cannot read this image — the file may be damaged or use an unsupported format.` |
| `E_FORMAT` | extension not .jpg/.jpeg/.png/.bmp | `Only JPG, PNG and BMP are supported. Convert HEIC/TIFF/WebP first.` |
| `E_TOOBIG` | > 200 MB or > 80 MPix | `Too large: 210 MP. Limit is 80 MP.` (actual numbers) |
| `E_TOO_SMALL` | target unreachable at 64 px / q30 | `Cannot reach 5 KB. The smallest this photo can be is 41 KB at 96×72.` |
| `E_ACCESS` | read-only or missing output dir | `Cannot write to C:\Forms — folder is read-only or missing.` |
| `E_NAME` | 999 name collisions | `Too many files named X_fit — rename the source files.` |
| `E_UNSTABLE` | file length still over target after 3 retries | `Encoder produced an oversized file 3 times; skipped.` |
| `E_BUDGET` | > 200 probes | `Internal error (E_BUDGET) — please report this file.` |
| — | output would equal the source path | cannot happen by construction (suffix), but the app asserts it and fails loudly if it ever does |

Other specified cases: Unicode/emoji filenames (written as-is, NFC preserved); paths > 260 chars (app manifest sets `longPathAware`, and the app must not crash if it fails); files on a network share mapped to a drive (works, but cancellation may be slow — no special handling in v1); file deleted between add and run (`E_DECODE`-adjacent message `File is gone.`); 1×1 pixel image with a 1 KB target (succeeds, q30); image with alpha (flattened to white, the row shows no warning — it is documented in the README, not in the UI); CMYK JPEG input (decode to RGB; if the codec fails → `E_DECODE`, acceptable for v1).

---

## 8. UI (words, no mockups)

Single window, title **`Kilobyt`**, ~760×620, min 640×520, resizable, per-monitor DPI aware v2, standard Win11 light theme (system theme followed, no custom chrome).

- **Top:** a drop zone occupying the top ~90 px: large text `Drop photos here`, sub-line `They stay on this PC. Nothing is uploaded.` and a `Add photos…` button (opens multi-select file dialog). The zone highlights when a drag hovers.
- **Middle:** a `ListView` in Details mode, full-width, 4 columns: `File` (name only, tooltip = full path), `Size` (source KB, 1 decimal), `Result` (`—`, `42 KB · 600×450`, or `—`), `Status` (`Queued` / `Working…` / `Done` / `Failed: <message>` / `Cancelled`). Multi-select; `Delete` removes selected rows while idle.
- **Right of the list, a settings panel** (fixed 240 px, top-aligned):
  - `Max size` [ numeric 1–99999 | `KB` ] default **100**; tooltip: `1 KB = 1000 bytes. The app guarantees the file is at or under this.`
  - `Limit dimensions` [ checkbox ] → two numeric fields `W` `H` (16–20000), default 1600×1600, disabled until checked.
  - `Save to` [ radio: `Same folder as original` (default) | `This folder:` + path + `Browse…` ].
- **Bottom bar:** primary button `Compress` (enabled only with ≥1 row and valid settings; also triggered by Enter), secondary `Cancel` (visible only while running, Esc), a determinate progress bar (files completed / total), and a status line: `Ready`, `Compressing 3 of 14…`, `14 of 14 done · all at or under 100 KB`, or `12 of 14 done · 2 failed`.
- **On completion with failures:** one modal listing `file — reason` with a `Copy` button. No modal on total success; instead the status line plus a `Open folder` link-button (enabled when at least one output exists; opens the output folder of the last successful file).
- **Disabled-while-running:** Add, Delete, both settings groups, Browse. The window stays responsive (work on a background task, UI updated via `IProgress<FileProgress>`).
- **Empty state:** list area shows `No photos yet. Drag some in.` Success/failure is never conveyed by colour alone — status text always carries the meaning.
- **Keyboard:** Ctrl+O add · Del remove · Enter compress · Esc cancel · Tab reaches every control in reading order. No custom-painted controls.

---

## 9. Data, privacy, permissions

| Question | Answer |
|---|---|
| Reads | The image files the user selects, nothing else. |
| Writes | New `*_fit.jpg` files in the chosen folder; temp `.part` in the same folder, deleted on exit/exception/cancel. |
| PII | The photos themselves are PII. They are never copied, logged, hashed into a name, or transmitted. |
| Network | None. No DNS, no sockets, no update check. Enforced by review, not by a firewall. |
| Retention | None — the app stores nothing between runs (no settings file in v1). |
| Telemetry | None. |
| GC review | Not triggered (no new data collection, no export of third-party data). If a future version adds any network call, GC review becomes mandatory. |

---

## 10. Dependencies and risks

| # | Risk | Owner | Mitigation |
|---|---|---|---|
| 1 | `System.Drawing.Common` is Windows-only; `Kilobyt.Core` must not reference it | Eng | Interface split in §5.8; CI fails if Core's project file references any native/Windows package. |
| 2 | JPEG encoder non-determinism across Windows versions | Eng | Determinism asserted on the CI image only; the size invariant is re-verified from disk (§5.2.11). |
| 3 | Unsigned exe + SmartScreen | CPO | Documented in release notes with SHA256 (decision.md R1). |
| 4 | Memory on 80 MPix images (≈240 MB buffer + encodes) | Eng | Cap at 80 MPix; sequential batch; no parallel encoding. |
| 5 | Users expecting exact 600×600 | CPO | Result column shows actual pixels; out-of-scope list published in the README. |

---

## 11. Rollout

- Distribution: one GitHub Release, tag `v1.0.0`, assets `Kilobyt.exe` (self-contained, single file, win-x64) + `Kilobyt-v1.0.0.zip` + `SHA256SUMS.txt`.
- No feature flags (offline, no server). Rollback = delete the release / publish `v1.0.1`.
- Pre-release gate: the full unit suite on `ubuntu-latest` + the Windows integration suite + one manual pass on a clean Windows 11 VM (network off) covering US-1, US-3, US-5.

---

## 12. Open questions

| # | Question | Owner | Needed by |
|---|---|---|---|
| 1 | CTO effort estimate vs the 2.0-week assumption (PS crosses 15.0 at ≤1.75 wk) | CTO | Before build |
| 2 | Is `System.Drawing` JPEG quality mapping monotonic enough for the ladder to be worth keeping as-is? (spike: 30 min, one test) | Eng | Before R-04 is closed |
| 3 | Does the founder accept the unsigned-exe distribution risk in writing? | Founder | Before the Release is made public |
