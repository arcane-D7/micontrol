use std::path::Path;

/// Driver directories referenced by `tauri.conf.json` `bundle.resources`.
/// In CI (where drivers are not present) we create empty placeholder
/// directories so the Tauri glob pattern does not panic.
const DRIVER_GLOBS: &[(&str, &str)] = &[
    ("../../drivers/VirtualControlHID", "VirtualControlHID"),
    ("../../drivers/IoTDriver", "IoTDriver"),
];

fn ensure_driver_placeholders() {
    for (rel_path, name) in DRIVER_GLOBS {
        let dir = Path::new(rel_path);
        if !dir.exists() {
            // Create the directory with a placeholder file so the Tauri
            // resource glob `../../drivers/<name>/*` matches at least one file.
            if let Err(e) = std::fs::create_dir_all(dir) {
                println!("cargo:warning=Could not create placeholder dir for {name}: {e}");
                continue;
            }
            let placeholder = dir.join(".gitkeep");
            if let Err(e) = std::fs::write(&placeholder, b"") {
                println!("cargo:warning=Could not write placeholder for {name}: {e}");
            }
            println!(
                "cargo:warning=Created placeholder for driver '{name}' — \
                 bundled driver files are not present (CI environment)."
            );
        }
    }
}

fn main() {
    // In CI environments the OEM driver directories (outside the repo)
    // do not exist. Create empty placeholders so `tauri-build` does not
    // panic on the resource glob patterns.
    ensure_driver_placeholders();

    let windows_attrs =
        tauri_build::WindowsAttributes::new().app_manifest(include_str!("windows-manifest.xml"));

    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows_attrs))
        .expect("failed to run tauri-build");
}
