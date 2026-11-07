# Decision — v1 direction

| Field | Value |
|---|---|
| Decision ID | D-2026-11-07-01 |
| Owner | CPO (PM-1 unfilled) |
| Decided | 2026-11-07 |
| Scores | `research.md` — C-3 PS **14.2**, next candidate 5.5 |
| Supersedes | — |

## 1. Chosen

**An offline Windows desktop utility that makes a photo meet a hard upload limit — target file size in KB, optional maximum pixel dimensions — without the photo ever leaving the machine.**

Repo slug: **`kilobyt`** (see `name.md`). Shape: `Kilobyt.Core` (net8.0, cross-platform, no native deps, fully unit-testable on the Linux dev host) + `Kilobyt.App` (net8.0-windows WinForms shell) + CI on `windows-latest` publishing an unsigned single-file exe to GitHub Releases.

**Score reading.** C-3 lands at 14.2 = band "Next", 0.8 under the 15.0 line. This is not fudged and it is not a reason to re-weight: the V anchors cap a free, no-revenue tool at Rev 2, so the band is not attainable by construction for any candidate here (`research.md` §0). The decision rests on rank (2.6× the runner-up on the same formula), on the constraint screen (only candidate that is both differentiated and buildable in one pass), and on the fact that at the ≤1.75 eng-week estimate the build actually implies, PS = 16.2 (Build now). If CTO estimates ≥3 weeks, re-score before building — that is the honest tripwire.

## 2. The wedge — one sentence

**The web uploaders make you hand your passport photo to a stranger's server; we produce the same guaranteed-under-the-limit file locally, in one drag, and can prove it has no metadata.**

Three specific alternatives and why someone moves:

| Alternative today | Why they leave it |
|---|---|
| iLoveIMG / resize-image-to-kb style web tools | Uploads an ID photo to an unknown server, sometimes requires an account, sometimes watermarks or silently upscales; no guarantee the result is actually under the limit until you download and check. |
| Photoshop / GIMP / Paint | Requires knowing that "Save for web → quality slider" exists, then guessing the quality number, then re-checking the file size, repeatedly. Most people cannot do this; the ones who can resent doing it 20 times for a batch. |
| Doing nothing / re-shooting the photo | The portal rejects the upload at the last step of a form they have already filled in — often with a deadline. Re-shooting is not possible for a document photo. |

The promise is one line long and testable: **the output is ≤ your limit, or you are told it cannot be done and why.** No "usually", no "approximately".

## 3. Target user

> **Someone who must upload a photo to a bureaucratic portal with a hard byte limit, once or in a batch, and who does not want that photo on a third-party server.**

Concretely, in priority order:
1. **Applicant filling a government/visa/exam/ID form** — has one photo, a stated limit ("600×600 px, max 50 KB"), a deadline, and no image tooling. Non-technical; will not read a README.
2. **HR / school / clinic admin** — 20–200 staff or student photos to submit in one batch, each under the same limit; currently does them one at a time in an online tool. This is the batch user and the reason batch is in v1, not v2.
3. **Privacy-motivated professional** (legal, medical, journalism) — will not upload client images to a website at all; wants offline and metadata-stripped.

**Explicitly not for:** photographers doing colour-managed export, designers needing exact crop/framing, anyone wanting to *increase* quality or upscale, macOS/Linux users (v1 is Windows-only by constraint, not by choice).

## 4. What we deliberately do NOT build (v1)

Each of these is a real request we are declining now, with the reason:

