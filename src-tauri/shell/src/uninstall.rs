//! Full removal of MiControl — port of the NSIS POSTUNINSTALL hook.

use std::path::Path;
use std::time::Duration;

use crate::ctx;
use crate::sys;

/// Remove everything. Runs elevated, from %TEMP% (staged) or from the shell's
/// own directory when that is outside `install_dir`.
pub fn run(install_dir: &Path, _silent: bool) -> i32 {
    println!("==> MiControl uninstall");
    println!("==> install directory: {}", install_dir.display());

    // ── 1. Stop + remove the app and all auxiliary processes/services ──────
    println!("==> stopping MiControl...");
    sys::kill_and_wait(ctx::MAIN_EXE, Duration::from_secs(10));

    println!("==> removing MiControlBridge service...");
    let bridge = install_dir.join(ctx::BRIDGE_EXE);
    if bridge.is_file() {
        let (code, out) = sys::run_capture(&bridge.display().to_string(), &["uninstall"]);
        if code == 0 {
            println!("    MiControlBridge service removed: OK");
        } else {
            println!("    WARNING: bridge uninstall returned {code} (ignored if already removed).\n    {out}");
        }
    }
    sys::service_stop_wait(ctx::APP_SERVICE, Duration::from_secs(10));
    sys::service_delete_best_effort(ctx::APP_SERVICE);

    println!("==> removing MiControlFace service + Credential Provider...");
    let face_svc = install_dir.join(ctx::FACE_SVC_EXE);
    if face_svc.is_file() {
        let (code, _) = sys::run_capture(&face_svc.display().to_string(), &["remove"]);
        if code != 0 {
            println!(
                "    WARNING: face service remove returned {code} (ignored if already removed)."
            );
        }
    }
    sys::service_stop_wait(ctx::FACE_SERVICE, Duration::from_secs(10));
    sys::service_delete_best_effort(ctx::FACE_SERVICE);
    sys::kill_and_wait("micontrol_face_svc.exe", Duration::from_secs(10));

    remove_credential_provider_registry();

    println!("==> removing IoTSvc + restoring DriverStore...");
    remove_iot_service_and_restore();

    // ── 2. Scheduled task, autostart, shortcuts ────────────────────────────
    let (code, _) = sys::run_capture("schtasks.exe", &["/delete", "/tn", ctx::ELEV_TASK, "/f"]);
    println!("==> MiControlElevated task removed: exit {code}");
    remove_run_key();

    crate::shortcut::delete_shortcut(&crate::shortcut::desktop_shortcut_path());
    crate::shortcut::delete_shortcut(&crate::shortcut::start_menu_shortcut_path());

    // ── 3. Uninstall registry key ───────────────────────────────────────────
    {
        use winreg::enums::HKEY_LOCAL_MACHINE;
        let hk = winreg::RegKey::predef(HKEY_LOCAL_MACHINE);
        match hk.delete_subkey(ctx::UNINST_KEY_PATH) {
            Ok(()) => println!("==> uninstall registry key removed."),
            Err(e) => println!("    note: uninstall key not removed ({e})."),
        }
    }

    // ── 4. Delete the installation directory ───────────────────────────────
    println!("==> deleting install directory...");
    match std::fs::remove_dir_all(install_dir) {
        Ok(()) => println!("    directory removed."),
        Err(e) => {
            // The staged shell may still hold the last handle for a moment.
            std::thread::sleep(Duration::from_millis(1500));
            match std::fs::remove_dir_all(install_dir) {
                Ok(()) => println!("    directory removed (retry)."),
                Err(e2) => println!(
                    "    WARNING: could not fully delete {}: {e2}",
                    install_dir.display()
                ),
            }
            let _ = e;
        }
    }

    println!("==> MiControl removed.");
    crate::EXIT_OK
}

fn remove_credential_provider_registry() {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    let hk = winreg::RegKey::predef(HKEY_LOCAL_MACHINE);
    for key in [
        r"SOFTWARE\Classes\CLSID\{E071A7CE-5D7F-4063-9A10-AE39AEC64EE8}",
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\{E071A7CE-5D7F-4063-9A10-AE39AEC64EE8}",
    ] {
        if hk.delete_subkey_all(key).is_ok() {
            println!("    removed: {key}");
        }
    }
}

/// Stop + delete IoTSvc; restore the original Xiaomi IoTService.exe from the
/// `.bak` rollback copy when it exists.
fn remove_iot_service_and_restore() {
    if sys::service_exists(ctx::IOT_SERVICE) {
        let _ = sys::run_capture(
            "sc.exe",
            &["failure", ctx::IOT_SERVICE, "reset=", "0", "actions=", ""],
        );
        sys::service_stop_wait(ctx::IOT_SERVICE, Duration::from_secs(15));
        sys::service_delete_best_effort(ctx::IOT_SERVICE);
        println!("    IoTSvc service removed.");
    } else {
        println!("    IoTSvc service not present — skipping.");
    }
    sys::kill_and_wait("IoTService.exe", Duration::from_secs(8));
    sys::kill_and_wait(ctx::ECRAM_EXE, Duration::from_secs(8));

    // Restore the .bak (best-effort; DriverStore package removal is left to
    // pnputil /delete-driver, which we do NOT run automatically to avoid
    // breaking other tooling that may share the driver).
    if let Some(pkg) = crate::install::driverstore_package_dir() {
        let bak = pkg.join("IoTService.exe.bak");
        let dest = pkg.join("IoTService.exe");
        if bak.is_file() {
            match std::fs::copy(&bak, &dest) {
                Ok(_) => println!("    original IoTService.exe restored from .bak."),
                Err(e) => println!("    WARNING: could not restore IoTService.exe.bak: {e}"),
            }
        }
    }
}

fn remove_run_key() {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_SET_VALUE};
    for (hive, label) in [(HKEY_LOCAL_MACHINE, "HKLM"), (HKEY_CURRENT_USER, "HKCU")] {
        let hk = winreg::RegKey::predef(hive);
        if let Ok(key) = hk.open_subkey_with_flags(ctx::RUN_KEY_PATH, KEY_SET_VALUE) {
            if key.delete_value("MiControl").is_ok() {
                println!("    {label}\\...\\Run: MiControl removed.");
            }
        }
    }
}
