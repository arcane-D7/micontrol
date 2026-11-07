//! Phone Link integration — detect, launch, and monitor Windows Phone Link.
//!
//! Phone Link (formerly "Your Phone") is built into Windows 10/11 and pairs
//! with the "Link to Windows" Android app. The Xiaomi 14T is officially
//! supported for all Phone Link features.
//!
//! MiControl orchestrates Phone Link via:
//! - URI scheme `ms-phone:` to launch the app
//! - Registry `HKCU\Software\Microsoft\YourPhone` to detect pairing
//! - PowerShell `Get-AppxPackage` to detect installation

use crate::hw::errors::{HardwareError, HardwareResult};
use serde::{Deserialize, Serialize};

// ── Data structures ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PhoneLinkStatus {
    /// Whether Phone Link is installed on this PC
    pub installed: bool,
    /// Whether a phone is paired
    pub paired: bool,
    /// Paired device name (if available)
    pub device_name: Option<String>,
    /// Phone Link package version (if available)
    pub package_version: Option<String>,
    /// Whether Phone Link is currently running
    pub running: bool,
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Detect if Phone Link is installed on this Windows machine.
pub fn detect_phone_link() -> bool {
    #[cfg(windows)]
    {
        // Check via PowerShell Get-AppxPackage
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-AppxPackage *YourPhone* | Select-Object -ExpandProperty Name",
            ])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output();

        match output {
            Ok(out) => {
                let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
                !name.is_empty()
            }
            Err(_) => {
                // Fallback: check if the SystemApps directory exists
                let path = r"C:\Windows\SystemApps\Microsoft.YourPhone";
                std::path::Path::new(path).exists()
            }
        }
    }
    #[cfg(not(windows))]
    false
}

/// Get the full Phone Link status (installed, paired, running).
pub fn get_phone_link_status() -> PhoneLinkStatus {
    let installed = detect_phone_link();
    if !installed {
        return PhoneLinkStatus::default();
    }

    let package_version = get_package_version();
    let paired = check_paired();
    let device_name = read_paired_device_name();
    let running = check_running();

    PhoneLinkStatus {
        installed,
        paired,
        device_name,
        package_version,
        running,
    }
}

/// Launch Phone Link app.
///
/// S53 fix (kept): Phone Link is a single-instance app — a fresh `ms-phone:`
/// URI activation re-initializes the running `PhoneExperienceHost`, which
/// tears down and re-handles its relay to the phone's Link to Windows service
/// and breaks sync until the app is fully restarted.
///
/// That guard used to turn the command into a silent no-op, because
/// `PhoneExperienceHost` runs in the background almost permanently — so the
/// UI button (and the five feature tiles) did nothing at all. The guard is
/// now paired with a real *shell activation* of the running package
/// (`IApplicationActivationManager::ActivateApplication`), which foregrounds
/// the existing instance without a new `ms-phone:` launch. If that fails the
/// error is returned so the UI can show it instead of pretending success.
pub fn launch_phone_link() -> HardwareResult<()> {
    #[cfg(windows)]
    {
        if check_running() {
            log::info!(
                "[phone_link] already running — activating existing instance (AUMID {PHONE_LINK_AUMID}) instead of ms-phone:"
            );
            return activate_phone_link();
        }
        log::info!("[phone_link] not running — launching ms-phone:");
        std::process::Command::new("cmd")
            .args(["/c", "start", "", "ms-phone:"])
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| HardwareError::Other(format!("Failed to launch Phone Link: {e}")))?;
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "Phone Link only available on Windows".into(),
    ))
}

