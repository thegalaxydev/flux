fn main() {
    // The plugin boundary is Rust ABI: host and plugin must be built by the
    // same compiler. Bake the rustc version into VERSION so a mismatched
    // plugin refuses to load instead of corrupting memory.
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let out = std::process::Command::new(rustc)
        .arg("--version")
        .output()
        .expect("run rustc --version");
    println!(
        "cargo:rustc-env=FLUX_RUSTC_VERSION={}",
        String::from_utf8_lossy(&out.stdout).trim()
    );
}
