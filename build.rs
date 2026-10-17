fn main() {
    println!("cargo:rerun-if-changed=assets/app-icon.ico");
    println!("cargo:rerun-if-changed=packaging/windows/app.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/app-icon.ico")
        .set_manifest_file("packaging/windows/app.manifest")
        .set("ProductName", "ScreenCue")
        .set("FileDescription", "ScreenCue desktop hints")
        .set("LegalCopyright", "ScreenCue contributors. MIT License.");
    if let Err(error) = resource.compile() {
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
            println!("cargo:warning=Windows resource compilation unavailable in this GNU development toolchain: {error}. Release installers use the MSVC build with required resources.");
        } else {
            panic!("Windows icon and manifest resources could not be compiled: {error}");
        }
    }
}