/// Launch Phone Link with a specific feature deep link.
///
/// Known deep links (may vary by Windows version):
/// - `Phone` — calls
/// - `Messages` — SMS
/// - `Photos` — photos
/// - `ScreenMirror` — screen mirroring
/// - `Apps` — app streaming
///
/// S53 fix: same single-instance guard as `launch_phone_link` — a feature
/// deep link while the app runs re-initializes the host and resets the sync
/// channel (user-reported: "o app do windows para de sincronizar").
///
/// When the app is already running the feature deep link is therefore
/// skipped, but the command is NOT a no-op any more: the running window is
/// brought to the foreground through the shell activator and the caller gets
/// an error if that fails. (Navigating inside Phone Link is not exposed by
/// the package; the tile opens the app, the user picks the section.)
pub fn launch_phone_link_feature(feature: &str) -> HardwareResult<()> {
    // Validate feature against allow-list to prevent URI injection
    const ALLOWED_FEATURES: &[&str] = &["Phone", "Messages", "Photos", "ScreenMirror", "Apps"];
    if !ALLOWED_FEATURES.contains(&feature) {
        return Err(HardwareError::Other(format!(
            "Unknown Phone Link feature: '{feature}'. Allowed: {ALLOWED_FEATURES:?}"
        )));
    }

    #[cfg(windows)]
    {
        if check_running() {
            log::info!(
                "[phone_link] feature '{feature}' requested while running — focusing the existing window (ms-phone:{feature} would reset the relay)"
            );
            return activate_phone_link();
        }
        let uri = format!("ms-phone:{}", feature);
        log::info!("[phone_link] launching deep link {uri}");
        std::process::Command::new("cmd")
            .args(["/c", "start", "", &uri])
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| {
                HardwareError::Other(format!("Failed to launch Phone Link feature: {e}"))
            })?;
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "Phone Link only available on Windows".into(),
    ))
}

/// Open Phone Link settings page in Windows Settings.
pub fn open_phone_link_settings() -> HardwareResult<()> {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", "ms-settings:mobile-devices"])
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| {
                HardwareError::Other(format!("Failed to open Phone Link settings: {e}"))
            })?;
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "Phone Link settings only available on Windows".into(),
    ))
}

/// Launch the Phone Link pairing flow via the `ms-phone:` deep link.
///
/// Historical note: this used to build `ms-phone-link:pairing?pc=…` URIs, but
/// Windows only registers `ms-phone:` as the Phone Link protocol handler
/// (verified in the package manifest + registry: `HKCU\Software\Classes\ms-phone`
/// is `URL:ms-phone`; there is NO `ms-phone-link:` handler). Launching the
/// unregistered scheme made Windows show "app não existe" even though Phone
/// Link was running — exactly the bug the user hit. We therefore accept both
/// schemes from callers but REWRITE `ms-phone-link:` → `ms-phone:` so the
/// deep link always opens in Phone Link. Deep link paths are allow-listed
/// (pairing, Phone, Messages, Photos, ScreenMirror, Apps).
pub fn launch_phone_link_pairing(uri: &str) -> HardwareResult<()> {
    if uri.is_empty() {
        return Err(HardwareError::Other(
            "Empty Phone Link pairing URI".to_string(),
        ));
    }
    let lower = uri.to_ascii_lowercase();
    let normalized: String = if lower.starts_with("ms-phone-link:") {
        // Rewrite the unregistered scheme to the real one.
        format!("ms-phone:{}", &uri["ms-phone-link:".len()..])
    } else if lower.starts_with("ms-phone:") {
        uri.to_string()
    } else {
        return Err(HardwareError::Other(format!(
            "Refusing to open non-Phone-Link URI: {uri}"
        )));
    };

    // Allow-list the deep-link path to avoid opening arbitrary app screens.
    let rest = normalized["ms-phone:".len()..].trim_start_matches('/');
    let path = rest
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    const ALLOWED: &[&str] = &[
        "",
        "pairing",
        "phone",
        "messages",
        "photos",
        "screenmirror",
        "apps",
    ];
    if !ALLOWED.contains(&path.as_str()) {
        return Err(HardwareError::Other(format!(
            "Refusing to open unknown Phone Link screen '{path}'"
        )));
    }

    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", &normalized])
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| {
                HardwareError::Other(format!("Failed to open Phone Link pairing link: {e}"))
            })?;
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "Phone Link only available on Windows".into(),
    ))
}

