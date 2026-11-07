//! On Windows, gives oxbow.exe its icon and version information (shown by Explorer, shortcuts and the installer).

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico")
        .set("ProductName", "Oxbow")
        .set("FileDescription", "Oxbow")
        .set("LegalCopyright", "GPL-3.0-only");
    if let Err(e) = res.compile() {
        // Without the Windows SDK's resource compiler the app still builds, just without an icon in Explorer.
        println!("cargo:warning=oxbow.exe gets no icon: {e}");
    }
}
