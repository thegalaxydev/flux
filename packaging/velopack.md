# Velopack: per-user installer + auto-update

Flux ships as a [Velopack](https://velopack.io) app: a small `Flux-<ver>-Setup.exe`
that installs per-user (no admin), and the editor silently updates itself from the
GitHub Releases feed. This complements the plain zip / Inno installer in
`release.yml`; you can drop those once you're happy with Velopack.

## One-time setup

1. **Point the updater at your repo.** The in-app updater
   (`crates/flux_editor/src/updater.rs`) reads the release feed from the crate's
   `repository` URL. Add it to `crates/flux_editor/Cargo.toml`:
   ```toml
   [package]
   repository = "https://github.com/<owner>/<repo>"
   ```
   Until this is set the updater is inert (dev builds never phone home).

2. **Install the CLI** (needs the .NET 8 SDK):
   ```
   dotnet tool install -g vpk
   ```

3. **Code signing.** SmartScreen trust now hinges on the `*-Setup.exe`. Configure
   Azure Trusted Signing (the `AZURE_*` / `TRUSTED_SIGNING_*` repo secrets in
   `.github/workflows/velopack.yml`), or pass `--signParams` to `vpk pack` for a
   local cert. The app exes inside the package are signed before packing.

## Build a release locally

```
# 1. Stage the self-contained folder (exes + flux_script.dll + std-*.dll + plugins).
cargo xtask dist --no-zip --version 0.1.0

# 2. Pack it into dist/velopack/ (Setup.exe + *.nupkg + JSON feed).
cargo xtask velopack --version 0.1.0
```

This emits into `dist/velopack/`: `Flux-win-Setup.exe` (what users download),
`Flux-<ver>-full.nupkg` (the package), `Flux-win-Portable.zip`, and the update
manifests (`releases.win.json` / `assets.win.json` / `RELEASES`).

Test the result by running `dist/velopack/Flux-win-Setup.exe` — it installs to
`%LOCALAPPDATA%\Flux`, adds Desktop + Start-menu shortcuts, and launches. Publish
another tagged version and an installed copy updates itself on next launch.

## Cutting a release (CI)

Push a version tag and `velopack.yml` runs automatically:
```
git tag v0.1.0 && git push origin v0.1.0
```
It stages the folder, signs the app exes, pulls the previous release for delta
updates, packs, signs the setup, and uploads `*Setup.exe` + `*.nupkg` + `RELEASES`
to the GitHub release. The updater's `GithubSource` reads that feed.

## Notes / gotchas

- **Pack id (`-u Flux`) must never change** across releases, or updates won't be
  recognized. The version (`-v`) must increase.
- **Delta updates** need the previous release's `.nupkg` present at pack time; CI
  fetches it via `vpk download github` (a no-op on the first release, so the first
  update is a full download).
- **App icon**: `vpk pack` can take `--icon path\to\app.ico`. `logo/` currently has
  a PNG only — add an `.ico` and pass it to get a branded installer/shortcut.
- The updater checks once at startup on a background thread and shows a
  "Restart & Update" banner when a new version is downloaded; it never interrupts
  work mid-session.