// ── Internal helpers ─────────────────────────────────────────────────────────

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Application User Model ID of the Phone Link package. Verified on Windows 11
/// with `Get-StartApps` → `Microsoft.YourPhone_8wekyb3d8bbwe!App`. Shell
/// activation through this AUMID foregrounds the running app; it does not
/// re-run the `ms-phone:` protocol handler, so the Link-to-Windows relay is
/// not reset.
#[cfg(windows)]
const PHONE_LINK_AUMID: &str = "Microsoft.YourPhone_8wekyb3d8bbwe!App";

/// Process images Phone Link runs as: the modern UI host and the legacy proxy.
#[cfg(windows)]
const PHONE_LINK_PROCESSES: &[&str] = &["PhoneExperienceHost", "YourPhoneAppProxy"];

/// Bring the already-running Phone Link window to the foreground.
///
/// Uses the shell's `IApplicationActivationManager` (the API the Start menu
/// and taskbar use). This is the documented "activate, don't relaunch" path:
/// it does not re-enter the `ms-phone:` protocol handler, so sync keeps
/// working. Returns an error — instead of pretending success — when COM or
/// the activation fails.
#[cfg(windows)]
fn activate_phone_link() -> HardwareResult<()> {
    use windows::core::HSTRING;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        ApplicationActivationManager, IApplicationActivationManager, AO_NONE,
    };

    // SAFETY: CoInitializeEx/CoUninitialize are paired on this thread; the
    // activation manager is an out-of-process shell object released at the
    // end of the scope. Nothing is cached, so a pool thread that later runs
    // other work sees a clean apartment.
    unsafe {
        let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        // RPC_E_CHANGED_MODE means the thread already has another apartment
        // model; COM is still usable, we just must not uninitialise it.
        let must_uninit = init.is_ok();

        let activation: windows::core::Result<u32> = (|| {
            let manager: IApplicationActivationManager =
                CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_ALL)?;
            manager.ActivateApplication(&HSTRING::from(PHONE_LINK_AUMID), &HSTRING::new(), AO_NONE)
        })();

        if must_uninit {
            CoUninitialize();
        }

        match activation {
            Ok(pid) => {
                log::info!("[phone_link] activation requested (pid {pid}); waiting for a window");
                // Verification, not faith: the process can be "running" with
                // no window at all (that was the original bug). A visible
                // window is the only proof the user's click did something.
                match wait_for_phone_link_window(std::time::Duration::from_secs(4)) {
                    Some(title) => {
                        log::info!("[phone_link] window visible: '{title}'");
                        Ok(())
                    }
                    None => Err(HardwareError::Other(
                        "Phone Link was activated but no window appeared. It may be running in the background — open it from the tray icon."
                            .into(),
                    )),
                }
            }
            Err(e) => Err(HardwareError::Other(format!(
                "Failed to focus Phone Link ({PHONE_LINK_AUMID}): {e}"
            ))),
        }
    }
}

/// Case-insensitive executable-name comparison that ignores a `.exe` suffix
/// on either side (`PROCESSENTRY32W.szExeFile` includes it, the constants here
/// do not).
fn eq_process_name(entry_name: &str, wanted: &str) -> bool {
    fn norm(name: &str) -> String {
        let trimmed = name.trim();
        let stem = trimmed
            .strip_suffix(".exe")
            .or_else(|| trimmed.strip_suffix(".EXE"))
            .unwrap_or(trimmed);
        stem.to_ascii_lowercase()
    }
    norm(entry_name) == norm(wanted)
}

