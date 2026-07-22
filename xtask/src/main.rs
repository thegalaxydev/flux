//! Flux developer automation. Run through the `cargo xtask` alias.
//!
//! Commands:
//!   dist [--version <v>] [--no-zip] [--installer]
//!       Build the release binaries and assemble a **self-contained** Windows
//!       bundle under `dist/`, then zip it and print its SHA-256.
//!   sync-plugins [--release]
//!       Copy the freshly built `flux_game.dll` into every `projects/*/plugins/`.
//!       The editor's `build.rs` tries to do this, but cargo only re-runs a build
//!       script when its own inputs change — so after an engine-only change the
//!       project plugins go stale and loading them fails. Run this to be sure.
//!   velopack --version <v>
//!       Pack the staged `dist/` folder into a Velopack per-user installer +
//!       update feed under `dist/velopack/` (`vpk pack`). Run `dist --no-zip`
//!       first. Requires the `vpk` CLI (`dotnet tool install -g vpk`).
//!
//! Why a bundler at all: the workspace builds with `-C prefer-dynamic` and
//! `flux_script` is a Rust `dylib` (a hard requirement — one mlua/Luau per
//! process). So a Flux binary is never standalone: it needs `flux_script.dll`,
//! the toolchain's `std-*.dll`, and the game plugin `flux_game.dll` beside it.
//! This command gathers all of them (plus licenses) so a download actually runs.

use std::path::{Path, PathBuf};
use std::process::Command;

type R<T> = Result<T, String>;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("dist") => dist(&args[1..]),
        Some("sync-plugins") => sync_plugins(&args[1..]),
        Some("velopack") => velopack(&args[1..]),
        Some(other) => {
            Err(format!("unknown command '{other}' (expected: dist, sync-plugins, velopack)"))
        }
        None => Err("usage: cargo xtask <dist|sync-plugins|velopack> [flags]".into()),
    };
    if let Err(e) = result {
        eprintln!("xtask: {e}");
        std::process::exit(1);
    }
}

/// Repo root (the workspace dir this crate lives under).
fn repo_root() -> PathBuf {
    // xtask/Cargo.toml is at <root>/xtask, so the manifest dir's parent is root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .expect("xtask is nested under the repo root")
}

struct DistOpts {
    version: String,
    zip: bool,
    installer: bool,
}

fn parse_dist_args(args: &[String]) -> R<DistOpts> {
    let mut version = None;
    let mut zip = true;
    let mut installer = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--version" => {
                version = Some(it.next().ok_or("--version needs a value")?.clone());
            }
            "--no-zip" => zip = false,
            "--installer" => installer = true,
            other => return Err(format!("unknown flag '{other}'")),
        }
    }
    // A leading `v` (git tag style) is stripped for folder/zip names.
    let version = version
        .map(|v| v.trim_start_matches('v').to_string())
        .unwrap_or_else(|| workspace_version().unwrap_or_else(|_| "0.0.0-dev".into()));
    Ok(DistOpts { version, zip, installer })
}

