fn main() {
    // ── Copy ecram_service.exe to bin/ BEFORE tauri_build so the resource
    //    path exists when tauri-build validates resources. ──────────────
    #[cfg(windows)]
    {
        let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
        let target_dir = std::env::var("CARGO_TARGET_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|_| std::path::PathBuf::from("."));
                manifest_dir.join("target")
            });
        let src = target_dir.join(&profile).join("ecram_service.exe");
        let dst_dir = std::path::Path::new("bin");
        let _ = std::fs::create_dir_all(dst_dir);
        let dst = dst_dir.join("ecram_service.exe");

        if src.exists() {
            let _ = std::fs::copy(&src, &dst);
            println!("cargo:rerun-if-changed={}", src.display());
        } else {
            // Create a placeholder so tauri-build doesn't fail on resource
            // validation. The real binary will be built by `cargo build
            // --bin ecram_service` and copied over in a subsequent build.
            if !dst.exists() {
                let _ = std::fs::write(&dst, []);
            }
            println!(
                "cargo:warning=ecram_service.exe not found at {} — placeholder created",
                src.display()
            );
        }
    }

    let windows_attrs =
        tauri_build::WindowsAttributes::new().app_manifest(include_str!("windows-manifest.xml"));

    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows_attrs))
        .expect("failed to run tauri-build");
}