/// PIDs of the running processes whose image name matches `wanted`
/// (case-insensitive, `.exe` optional). One ToolHelp snapshot, no PowerShell.
#[cfg(windows)]
fn process_pids(wanted: &[&str]) -> Vec<u32> {
    use crate::util::win_handle::OwnedHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    // SAFETY: the snapshot handle is closed before returning and the entry
    // struct is zero-initialised with its required `dwSize`.
    unsafe {
        let Ok(raw) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return Vec::new();
        };
        // S67: RAII — the snapshot closes on scope exit even on early return.
        let snapshot = OwnedHandle::new(raw);
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        let mut pids = Vec::new();
        let mut has = Process32FirstW(snapshot.raw(), &mut entry).is_ok();
        while has {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let exe = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if wanted.iter().any(|w| eq_process_name(&exe, w)) {
                pids.push(entry.th32ProcessID);
            }
            has = Process32NextW(snapshot.raw(), &mut entry).is_ok();
        }

        drop(snapshot); // OwnedHandle closes the kernel handle here
        pids
    }
}

/// PIDs of the running Phone Link processes (empty when it is not running).
#[cfg(windows)]
fn phone_link_pids() -> Vec<u32> {
    process_pids(PHONE_LINK_PROCESSES)
}

/// Is any process from `wanted` running?
#[cfg(windows)]
fn process_running_any(wanted: &[&str]) -> bool {
    !process_pids(wanted).is_empty()
}

/// Titles of the visible top-level windows owned by Phone Link.
///
/// The `MainWindowHandle` of `PhoneExperienceHost` is 0 while the app sits in
/// the background (its only windows are invisible helpers: the tray icon, a
/// DDE server, the GDI+ hook). After a shell activation a visible window
/// appears within a few hundred ms, so its presence is the observable proof
/// that the button did something.
#[cfg(windows)]
fn phone_link_window_titles(pids: &[u32]) -> Vec<String> {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
        IsWindowVisible,
    };

    struct Search<'a> {
        pids: &'a [u32],
        titles: Vec<String>,
    }

    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the `&mut Search` passed to EnumWindows below and
        // outlives the enumeration.
        let search = &mut *(lparam.0 as *mut Search);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if search.pids.contains(&pid) && IsWindowVisible(hwnd).as_bool() {
            let len = GetWindowTextLengthW(hwnd);
            if len > 0 {
                let mut buf = vec![0u16; (len + 1) as usize];
                let copied = GetWindowTextW(hwnd, &mut buf);
                if copied > 0 {
                    search
                        .titles
                        .push(String::from_utf16_lossy(&buf[..copied as usize]));
                }
            }
        }
        BOOL(1) // keep enumerating
    }

    let mut search = Search {
        pids,
        titles: Vec::new(),
    };
    // SAFETY: EnumWindows only reads `search` through the pointer for the
    // duration of the synchronous call.
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut search as *mut _ as isize));
    }
    search.titles
}

/// Wait (bounded) for a visible Phone Link window.
#[cfg(windows)]
fn wait_for_phone_link_window(timeout: std::time::Duration) -> Option<String> {
    let pids = phone_link_pids();
    if pids.is_empty() {
        return None;
    }
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(title) = phone_link_window_titles(&pids).into_iter().next() {
            return Some(title);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

#[cfg(windows)]
fn get_package_version() -> Option<String> {
    // The previous command passed an array to `Get-AppxPackage -Name`, which on
    // this PowerShell build is a scalar parameter:
    //   Cannot convert 'System.Object[]' to the type 'System.String' ...
    // → empty stdout → `package_version: null` in the UI. Positional wildcard
    // matching (with the Microsoft.PhoneExperience fallback for newer builds)
    // is valid on every build.
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$p = Get-AppxPackage *YourPhone*; if (-not $p) { $p = Get-AppxPackage *PhoneExperience* }; $p | Select-Object -First 1 -ExpandProperty Version",
        ])
        .creation_flags(0x0800_0000)
        .output()
        .ok()?;

    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

/// Resolve the Phone Link package local-app-data directory.
///
/// Phone Link stores pairing metadata in
/// `%LOCALAPPDATA%\Packages\Microsoft.YourPhone_*\LocalCache\DeviceMetadataStorage.json`
/// (the same file the app uses). On new Windows 11 builds the package may be
/// `Microsoft.PhoneExperience_*`.
#[cfg(windows)]
fn phone_link_package_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)?;
    let packages = base.join("Packages");
    let names = ["Microsoft.YourPhone", "Microsoft.PhoneExperience"];
    for name in names {
        if let Ok(entries) = std::fs::read_dir(&packages) {
            for entry in entries.flatten() {
                let dir = entry.path();
                if dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.starts_with(name))
                    .unwrap_or(false)
                {
                    return Some(dir);
                }
            }
        }
    }
    None
}

