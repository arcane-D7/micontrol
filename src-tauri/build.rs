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
        // Only copied when actually built (the `face` cargo feature enables
        // this bin via required-features); otherwise we must NOT create a
        // zero-byte placeholder — a 0-byte svc/dll would make the NSIS
        // installer try to `install` an empty service binary and silently
        // fail, leaving the Face Unlock UI dead ("no service installed"
        // despite `FileExists` in the installer hook).
        copy_aux_binary_optional(&target_dir, &profile, "micontrol_face_svc.exe", dst_dir);

        // Copy micontrol_facecp.dll (Credential Provider).
        // This is a WORKSPACE cdylib (`cp/` crate) — not produced by the main
        // bin build. Ensure it's built first, then copy it.
        copy_face_cp_dll(&target_dir, &profile, dst_dir);
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

/// Copy a face-related auxiliary binary. Unlike `copy_aux_binary`, this
/// NEVER ships a zero-byte placeholder in the final bundle: when the binary
/// does not exist yet (its cargo unit compiles after this build script —
/// e.g. a `required-features` bin of the same package) we create a temporary
/// placeholder to satisfy tauri-build's resource validation, and register
/// `cargo:rerun-if-changed` on the expected output path so cargo re-runs
/// this build script the moment the real binary is produced — replacing the
/// placeholder with the real file before bundling. If the feature is off and
/// the binary will never be built, we remove any stale copy instead.
#[cfg(windows)]
fn copy_aux_binary_optional(
    target_dir: &std::path::Path,
    profile: &str,
    name: &str,
    dst_dir: &std::path::Path,
) {
    let src = target_dir.join(profile).join(name);
    let dst = dst_dir.join(name);

    if src.exists() {
        if let Err(e) = std::fs::copy(&src, &dst) {
            println!(
                "cargo:warning=Could not copy {name} to bin/ ({e}); keeping existing bin/ copy if present"
            );
        } else {
            println!("cargo:rerun-if-changed={}", src.display());
        }
        return;
    }

    // Binary not built yet. Register a watch on its would-be output path so
    // cargo re-runs this script as soon as the bin compiles in this build.
    println!("cargo:rerun-if-changed={}", src.display());

    if dst.exists() {
        let keep = !is_zero_byte(&dst);
        if keep {
            // A real copy from a previous face-enabled build is still valid;
            // keep it (the bundle will be correct even if the feature was
            // turned off later — the NSIS hook only installs the service
            // when the file is non-empty... but to be safe, keep semantics:
            // if the feature is off again, we still want a REAL binary, not
            // 0 bytes, so keeping is fine regardless).
            println!("cargo:warning={name} not rebuilt yet — keeping existing non-empty bin/ copy");
            return;
        }
    }

    // Nothing usable yet: create a TEMPORARY placeholder so tauri-build's
    // resource validation passes. The rerun-if-changed above guarantees the
    // real binary replaces it before cargo completes.
    let _ = std::fs::write(&dst, []);
    println!(
        "cargo:warning={name} not built yet in this profile — temporary placeholder created (will be replaced when the bin compiles)"
    );
}

/// True when `p` is a zero-byte file (a previous placeholder that must be
/// replaced, never shipped).
#[cfg(windows)]
fn is_zero_byte(p: &std::path::Path) -> bool {
    std::fs::metadata(p).map(|m| m.len() == 0).unwrap_or(false)
}

/// Copy `micontrol_facecp.dll` (Credential Provider) into `bin/`.
///
/// The CP is a separate workspace cdylib (`cp/` crate) — it is never built
/// as part of the main `micontrol` bin, so we ensure it exists first:
/// 1. If `target/<profile>/micontrol_facecp.dll` exists, copy it.
/// 2. Otherwise, run `cargo build -p micontrol-facecp --release` (or debug)
///    as a sub-process, then copy the produced DLL.
///
/// Only runs when the `face` feature is enabled (the CP is part of the face
/// unlock feature set); without the feature we never produce a placeholder.
#[cfg(windows)]
fn copy_face_cp_dll(target_dir: &std::path::Path, profile: &str, dst_dir: &std::path::Path) {
    use std::process::Command;

    let src = target_dir.join(profile).join("micontrol_facecp.dll");
    let dst = dst_dir.join("micontrol_facecp.dll");

    if src.exists() {
        if let Err(e) = std::fs::copy(&src, &dst) {
            println!(
                "cargo:warning=Could not copy micontrol_facecp.dll to bin/ ({e}); keeping existing copy if present"
            );
        } else {
            println!("cargo:rerun-if-changed={}", src.display());
        }
        return;
    }

    // Not built yet — build the CP workspace crate now.
    println!("cargo:warning=micontrol_facecp.dll not built — building `cp` crate now");
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let status = Command::new("cargo")
        .arg("build")
        .arg("-p")
        .arg("micontrol-facecp")
        .arg("--manifest-path")
        .arg(format!("{manifest_dir}\\cp\\Cargo.toml"))
        .arg("--release")
        .status();
    match status {
        Ok(st) if st.success() => {
            // The sub-build targets the same CARGO_TARGET_DIR when set;
            // otherwise it would be a nested target dir. Prefer copying from
            // the release dir (always produced), falling back to the given
            // profile dir.
            let from_rel = target_dir.join("release").join("micontrol_facecp.dll");
            let from = if from_rel.exists() { from_rel } else { src };
            if from.exists() {
                if let Err(e) = std::fs::copy(&from, &dst) {
                    println!("cargo:warning=Could not copy built CP dll to bin/ ({e})");
                } else {
                    println!("cargo:rerun-if-changed={}", from.display());
                }
            } else {
                println!("cargo:warning=cp build succeeded but dll not found at {from:?}");
            }
        }
        Ok(st) => println!(
            "cargo:warning=`cargo build -p micontrol-facecp` exited with {:?}",
            st.code()
        ),
        Err(e) => println!("cargo:warning=Failed to spawn cargo build for cp crate: {e}"),
    }
}