fn dist(args: &[String]) -> R<()> {
    let opts = parse_dist_args(args)?;
    let root = repo_root();
    let target_id = "windows-x64";
    let name = format!("Flux-{}-{target_id}", opts.version);
    let dist_dir = root.join("dist");
    let stage = dist_dir.join(&name);

    println!("== Flux dist {} ==", opts.version);

    // 1. Release build: exes, the shared flux_script dylib, and the game plugin.
    println!("-> building release binaries");
    run(
        Command::new(cargo())
            .current_dir(&root)
            .args(["build", "--release", "-p", "flux_editor", "-p", "flux_player", "-p", "flux_game"]),
    )?;

    // 2. Fresh staging dir.
    if stage.exists() {
        std::fs::remove_dir_all(&stage).map_err(|e| format!("clean {}: {e}", stage.display()))?;
    }
    std::fs::create_dir_all(stage.join("plugins")).map_err(|e| e.to_string())?;

    let rel = root.join("target").join("release");

    // 3. Core binaries + the shared dylib (required to launch).
    for f in ["flux_editor.exe", "flux_player.exe", "flux_script.dll"] {
        copy_into(&rel.join(f), &stage)?;
    }

    // 4. The stock game plugin, staged next to the exes. The engine loads a
    //    project's plugin from `<project>/plugins`, falling back to this bundled
    //    `plugins/` dir (see flux_plugin::resolve_plugin), so shipped/new
    //    projects work without the dev-only build.rs sync.
    copy_into(&rel.join("flux_game.dll"), &stage.join("plugins"))?;

    // 5. The toolchain std dylib(s) — `prefer-dynamic` means the exes import
    //    from `std-<hash>.dll`, which is version-pinned to this exact toolchain.
    let std_dlls = std_dlls()?;
    if std_dlls.is_empty() {
        return Err("no std-*.dll found in the toolchain — is this an MSVC toolchain?".into());
    }
    for dll in &std_dlls {
        copy_into(dll, &stage)?;
    }

    // 6. Legal + docs.
    copy_into(&root.join("LICENSE"), &stage)?;
    if root.join("README.md").exists() {
        copy_into(&root.join("README.md"), &stage)?;
    }
    third_party_licenses(&root, &stage.join("THIRD-PARTY-LICENSES.txt"));

    // 7. Zip + checksum.
    let manifest: Vec<String> = list_files(&stage)?
        .iter()
        .map(|p| p.strip_prefix(&stage).unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    println!("-> staged {} files in {}", manifest.len(), rel_display(&root, &stage));
    for m in &manifest {
        println!("     {m}");
    }

    if opts.zip {
        let zip_path = dist_dir.join(format!("{name}.zip"));
        zip_dir(&stage, &zip_path, &name)?;
        let sum = sha256_file(&zip_path)?;
        std::fs::write(dist_dir.join(format!("{name}.zip.sha256")), format!("{sum}  {name}.zip\n"))
            .map_err(|e| e.to_string())?;
        println!("-> {}  ({:.1} MiB)", rel_display(&root, &zip_path), file_mib(&zip_path));
        println!("   sha256 {sum}");
    }

    if opts.installer {
        build_installer(&root, &stage, &opts.version)?;
    }

    println!("== done ==");
    Ok(())
}

// ---------------------------------------------------------------------------
// sync-plugins
// ---------------------------------------------------------------------------

fn sync_plugins(args: &[String]) -> R<()> {
    let release = args.iter().any(|a| a == "--release");
    if let Some(bad) = args.iter().find(|a| *a != "--release") {
        return Err(format!("unknown flag '{bad}'"));
    }
    let root = repo_root();
    let profile = if release { "release" } else { "debug" };
    let src = root.join("target").join(profile).join("flux_game.dll");
    if !src.exists() {
        return Err(format!("{} not found — build it first (cargo build{})", src.display(),
            if release { " --release" } else { "" }));
    }
    let projects = root.join("projects");
    let mut synced = 0;
    for entry in std::fs::read_dir(&projects).map_err(|e| format!("read {}: {e}", projects.display()))? {
        let dir = entry.map_err(|e| e.to_string())?.path();
        if !dir.is_dir() {
            continue;
        }
        let plugins = dir.join("plugins");
        std::fs::create_dir_all(&plugins).map_err(|e| e.to_string())?;
        std::fs::copy(&src, plugins.join("flux_game.dll")).map_err(|e| e.to_string())?;
        println!("  synced -> {}", rel_display(&root, &plugins.join("flux_game.dll")));
        synced += 1;
    }
    println!("synced flux_game.dll ({profile}) into {synced} project(s)");
    Ok(())
}

// ---------------------------------------------------------------------------
// velopack (per-user installer + auto-update feed)
// ---------------------------------------------------------------------------

fn velopack(args: &[String]) -> R<()> {
    let mut version = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--version" => version = Some(it.next().ok_or("--version needs a value")?.clone()),
            other => return Err(format!("unknown flag '{other}'")),
        }
    }
    let version = version
        .map(|v| v.trim_start_matches('v').to_string())
        .unwrap_or_else(|| workspace_version().unwrap_or_else(|_| "0.0.0-dev".into()));

    let root = repo_root();
    let stage = root.join("dist").join(format!("Flux-{version}-windows-x64"));
    if !stage.exists() {
        return Err(format!(
            "{} not found — run `cargo xtask dist --no-zip --version {version}` first",
            rel_display(&root, &stage)
        ));
    }
    if Command::new("vpk").arg("--help").output().is_err() {
        return Err(
            "the `vpk` CLI was not found — install it with `dotnet tool install -g vpk` \
             (needs the .NET 8 SDK)"
                .into(),
        );
    }

    let out = root.join("dist").join("velopack");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    println!("-> vpk pack Flux {version}");
    // -u pack id (stable, must not change) · -v version · -p staged folder ·
    // -e main exe · -o output feed dir · --packTitle display name.
    run(Command::new("vpk").current_dir(&root).args([
        "pack",
        "-u",
        "Flux",
        "-v",
        &version,
        "-p",
        &stage.to_string_lossy(),
        "-e",
        "flux_editor.exe",
        "-o",
        &out.to_string_lossy(),
        "--packTitle",
        "Flux",
    ]))?;
    println!("-> velopack output in {}", rel_display(&root, &out));
    println!("   (Setup.exe + *.nupkg + RELEASES — upload all of these as release assets)");
    Ok(())
}

// ---------------------------------------------------------------------------
// std dylib discovery
// ---------------------------------------------------------------------------