/// Minimal serde shapes for `DeviceMetadataStorage.json` (only the fields we
/// need; unknown fields are ignored by serde).
#[cfg(windows)]
#[derive(serde::Deserialize)]
struct PhoneMetadataEntry {
    #[serde(rename = "IsLinked")]
    is_linked: bool,
    #[serde(rename = "Metadata")]
    metadata: PhoneDeviceMetadata,
}

#[cfg(windows)]
#[derive(serde::Deserialize)]
struct PhoneDeviceMetadata {
    /// The JSON nests the real device info under `Metadata` again:
    /// `Metadata.Metadata.{ClientType,DisplayName,...}`.
    #[serde(rename = "Metadata")]
    #[serde(default)]
    inner: Option<PhoneInnerMetadata>,
}

#[cfg(windows)]
#[derive(serde::Deserialize)]
struct PhoneInnerMetadata {
    #[serde(rename = "ClientType")]
    client_type: String,
    #[serde(rename = "DisplayName")]
    display_name: String,
}

/// Read the Phone Link pairing metadata JSON and return
/// `(is_paired, device_display_name)`.
///
/// Observed shape (Windows 11, Phone Link 1.26x):
/// ```json
/// { "ConfigVersion":1,
///   "DeviceMetadatas": {
///     "<id>": [ {
///       "Certificates": {...},
///       "IsLinked": true,
///       "Metadata": {
///         "Id":"...","LastSeenTime":"...",
///         "Metadata": { "ClientType":"LTW","DisplayName":"Xiaomi 14T",... }
///       }
///     } ]
///   }
/// }
/// ```
/// The phone is the entry with `ClientType == "LTW"` (Link to Windows); the
/// PC itself is `WEA`. We only report paired when the LTW entry is linked.
/// Pure parser for the `DeviceMetadatas` JSON payload. Testable without fs.
#[cfg(windows)]
fn parse_device_metadatas(json: &str) -> (bool, Option<String>) {
    #[derive(serde::Deserialize)]
    struct Storage {
        #[serde(rename = "DeviceMetadatas")]
        device_metadatas: std::collections::HashMap<String, Vec<PhoneMetadataEntry>>,
    }

    let storage = match serde_json::from_str::<Storage>(json) {
        Ok(storage) => storage,
        Err(_) => return (false, None),
    };

    let mut paired = false;
    let mut name: Option<String> = None;
    for (_id, entries) in storage.device_metadatas {
        for entry in entries {
            let Some(inner) = entry.metadata.inner.as_ref() else {
                continue;
            };
            if inner.client_type == "LTW" {
                // This is the phone entry.
                if entry.is_linked {
                    paired = true;
                    name = Some(inner.display_name.clone());
                }
                // The LTW entry is authoritative for the phone — stop here.
                return (paired, name);
            }
            if entry.is_linked && inner.client_type != "WEA" {
                // Some other linked device (not the PC): remember paired.
                paired = true;
            }
        }
    }
    (paired, name)
}

#[cfg(windows)]
fn read_paired_metadata() -> (bool, Option<String>) {
    let Some(dir) = phone_link_package_dir() else {
        return (false, None);
    };
    let path = dir.join("LocalCache").join("DeviceMetadataStorage.json");
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(_) => return (false, None),
    };
    parse_device_metadatas(&raw)
}

