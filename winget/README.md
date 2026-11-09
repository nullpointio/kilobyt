# winget manifests

Local (non-store) [WinGet](https://learn.microsoft.com/windows/package-manager/) manifests for
Kilobyt 1.1.4. These are **not** in the community repository yet - they let anyone install the
release without waiting for a review, and they are what a submission PR would contain.

## Install from this folder

Local manifest installs are disabled by default. Enable them once, in an **elevated** PowerShell:

```powershell
winget settings --enable LocalManifestFiles
```

Then, from the repository root:

```powershell
winget install --manifest winget
```

Validate without installing:

```powershell
winget validate --manifest winget
```

## Files

| File | Purpose |
|---|---|
| `nullpointio.Kilobyt.yaml` | version manifest |
| `nullpointio.Kilobyt.locale.en-US.yaml` | name, publisher, license, description |
| `nullpointio.Kilobyt.installer.yaml` | portable installer URL + SHA-256 |

The installer URL is version-pinned and the hash is the one published in the release's
`SHA256SUMS.txt`, so the manifest cannot silently follow a replaced asset.

## Submitting to the community repository

A submission to [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) needs the same
three files under `manifests/<letter>/<publisher>/<package>/<version>/`, a fork, and a pull request
whose author has signed the Microsoft CLA. An unattended GitHub App cannot complete that step; the
blocker is recorded in the org's marketing notes (`packaging.md`, round 1).
