# winget manifests (templates)

These are **templates** for publishing Flux to the [Windows Package Manager
Community Repository](https://github.com/microsoft/winget-pkgs). winget installs
the signed Inno Setup installer produced by the `release` workflow.

Package identifier: **`thegalaxydev.Flux`** (fixed — keep it identical across
every release). You submit these *after* a signed release exists, because two
fields come from the published artifacts.

## Per-release steps

1. Publish a signed release (push a `v*` tag → `release.yml`). Note the
   installer URL and its SHA-256 from `dist/SHA256SUMS.txt` in the release.
2. In all three files, replace the per-release placeholders:
   - `PACKAGE_VERSION` → the release version, e.g. `0.1.0`.
   - `INSTALLER_URL` → the `Flux-<ver>-setup.exe` download URL.
   - `INSTALLER_SHA256` → its SHA-256 (upper- or lower-case).
3. Validate and submit:
   ```
   winget validate --manifest packaging/winget
   winget submit  # or open a PR to microsoft/winget-pkgs under
                  # manifests/<f>/<Publisher>/Flux/<version>/
   ```

Tip: once the first version is accepted, [`wingetcreate update`](https://github.com/microsoft/winget-create)
can automate future version bumps straight from a GitHub release (a small
follow-up workflow can run it on each tag).