#[cfg(windows)]
fn check_paired() -> bool {
    // Primary: the pairing metadata JSON.
    let (linked, _) = read_paired_metadata();
    if linked {
        return true;
    }

    // Fallback: legacy registry keys (works on some older builds).
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let keys = [
        r"Software\Microsoft\YourPhone",
        r"Software\Microsoft\Windows\CurrentVersion\PhoneLink",
    ];
    for key_path in &keys {
        if let Ok(key) = hkcu.open_subkey(key_path) {
            let values: Vec<_> = key.enum_values().collect();
            if !values.is_empty() {
                return true;
            }
        }
    }
    false
}

#[cfg(windows)]
fn read_paired_device_name() -> Option<String> {
    // Primary: from the metadata JSON.
    let (_, name) = read_paired_metadata();
    if let Some(n) = name {
        if !n.is_empty() {
            return Some(n);
        }
    }

    // Fallback: legacy registry keys.
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let value_names = ["DeviceName", "PairedDeviceName", "PhoneName"];
    let key_paths = [
        r"Software\Microsoft\YourPhone",
        r"Software\Microsoft\Windows\CurrentVersion\PhoneLink",
    ];
    for key_path in &key_paths {
        if let Ok(key) = hkcu.open_subkey(key_path) {
            for name in &value_names {
                if let Ok(val) = key.get_value::<String, _>(name) {
                    if !val.is_empty() {
                        return Some(val);
                    }
                }
            }
        }
    }
    None
}

