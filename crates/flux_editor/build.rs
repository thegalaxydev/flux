fn main() {
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
