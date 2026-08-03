fn main() {
    // ── Copy ecram_service.exe + micontrol_bridge.exe to bin/ BEFORE
    //    tauri_build so the resource paths exist when tauri-build validates
    //    resources. ──────────────────────────────────────────────────────
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
        let dst_dir = std::path::Path::new("bin");
        let _ = std::fs::create_dir_all(dst_dir);

        // Copy ecram_service.exe (IoTService.exe replacement in DriverStore).
        copy_aux_binary(&target_dir, &profile, "ecram_service.exe", dst_dir);

        // Copy micontrol_bridge.exe (autonomous elevated service).
        copy_aux_binary(&target_dir, &profile, "micontrol_bridge.exe", dst_dir);

        // Copy micontrol_face_svc.exe (face auth service, LocalSystem).
        copy_aux_binary(&target_dir, &profile, "micontrol_face_svc.exe", dst_dir);

        // Copy micontrol_facecp.dll (Credential Provider).
        copy_aux_binary(&target_dir, &profile, "micontrol_facecp.dll", dst_dir);
    }

    let windows_attrs =
        tauri_build::WindowsAttributes::new().app_manifest(include_str!("windows-manifest.xml"));

    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows_attrs))
        .expect("failed to run tauri-build");
}

/// Copy an auxiliary binary from the cargo target dir into `bin/`.
/// If the binary is not yet built, create an empty placeholder so
/// tauri-build's resource validation does not fail.
#[cfg(windows)]
fn copy_aux_binary(
    target_dir: &std::path::Path,
    profile: &str,
    name: &str,
    dst_dir: &std::path::Path,
) {
    let src = target_dir.join(profile).join(name);
    let dst = dst_dir.join(name);

    if src.exists() {
        // The source binary may be locked by a running service (e.g. the
        // bridge service running from target\debug in dev mode). In that
        // case keep the previously-copied bin/ copy instead of failing.
        if let Err(e) = std::fs::copy(&src, &dst) {
            println!(
                "cargo:warning=Could not copy {name} to bin/ ({e}); keeping existing bin/ copy if present"
            );
        } else {
            println!("cargo:rerun-if-changed={}", src.display());
        }
    } else {
        if !dst.exists() {
            let _ = std::fs::write(&dst, []);
        }
        println!(
            "cargo:warning={name} not found at {} — placeholder created",
            src.display()
        );
    }
}
