//! Tauri's build step. On Windows the Common Controls 6 manifest goes into every binary the crate links, its tests
//! included: tauri-build embeds it in the app alone, and a test binary without it can't start
//! (`STATUS_ENTRYPOINT_NOT_FOUND`).

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let manifest =
            std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR")?).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    let windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))?;
    Ok(())
}
