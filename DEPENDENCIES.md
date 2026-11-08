# Dependency policy

Kilobyt is an offline, zero-runtime desktop binary, so a vulnerable or
unmaintained crate is a shipped defect, not a build detail. These are the rules.

## Rules

- **Latest stable at build time.** CI always builds on
  `dtolnay/rust-toolchain@stable`; there is no pinned toolchain file.
- **MSRV is declared, not implied.** `rust-version` in `Cargo.toml` (currently
  **1.75**) is the oldest compiler we promise to support. A bump that raises the
  MSRV is a deliberate decision: update `rust-version` in the same PR and say so
  in the description.
- **`Cargo.lock` is committed.** It is the input to every audit below; an
  uncommitted lockfile means the checks describe a graph nobody builds.
- **Dependabot runs weekly** (`.github/dependabot.yml`). Minor + patch updates
  arrive as one grouped PR (`rust-deps`), majors as a separate PR
  (`rust-deps-major`), and `github-actions` versions get their own weekly PR.
  At most 5 open PRs per ecosystem. Review and merge; never let a PR sit for
  weeks while the issue below fills up.
- **Advisories block release.** `cargo audit` runs in `ci.yml` and in the `deps`
  workflow. A failing audit means no release.
- **Licenses are allow-listed.** `cargo deny check licenses bans sources`
  (`deny.toml`) permits only MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC,
  Unicode-3.0, Zlib and MPL-2.0. GPL, AGPL and SSPL are effectively denied
  (anything not on the allow-list is), and only the crates.io registry is
  accepted — no git or unknown-registry sources.

## What runs where

| Workflow | Job (check name) | Trigger | Blocks a release? |
| --- | --- | --- | --- |
| `ci.yml` | `Rust tests + lint (linux)` | every push, PR and `v*` tag | **yes** — `windows` needs it, `release` needs `windows` |
| `deps.yml` | `Dependency audit` | weekly cron, manual, push touching `Cargo.lock`/`Cargo.toml`/`deny.toml` | **yes** — required check by name |
| `deps.yml` | `Dependency freshness` | same | no — advisory, reports via the issue |

The `deps` workflow runs on a weekly schedule (`17 6 * * 1`, Mondays 06:17 UTC),
on `workflow_dispatch`, and on any push to `main` that changes the lockfile or
manifest. All checks are **resolution-only**: nothing is compiled, so the
Windows-only crates and `build.rs` are never exercised and the checks run fine
on `ubuntu-latest`.

## Release gating

`release` in `ci.yml` requires `windows`, which requires `test`. The `test` job
runs `cargo audit` directly, so a vulnerable dependency fails the build on the
very commit being tagged — no tag can produce a release with a known advisory.

Belt and braces: mark **`Dependency audit`** (workflow `deps`) as a *required
status check* on the `main` branch protection rule. GitHub reports it as
`deps / Dependency audit`. Keep the name stable — renaming the job silently
un-gates releases.

## When the issue appears

An open issue titled **`deps: dependency drift or advisory`** means one of the
checks failed. Its body holds the captured output in collapsible blocks.

1. Read the failing section. `audit.txt` = advisory, `deny.txt` = license/source
   problem, `outdated.txt` = a direct dependency is behind.
2. **Advisory:** bump the affected crate to a patched version
   (`cargo update -p <crate> --precise <version>`), or if no fix exists, stop
   shipping and evaluate removing the dependency. Do not add an `ignore` entry
   to silence it.
3. **License failure:** either swap the crate, or — only with a deliberate
   decision recorded in the PR — add a narrow `exceptions` entry in `deny.toml`.
   Never widen the global allow-list to unblock one build.
4. **Outdated:** take the Dependabot PR, or run `cargo update -p <crate>`. Then
   `cargo test --all --release` on Windows and Linux.
5. Close the issue once `deps` is green. It reopens itself if the problem
   returns.

Local reproduction:

```sh
cargo audit
cargo deny check licenses bans sources
cargo outdated --workspace --root-deps-only --exit-code 1
```
