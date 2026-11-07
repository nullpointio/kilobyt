# Name candidates — v1

Constraint: lowercase GitHub repo slug, cloneable as typed, no hyphen noise, reads cleanly in a Windows title bar (`Name — window state`), no plausible trademark collision. **No web access during this exercise**, so collision checks are reasoning against known products only — see §4 for the verification step that must happen before the repo is created.

## 1. `kilobyt` — **CHOSEN**

| | |
|---|---|
| Slug | `kilobyt` — 7 chars, lowercase, one word, no hyphen |
| Title bar | `Kilobyt` / `Kilobyt — 3 of 14 done` |
| Why | Coined spelling of "kilobyte" that is not the dictionary word, which is exactly what keeps the slug and the trademark clear. Says the product's whole promise (a KB number) without saying "compress", which is the crowded word. Short enough for a taskbar label, one syllable-pair to say out loud. Ends in a hard consonant — survives being read over the phone. |
| Risk | Someone may type "kilobyte" into GitHub and miss it; a misspelling as a brand is a small memory tax. Mitigation: the release title is `Kilobyt — fit photos under a size limit`, and the repo description repeats "kilobyte" spelled correctly for search. |

## 2. `sizecap`

| | |
|---|---|
| Slug | `sizecap` |
| Title bar | `SizeCap — 14 photos` |
| Why | Immediately legible to a non-technical user: a cap on size. No spelling risk, no explanation needed in a download listing. |
| Risk | Generic descriptive coinage — the class of name most likely to already exist (a "SizeCap" utility, a CSS helper, a phone-plan feature) and the hardest to defend if it ever mattered. Also reads like a server quota tool, not a photo tool. |

## 3. `fitunder`

| | |
|---|---|
| Slug | `fitunder` |
| Title bar | `FitUnder — 14 photos` |
| Why | Describes the job in two words: fit *under* the limit. Pairs naturally with the product sentence ("fits under 100 KB, guaranteed") and is unusual enough that the slug is almost certainly free. |
| Risk | Reads as a bad compound ("fit under what?"), sounds like a fitness app to a skimming user, and the two-word run-together form is harder to say than `kilobyt`. Weakest of the three in a Windows title bar, where it looks like a typo. |

## 4. Decision and verification

**Pick `kilobyt`.** It is the only candidate that is memorable, defensible as a coinage, and legible at 12 pt in a title bar. `sizecap` is the fallback if §4 fails; `fitunder` is not a fallback (it mis-signals the category).

Before creating the repo (not done here — no external side effects allowed):
1. Search GitHub, the Microsoft Store, and a general web search for `kilobyt`, `kilobyt app`, `kilobyt photo`. Any live photo/compression product with that name → fall back to `sizecap`.
2. Check the slug is free at `github.com/<owner>/kilobyt`; if taken, `kilobyt-app` is acceptable but ships a hyphen into the clone URL, so prefer a different owner over a different name.
3. No trademark filing is contemplated; the name is used as a repo/product label only, and the README states it is not affiliated with any portal or government service (a claim-risk guard, not a legal opinion — GC owns that call).

**ASSUMPTION:** no registered mark on "Kilobyt" in software classes. Unverified without web access; verification is step 1 above, owned by whoever creates the repo.