| Not building | Why (and what happens instead) |
|---|---|
| **Exact W×H output (e.g. force 600×600)** | Requires cropping, i.e. a framing decision the app cannot make for the user. v1 fits *within* a box and never distorts. The result panel shows the resulting pixels so the user sees `600×450` and knows. v2 candidate, behind a crop UI. |
| **PNG output / transparency preservation** | A target byte size on PNG is not solvable by quality search (no quality dial); it is a different algorithm (palette quantisation). Flatten to white on JPEG only, documented in the README. |
| **PDF, TIFF, HEIC, WebP, GIF input** | HEIC needs a Windows codec pack we cannot ship; GIF/TIFF/WebP add decoders and edge cases for a small share of the need. Reject with a clear message naming the supported formats. |
| **OCR / text extraction / PDF assembly** | Different product (see C-1, C-2 in `research.md`). Never in this repo. |
| **Any network feature** — update check, telemetry, "share", cloud, licence check | The entire wedge is "nothing leaves the machine". One HTTP call destroys the claim, and offline is also what makes the app work on the locked-down machine at the counter. |
| **Installers, MSIX, code signing, auto-update** | No budget, no certificate, no hosting. Ship a portable exe + SHA256 in the release notes. |
| **Explorer shell integration / right-click "Compress to 100 KB"** | Registry writes, uninstall debt, and per-user elevation. Drag-and-drop covers it. |
| **Presets library ("visa photo India", "US passport")** | We cannot verify any portal's rules (no web access, and rules change); shipping wrong presets is worse than shipping none. Two numeric fields instead. |

## 5. Top 3 risks

| # | Risk | Likelihood / impact | Mitigation (concrete) |
|---|---|---|---|
| **R1** | **SmartScreen / antivirus blocks an unsigned exe.** The user we described is non-technical; "Windows protected your PC" is where downloads die. | High / fatal to distribution | Publish the exe **and** a `.zip`; release notes carry the SHA256 and the exact click path ("More info → Run anyway"); source is public and the Actions workflow is reproducible so the binary matches the source; README states plainly that the exe is unsigned and why. Accepted residual risk: we cannot remove the warning without a certificate we will not buy. **Escalate to Founder before any public announcement** — a release that triggers AV flags is reputational (BRIEF rule 4). |
| **R2** | **"Under the limit" is not true on the user's portal** — they mean 100,000 bytes, or they also enforce DPI, or they reject the dimensions. | Medium / high (it is the whole promise) | Target is interpreted as **1 KB = 1000 bytes**, which satisfies both common readings (≤100,000 and ≤102,400) at a ~2% quality cost; every output's exact byte count and pixel dimensions are shown in the list; the app states the limits it enforced in the success line; the target is re-verified after the final encode by re-reading the written file's length, not by trusting the in-memory buffer. |
| **R3** | **The core is untestable on the Linux dev host** because image codecs are Windows-only (`System.Drawing.Common` throws on non-Windows since .NET 6), so real JPEG sizes only get exercised in CI — a bug class that costs money to find late. | Medium / medium | Hard architectural split: `Kilobyt.Core` has **no** image dependency — it owns pixel buffers (`Rgb24Image`), the resampler, the planner, the search policy and the namer behind an `IJpegCodec` interface. All of that is unit-tested on Linux with a deterministic fake codec. One Windows-only integration test project runs in the same CI job and asserts real encoder behaviour (size monotonicity, metadata stripping). If the Windows integration test fails, the release does not publish. |

Secondary risks tracked but not top-3: V1 quality cliff producing visibly bad JPEGs at q30 (mitigated by a quality floor + explicit failure instead of silent garbage); target unreachable on large images (explicit failure, no output file); path/permission edge cases (specified in PRD §7).

## 6. Success metric for v1 (post-launch, day 30)

| # | Metric | Baseline | Target | Source |
|---|---|---|---|---|
| M1 | Release downloads → successful batch completions | 0 | ≥ 40% of downloads run ≥1 successful compression | no telemetry by design — measured only from opt-in GitHub issue reports and release asset download counts (**ASSUMPTION** — this is a weak proxy, since an offline tool cannot measure activation without violating the no-network rule) |
| M2 | Reports of "output was rejected by the portal" | — | ≤ 2 in the first 30 days | issue tracker |
| M3 | Crash/failure reports per 100 downloads | — | ≤ 1 | issue tracker |

> Note for CPO: M1 cannot be instrumented honestly in an offline app. It is recorded as a **proxy**, not an outcome metric. Do not report it as adoption.

## 7. What would make us kill this

- ≥3 independent reports that the app produced a file the portal still rejected for size (means the byte-limit model is wrong).
- SmartScreen/AV blocking in a way users cannot click through.
- A free, offline, no-install tool from a credible vendor shipping the same guarantee (the wedge is "offline + guaranteed", not "compress").
