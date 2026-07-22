fn main() {
    sync_project_plugins();
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=../../logo/flux.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../logo/flux.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed exe icon: {e}");
        }
    }
}

fn sync_project_plugins() {
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent());
    let Some(root) = root else {
        return;
    };
    let src = root.join("target").join(&profile).join(plugin_filename());
    if !src.is_file() {
        return;
    }
    let projects = root.join("projects");
    let Ok(entries) = std::fs::read_dir(&projects) else {
        return;
    };
    for entry in entries.flatten() {
        let dst = entry.path().join("plugins").join(plugin_filename());
        if dst.parent().is_some_and(|p| p.parent().is_some()) {
            if let Some(parent) = dst.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::copy(&src, &dst).is_ok() {
                println!("cargo:warning=synced {} -> {}", src.display(), dst.display());
            }
        }
    }
    println!("cargo:rerun-if-changed={}", src.display());
}

fn plugin_filename() -> &'static str {
    if cfg!(windows) {
        "flux_game.dll"
    } else if cfg!(target_os = "macos") {
        "libflux_game.dylib"
    } else {
        "libflux_game.so"
    }
}
