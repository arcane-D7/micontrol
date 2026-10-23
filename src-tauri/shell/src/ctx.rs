//! Shared constants, install-directory resolution and path helpers.

use std::path::{Path, PathBuf};

pub const PRODUCT: &str = "MiControl";
pub const MAIN_EXE: &str = "micontrol.exe";
pub const ECRAM_EXE: &str = "ecram_service.exe";
pub const BRIDGE_EXE: &str = "micontrol_bridge.exe";
pub const FACE_SVC_EXE: &str = "micontrol_face_svc.exe";
pub const FACE_CP_DLL: &str = "micontrol_facecp.dll";
pub const SHELL_DIR: &str = "setup";

pub const APP_SERVICE: &str = "MiControlBridge";
pub const FACE_SERVICE: &str = "MiControlFace";
pub const IOT_SERVICE: &str = "IoTSvc";
pub const ELEV_TASK: &str = "MiControlElevated";

pub const UNINST_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\MiControl";
pub const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Default GitHub repo used to locate the latest signed payload.
pub const DEFAULT_REPO: &str = "arcane-D7/micontrol";

/// minisign public key (same key pair the Tauri updater uses). The blob is
/// the base64 of "untrusted comment: ...\nRWS<key>\n".
pub const PUBKEY_BLOB: &str =
    "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDgyODg4MjcyNUM2MjIyOUEKUldTYUltSmNjb0tJZ2tGdXBhSkNoU3VrTjI5bDRiMXYzL2l1NlQ0MnBSblR1OW9TemhZWXRRV3cK";

/// WebView2 runtime GUID (per-user/machine presence check).
pub const WEBVIEW2_GUID: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";

/// Resolve the installation directory.
///
/// Order: explicit `--dir` → registry `InstallLocation` → `%ProgramFiles%\MiControl`.
/// When `create` is set the directory is created if missing.
pub fn resolve_install_dir(explicit: Option<&Path>, create: bool) -> PathBuf {
    let dir: PathBuf = if let Some(d) = explicit {
        d.to_path_buf()
    } else if let Some(d) = registry_install_location() {
        d
    } else {
        let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
        PathBuf::from(pf).join(PRODUCT)
    };

    if create {
        let _ = std::fs::create_dir_all(&dir);
    }
    dir
}

/// Read `InstallLocation` from the uninstall registry key (written by both
/// the legacy NSIS installer and this shell).
fn registry_install_location() -> Option<PathBuf> {
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY};
    let hk = winreg::RegKey::predef(HKEY_LOCAL_MACHINE);
    for flags in [KEY_READ, KEY_READ | KEY_WOW64_32KEY] {
        if let Ok(key) = hk.open_subkey_with_flags(UNINST_KEY_PATH, flags) {
            if let Ok(loc) = key.get_value::<String, _>("InstallLocation") {
                let loc = loc.trim().trim_matches('"').trim_end_matches('\\');
                if !loc.is_empty() {
                    return Some(PathBuf::from(loc));
                }
            }
        }
    }
    None
}

/// Path of the installed shell binary inside the install directory.
pub fn shell_path(install_dir: &Path) -> PathBuf {
    install_dir.join(SHELL_DIR).join("micontrol_setup.exe")
}

/// True when both paths exist and refer to the same file.
pub fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// True when `path` is located under `dir`.
pub fn is_under(path: &Path, dir: &Path) -> bool {
    match (std::fs::canonicalize(path), std::fs::canonicalize(dir)) {
        (Ok(p), Ok(d)) => p.starts_with(&d),
        _ => false,
    }
}

/// Path of the local per-user staging area used by the update flow.
pub fn local_appdata_dir() -> PathBuf {
    std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Users\Public"))
        .join(PRODUCT)
}