/// The toolchain's `std-*.dll`(s), from `rustc --print sysroot`/bin. These carry
/// the dynamically-linked std the `prefer-dynamic` binaries import.
fn std_dlls() -> R<Vec<PathBuf>> {
    let out = Command::new(rustc())
        .args(["--print", "sysroot"])
        .output()
        .map_err(|e| format!("rustc --print sysroot: {e}"))?;
    if !out.status.success() {
        return Err("rustc --print sysroot failed".into());
    }
    let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let bin = Path::new(&sysroot).join("bin");
    let mut dlls = Vec::new();
    for entry in std::fs::read_dir(&bin).map_err(|e| format!("read {}: {e}", bin.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
        if name.starts_with("std-") && name.ends_with(".dll") {
            dlls.push(path);
        }
    }
    Ok(dlls)
}

// ---------------------------------------------------------------------------
// third-party licenses (best-effort via cargo-about)
// ---------------------------------------------------------------------------

/// Generate `THIRD-PARTY-LICENSES.txt` with `cargo about` if it's installed;
/// otherwise leave a placeholder pointing at the config so the release still
/// builds. cargo-about walks the dependency tree and emits every crate's notice.
fn third_party_licenses(root: &Path, out: &Path) {
    let cfg = root.join("about.toml");
    let has_about = Command::new(cargo())
        .args(["about", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if has_about && cfg.exists() {
        let res = Command::new(cargo())
            .current_dir(root)
            .args(["about", "generate", "about.hbs"])
            .output();
        if let Ok(o) = res {
            if o.status.success() {
                let _ = std::fs::write(out, o.stdout);
                println!("-> generated THIRD-PARTY-LICENSES.txt via cargo-about");
                return;
            }
            eprintln!("   cargo-about failed: {}", String::from_utf8_lossy(&o.stderr));
        }
    }
    let _ = std::fs::write(
        out,
        "Third-party license notices are generated by `cargo about generate about.hbs`.\n\
         Install it with `cargo install cargo-about` and re-run `cargo xtask dist`.\n",
    );
    eprintln!("   note: cargo-about not found — wrote a placeholder THIRD-PARTY-LICENSES.txt");
}

// ---------------------------------------------------------------------------
// installer (best-effort via Inno Setup's ISCC)
// ---------------------------------------------------------------------------

fn build_installer(root: &Path, stage: &Path, version: &str) -> R<()> {
    let iss = root.join("installer").join("flux.iss");
    if !iss.exists() {
        return Err(format!("{} not found", iss.display()));
    }
    let iscc = which_iscc().ok_or(
        "Inno Setup's ISCC.exe not found on PATH or in Program Files — install Inno Setup 6",
    )?;
    println!("-> building installer with {}", iscc.display());
    run(Command::new(iscc)
        .current_dir(root)
        .arg(format!("/DFluxVersion={version}"))
        .arg(format!("/DStageDir={}", stage.display()))
        .arg(&iss))?;
    Ok(())
}

fn which_iscc() -> Option<PathBuf> {
    // The default install locations first, then a bare `ISCC` on PATH.
    for base in ["C:/Program Files (x86)/Inno Setup 6", "C:/Program Files/Inno Setup 6"] {
        let p = Path::new(base).join("ISCC.exe");
        if p.is_file() {
            return Some(p);
        }
    }
    // `ISCC /?` prints help and exits non-zero, so a clean spawn means it exists.
    Command::new("ISCC").arg("/?").output().ok().map(|_| PathBuf::from("ISCC"))
}

// ---------------------------------------------------------------------------
// small helpers
// ---------------------------------------------------------------------------

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}
fn rustc() -> String {
    std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into())
}

fn run(cmd: &mut Command) -> R<()> {
    let status = cmd.status().map_err(|e| format!("spawn {cmd:?}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("command failed ({status}): {cmd:?}"))
    }
}

fn copy_into(src: &Path, dst_dir: &Path) -> R<()> {
    if !src.exists() {
        return Err(format!("missing build artifact: {}", src.display()));
    }
    let dst = dst_dir.join(src.file_name().ok_or("source has no file name")?);
    std::fs::copy(src, &dst).map_err(|e| format!("copy {} -> {}: {e}", src.display(), dst.display()))?;
    Ok(())
}

/// Every file under `dir`, recursively (relative order unspecified).
fn list_files(dir: &Path) -> R<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).map_err(|e| format!("read {}: {e}", d.display()))? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Zip `dir`'s contents under a top-level `prefix/` folder (so extraction makes
/// one clean `Flux-<ver>-windows-x64/` directory).
fn zip_dir(dir: &Path, zip_path: &Path, prefix: &str) -> R<()> {
    use std::io::Write;
    let file = std::fs::File::create(zip_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::FileOptions<()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for path in list_files(dir)? {
        let rel = path.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/");
        zip.start_file(format!("{prefix}/{rel}"), opts).map_err(|e| e.to_string())?;
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn sha256_file(path: &Path) -> R<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let digest = Sha256::digest(&bytes);
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

fn file_mib(path: &Path) -> f64 {
    std::fs::metadata(path).map(|m| m.len() as f64 / (1024.0 * 1024.0)).unwrap_or(0.0)
}

fn rel_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

fn workspace_version() -> R<String> {
    let text = std::fs::read_to_string(repo_root().join("crates/flux_editor/Cargo.toml"))
        .map_err(|e| e.to_string())?;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("version")
            && let Some(q0) = rest.find('"')
            && let Some(q1) = rest[q0 + 1..].find('"')
        {
            return Ok(rest[q0 + 1..q0 + 1 + q1].to_string());
        }
    }
    Err("could not read version from crates/flux_editor/Cargo.toml".into())
}
