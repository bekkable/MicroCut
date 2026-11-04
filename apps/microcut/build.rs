//! Windows: puts the icon and the version information into `microcut.exe`. Elsewhere nothing to do (the
//! window icon is set at run time, the macOS bundle icon by `cargo xtask dist`).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
    }
}

/// Never fails the build: without the Windows SDK's resource compiler the exe just has no icon.
#[cfg(windows)]
fn windows_resources() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico")
        // Task Manager and the "Open with" list show these.
        .set("FileDescription", "MicroCut")
        .set("ProductName", "MicroCut")
        .set(
            "LegalCopyright",
            "© The MicroCut contributors, MIT OR Apache-2.0",
        );
    if let Err(err) = res.compile_for("microcut") {
        println!("cargo:warning=no icon in microcut.exe: {err}");
    }
}
