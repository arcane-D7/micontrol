fn main() {
    let windows_attrs = tauri_build::WindowsAttributes::new()
        .app_manifest(include_str!("windows-manifest.xml"));

    tauri_build::try_build(
        tauri_build::Attributes::new().windows_attributes(windows_attrs),
    )
    .expect("failed to run tauri-build");
}