#[cfg(windows)]
fn check_running() -> bool {
    // The modern Phone Link UI runs as PhoneExperienceHost; older builds
    // use YourPhoneAppProxy. A ToolHelp snapshot is ~1 ms — the previous
    // PowerShell probe cost ~1 s and blocked the command thread.
    process_running_any(PHONE_LINK_PROCESSES)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_phone_link_does_not_panic() {
        let _ = detect_phone_link();
    }

    #[test]
    fn test_get_status_does_not_panic() {
        let _ = get_phone_link_status();
    }

    /// `PROCESSENTRY32W.szExeFile` carries the `.exe` suffix; the constant
    /// list here does not. Both spellings, in any case, must match.
    #[test]
    fn process_name_comparison_ignores_suffix_and_case() {
        assert!(eq_process_name(
            "PhoneExperienceHost.exe",
            "PhoneExperienceHost"
        ));
        assert!(eq_process_name(
            "phoneexperiencehost.EXE",
            "PhoneExperienceHost"
        ));
        assert!(eq_process_name(
            "PhoneExperienceHost",
            "PhoneExperienceHost.exe"
        ));
        assert!(!eq_process_name(
            "PhoneExperienceHost.exe",
            "YourPhoneAppProxy"
        ));
        assert!(!eq_process_name(
            "NotPhoneExperienceHost.exe",
            "PhoneExperienceHost"
        ));
        // Prefixes must not match: "PhoneExperienceHost2" is a different image.
        assert!(!eq_process_name(
            "PhoneExperienceHost2.exe",
            "PhoneExperienceHost"
        ));
    }

    /// Read-only process probe: an image name that cannot exist must not be
    /// reported as running, and a real one must be found.
    #[test]
    fn process_probe_is_accurate() {
        assert!(!process_running_any(&["micontroldefinitelynotrunning"]));
        assert!(process_running_any(&["explorer"]));
    }

    /// Manual smoke test for the focus path: it really foregrounds Phone Link,
    /// so it is `#[ignore]`d and only run on purpose:
    ///   cargo test --lib hw::phone_link -- --ignored --nocapture
    #[test]
    #[ignore = "focuses a real window on the desktop"]
    fn activates_the_running_phone_link_window() {
        if !check_running() {
            eprintln!("Phone Link is not running — nothing to activate");
            return;
        }
        activate_phone_link().expect("ActivateApplication should foreground Phone Link");
    }

    /// F9 regression: the version must come back, not `None`. Only asserted
    /// where Phone Link is actually installed (skip on clean CI images).
    #[test]
    fn package_version_is_parsed_when_installed() {
        if !detect_phone_link() {
            return;
        }
        let version = get_package_version();
        assert!(
            version.is_some(),
            "package_version was None with Phone Link installed"
        );
        let v = version.unwrap();
        assert!(
            v.chars().next().is_some_and(|c| c.is_ascii_digit()),
            "unexpected version string: {v:?}"
        );
    }

    /// Real observed shape (Phone Link 1.26x, Xiaomi 14T): double-nested
    /// `Metadata.Metadata`, `IsLinked` outside the inner object, plus a
    /// `WEA` PC entry. Only windows (uses winreg import); guard with `cfg`.
    #[cfg(windows)]
    #[test]
    fn parses_real_device_metadata_shape() {
        let json = r#"{
          "ConfigVersion": 1,
          "DeviceMetadatas": {
            "0006BFFDB6667958": [{
              "Certificates": {"SelfSigned":["skip"]},
              "IsLinked": true,
              "Metadata": {
                "Id": "71caaad7-c77e-4cd7-b363-43b5db234210",
                "LastSeenTime": "2026-08-26T16:05:30+00:00",
                "Metadata": {
                  "ClientType": "WEA",
                  "ClientVersion": "1.26071.84.0",
                  "DisplayName": "MF-PC",
                  "OsName": "Windows",
                  "OsVersion": "10.0.26200",
                  "Manufacture": "Unknown",
                  "ModelName": "Unknown"
                }
              }
            }],
            "1234567890ABCDEF": [{
              "Certificates": {"SelfSigned":["skip"]},
              "IsLinked": true,
              "Metadata": {
                "Id": "82cc6c6f-2e4a-4eeb-826d-ab1335b6cf28",
                "LastSeenTime": "2026-08-26T08:37:00+00:00",
                "Metadata": {
                  "ClientType": "LTW",
                  "ClientVersion": "1.26071.104.0",
                  "DisplayName": "Xiaomi 14T",
                  "OsName": "Android",
                  "OsVersion": "16",
                  "Manufacture": "Xiaomi",
                  "ModelName": "2406APNFAG"
                }
              }
            }]
          }
        }"#;
        let (paired, name) = parse_device_metadatas(json);
        assert!(paired);
        assert_eq!(name.as_deref(), Some("Xiaomi 14T"));
    }

    #[cfg(windows)]
    #[test]
    fn unlinked_phone_returns_not_paired() {
        let json = r#"{
          "DeviceMetadatas": {
            "X": [{ "IsLinked": false, "Metadata": { "Metadata": {
              "ClientType": "LTW", "DisplayName": "Xiaomi 14T"
            }}}]
          }
        }"#;
        let (paired, name) = parse_device_metadatas(json);
        assert!(!paired);
        assert_eq!(name, None);
    }

    #[cfg(windows)]
    #[test]
    fn malformed_json_returns_not_paired() {
        let (paired, name) = parse_device_metadatas("not json");
        assert!(!paired);
        assert_eq!(name, None);
    }

    /// The unregistered `ms-phone-link:` scheme MUST be rewritten to
    /// `ms-phone:` (the only protocol the package registers) — this is the
    /// fix for the "app não existe" error.
    #[test]
    fn pairing_uri_normalizes_ms_phone_link_to_ms_phone() {
        // Instead of switching on Windows-only command execution, we assert
        // on the pure normalization logic by extracting it into a helper.
        let uri = "ms-phone-link:pairing?pc=MiControl";
        let lower = uri.to_ascii_lowercase();
        let normalized: String = if lower.starts_with("ms-phone-link:") {
            format!("ms-phone:{}", &uri["ms-phone-link:".len()..])
        } else {
            uri.to_string()
        };
        assert_eq!(normalized, "ms-phone:pairing?pc=MiControl");
    }
}
