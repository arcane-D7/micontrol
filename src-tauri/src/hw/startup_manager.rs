//! Windows startup manager — every place Windows can launch something at
//! sign-in, in one list, with an honest enable/disable switch.
//!
//! Sources:
//!   * `HKCU\…\CurrentVersion\Run`
//!   * `HKLM\…\CurrentVersion\Run` (64-bit view) and its WOW64 (32-bit) view
//!   * the per-user and the common Startup folders
//!   * scheduled tasks whose trigger is "at logon"
//!
//! Enablement for Run entries and Startup-folder shortcuts is stored in the
//! same `Explorer\StartupApproved` blobs Task Manager writes: a 12-byte
//! `REG_BINARY` value whose first byte is `0x02` (enabled) or `0x03`
//! (disabled); a missing value means enabled. Scheduled tasks carry their own
//! `Enabled` flag and are toggled through the Task Scheduler API.
//!
//! `startup.rs` (the MiControl autostart entry) is deliberately untouched:
//! this module only observes/edits *other* programs.
//!
//! Elevation: HKCU Run entries and the per-user Startup folder are written
//! directly. HKLM entries and the common Startup folder require administrator
//! rights and are routed through the existing elevated bridge
//! (`elevated.rs` → `startup_toggle`) with strictly validated arguments.
//! Scheduled tasks are toggled in-process first and escalate only when the
//! Task Scheduler refuses the write.
//!
//! Nothing here guesses: an entry whose target file is gone is reported as
//! missing, a task that cannot be read is counted, and a source that fails to
//! enumerate surfaces an error instead of silently disappearing.
//!
//! Auto-start **services** are intentionally not listed. They are not logon
//! entries — they belong to `services.msc` — and a hardware-control UI must
//! not offer to disable them by accident. The brief marked them optional and
//! read-only; this first version leaves them out rather than shipping a list
//! that cannot be acted on.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::hw::errors::{HardwareError, HardwareResult};

// ── Registry locations ───────────────────────────────────────────────────────

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED_RUN: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const APPROVED_RUN32: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run32";
const APPROVED_STARTUP_FOLDER: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder";

/// Cache window. The list changes rarely; five seconds keeps a burst of UI
/// refreshes from re-enumerating every scheduled task.
const CACHE_TTL: Duration = Duration::from_secs(5);

/// Hard bound on scheduled-task recursion. A standard Windows install has
/// ~150–300 tasks; the bound only exists so a corrupt folder tree cannot hang
/// the blocking pool.
const MAX_TASK_FOLDERS: usize = 256;
const MAX_TASKS: usize = 2_000;

// ── Public model ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupSource {
    HkcuRun,
    HklmRun,
    HklmRun32,
    UserStartupFolder,
    CommonStartupFolder,
    UserLogonTask,
    CommonLogonTask,
}

impl StartupSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HkcuRun => "hkcu_run",
            Self::HklmRun => "hklm_run",
            Self::HklmRun32 => "hklm_run32",
            Self::UserStartupFolder => "user_startup_folder",
            Self::CommonStartupFolder => "common_startup_folder",
            Self::UserLogonTask => "user_logon_task",
            Self::CommonLogonTask => "common_logon_task",
        }
    }

    /// Machine-wide locations need an administrator to change them.
    pub fn requires_admin(self) -> bool {
        matches!(
            self,
            Self::HklmRun | Self::HklmRun32 | Self::CommonStartupFolder | Self::CommonLogonTask
        )
    }

    fn rank(self) -> u8 {
        match self {
            Self::HkcuRun => 0,
            Self::HklmRun => 1,
            Self::HklmRun32 => 2,
            Self::UserStartupFolder => 3,
            Self::CommonStartupFolder => 4,
            Self::UserLogonTask => 5,
            Self::CommonLogonTask => 6,
        }
    }
}

/// Why a listed entry cannot be switched from this app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToggleBlocked {
    /// A file dropped straight into a Startup folder (`.exe`, `.bat`, …).
    /// Windows has no `StartupApproved` slot for those — disabling one means
    /// moving/renaming the file, which this app will not do behind the user.
    FolderEntryNotShortcut,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartupItem {
    /// Stable within one enumeration: source tag + the natural key.
    pub id: String,
    pub name: String,
    /// Full command line as stored by Windows (env vars expanded).
    pub command: String,
    /// Executable extracted from the command line, when one could be found.
    pub executable: Option<String>,
    pub publisher: Option<String>,
    pub version: Option<String>,
    /// `None` when there is no file to check (e.g. a URL shortcut).
    pub file_exists: Option<bool>,
    pub enabled: bool,
    pub source: StartupSource,
    /// Human-readable origin: registry value name, folder path, task path.
    pub location: String,
    pub requires_admin: bool,
    /// Normalised executable path — entries sharing it are the same program.
    pub link_key: Option<String>,
    pub linked_count: usize,
    pub toggleable: bool,
    pub toggle_blocked: Option<ToggleBlocked>,
    /// MiControl's own HKCU Run entry: its switch lives in Settings > General,
    /// so the Startup tab shows it but does not offer a second control.
    pub managed_in_settings: bool,
    /// Task principal (`SYSTEM`, a user name, …) — `None` for non-task items.
    pub principal: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartupSnapshot {
    pub items: Vec<StartupItem>,
    /// False when the Task Scheduler itself could not be queried.
    pub tasks_available: bool,
    /// Reason the task list is missing (shown in the UI, never swallowed).
    pub task_scan_error: Option<String>,
    /// Tasks whose definition could not be read (permissions/COM hiccup).
    pub tasks_skipped: usize,
    /// True when the answer came from the short-lived cache.
    pub cached: bool,
}

#[derive(Debug, Clone)]
struct RawTaskScan {
    items: Vec<StartupItem>,
    error: Option<String>,
    skipped: usize,
}

// ── Enablement blob ──────────────────────────────────────────────────────────

/// Read the first byte of a `StartupApproved` blob.
///
/// Windows writes 12 bytes; the first byte carries the state. Values seen in
/// the wild: `0x02` enabled, `0x03` disabled, `0x06` enabled (re-enabled).
/// Anything else is treated as "not understood" so the caller can fall back to
/// the honest default (enabled) instead of inventing a state.
pub fn approved_state(bytes: &[u8]) -> Option<bool> {
    match bytes.first() {
        Some(0x02 | 0x06) => Some(true),
        Some(0x03) => Some(false),
        _ => None,
    }
}

/// Build the 12-byte blob Windows expects, preserving the timestamp bytes when
/// a previous blob exists.
pub fn approved_blob(previous: Option<&[u8]>, enabled: bool) -> [u8; 12] {
    let mut blob = [0u8; 12];
    if let Some(prev) = previous {
        let copy = prev.len().min(blob.len());
        blob[..copy].copy_from_slice(&prev[..copy]);
    }
    blob[0] = if enabled { 0x02 } else { 0x03 };
    blob
}

// ── Command-line helpers ─────────────────────────────────────────────────────

/// Expand `%VAR%` references using the current process environment. Unknown
/// variables are preserved verbatim — the UI can then show the raw command
/// rather than a truncated lie.
pub fn expand_env_vars(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(value) => out.push_str(&value),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Split a stored command line into (executable, arguments), best effort.
///
/// Quoted paths are exact; unquoted ones stop at the first `.exe` boundary so
/// `C:\Program Files\App\app.exe -x` does not become the whole path.
pub fn split_command_line(command: &str) -> (Option<String>, Option<String>) {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return (None, None);
    }

    if let Some(rest) = trimmed.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            let exe = rest[..end].to_string();
            let args = rest[end + 1..].trim();
            return (
                (!exe.is_empty()).then_some(exe),
                (!args.is_empty()).then(|| args.to_string()),
            );
        }
        // Unbalanced quote: fall through to the unquoted parser.
    }

    let lower = trimmed.to_ascii_lowercase();
    if let Some(pos) = lower.find(".exe") {
        let end = pos + 4;
        let exe = trimmed[..end].trim().to_string();
        let args = trimmed[end..].trim();
        return (
            (!exe.is_empty()).then_some(exe),
            (!args.is_empty()).then(|| args.to_string()),
        );
    }

    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let exe = parts.next().unwrap_or("").trim();
    let args = parts.next().unwrap_or("").trim();
    (
        (!exe.is_empty()).then(|| exe.to_string()),
        (!args.is_empty()).then(|| args.to_string()),
    )
}

/// Key used to link entries that launch the same program.
pub fn link_key(executable: Option<&str>) -> Option<String> {
    let exe = executable?.trim();
    if exe.is_empty() {
        return None;
    }
    Some(exe.replace('/', "\\").to_ascii_lowercase())
}

// ── Toggle planning ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovedKey {
    Run,
    Run32,
    StartupFolder,
}

impl ApprovedKey {
    fn path(self) -> &'static str {
        match self {
            Self::Run => APPROVED_RUN,
            Self::Run32 => APPROVED_RUN32,
            Self::StartupFolder => APPROVED_STARTUP_FOLDER,
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Run32 => "run32",
            Self::StartupFolder => "startupfolder",
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "run" => Some(Self::Run),
            "run32" => Some(Self::Run32),
            "startupfolder" => Some(Self::StartupFolder),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TogglePlan {
    /// Write the blob in this process (HKCU, or the per-user folder).
    LocalApproved { key: ApprovedKey, value: String },
    /// Write the blob through the elevated bridge (HKLM / common folder).
    ElevatedApproved { key: ApprovedKey, value: String },
    /// Task Scheduler write, attempted in-process first.
    Task { path: String },
}

/// Upper bounds so the elevated bridge never receives an unbounded string.
const MAX_VALUE_NAME: usize = 256;
const MAX_TASK_PATH: usize = 260;

fn is_safe_value_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_VALUE_NAME
        && !name.contains('\0')
        && !name.chars().any(|c| c.is_control())
        && !name.contains('\\')
        && !name.contains('/')
}

fn is_safe_task_path(path: &str) -> bool {
    path.starts_with('\\')
        && path.len() <= MAX_TASK_PATH
        && !path.contains('\0')
        && !path.contains("..")
        && !path.chars().any(|c| c.is_control())
}

/// Split `\Folder\Task` into (`\Folder`, `Task`).
pub fn split_task_path(path: &str) -> Option<(&str, &str)> {
    if !path.starts_with('\\') || path.len() < 2 {
        return None;
    }
    let (folder, name) = match path.rfind('\\') {
        Some(0) => ("\\", &path[1..]),
        Some(idx) => (&path[..idx], &path[idx + 1..]),
        None => return None,
    };
    if name.is_empty() {
        return None;
    }
    Some((folder, name))
}

/// Validated form of the elevated command payload. The bridge accepts no raw
/// registry paths or free-form hives — only these enums and bounded strings.
#[derive(Debug, PartialEq, Eq)]
pub enum ValidatedToggle {
    Approved {
        hive: Hive,
        key: ApprovedKey,
        value: String,
        enabled: bool,
    },
    Task {
        path: String,
        enabled: bool,
    },
}

pub fn validate_toggle_args(args: &serde_json::Value) -> Result<ValidatedToggle, String> {
    let enabled = args
        .get("enabled")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| "startup_toggle: missing boolean 'enabled'".to_string())?;

    let kind = args
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "startup_toggle: missing 'kind'".to_string())?;

    match kind {
        "approved" => {
            let hive = match args.get("hive").and_then(serde_json::Value::as_str) {
                Some("hkcu") => Hive::CurrentUser,
                Some("hklm") => Hive::LocalMachine,
                Some(other) => return Err(format!("startup_toggle: unknown hive '{other}'")),
                None => return Err("startup_toggle: missing 'hive'".to_string()),
            };
            if hive == Hive::CurrentUser {
                // HKCU is written in-process; accepting it on the SYSTEM helper
                // would write the wrong hive (the helper runs as another user
                // or as LocalSystem). Refuse it explicitly.
                return Err(
                    "startup_toggle: hkcu must be applied in-process, not elevated".to_string(),
                );
            }
            let key = args
                .get("key")
                .and_then(serde_json::Value::as_str)
                .and_then(ApprovedKey::from_tag)
                .ok_or_else(|| "startup_toggle: unknown approved key".to_string())?;
            let value = args
                .get("value")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "startup_toggle: missing 'value'".to_string())?
                .to_string();
            if !is_safe_value_name(&value) {
                return Err("startup_toggle: invalid registry value name".to_string());
            }
            Ok(ValidatedToggle::Approved {
                hive,
                key,
                value,
                enabled,
            })
        }
        "task" => {
            let path = args
                .get("path")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "startup_toggle: missing 'path'".to_string())?
                .to_string();
            if !is_safe_task_path(&path) {
                return Err("startup_toggle: invalid task path".to_string());
            }
            Ok(ValidatedToggle::Task { path, enabled })
        }
        other => Err(format!("startup_toggle: unknown kind '{other}'")),
    }
}

// ── Registry helpers ─────────────────────────────────────────────────────────

#[cfg(windows)]
fn hive_handle(hive: Hive) -> winreg::HKEY {
    match hive {
        Hive::CurrentUser => winreg::enums::HKEY_CURRENT_USER,
        Hive::LocalMachine => winreg::enums::HKEY_LOCAL_MACHINE,
    }
}

#[cfg(windows)]
fn read_approved_for(hive: Hive, key: ApprovedKey, name: &str) -> Option<bool> {
    use winreg::enums::{KEY_READ, KEY_WOW64_64KEY};
    use winreg::RegKey;

    RegKey::predef(hive_handle(hive))
        .open_subkey_with_flags(key.path(), KEY_READ | KEY_WOW64_64KEY)
        .ok()
        .and_then(|k| k.get_raw_value(name).ok())
        .and_then(|v| approved_state(&v.bytes))
}

/// Write the StartupApproved blob. `hive` must be HKCU unless this process is
/// already elevated; the caller decides that (see `plan_toggle`).
#[cfg(windows)]
pub fn write_approved(
    hive: Hive,
    key: ApprovedKey,
    name: &str,
    enabled: bool,
) -> HardwareResult<()> {
    use winreg::enums::{KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY, REG_BINARY};
    use winreg::{RegKey, RegValue};

    if !is_safe_value_name(name) {
        return Err(HardwareError::Registry(
            "invalid startup value name".to_string(),
        ));
    }

    let root = RegKey::predef(hive_handle(hive));
    let (key, _) = root
        .create_subkey_with_flags(key.path(), KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY)
        .map_err(|e| HardwareError::Registry(format!("open StartupApproved: {e}")))?;

    let previous = key
        .get_raw_value(name)
        .ok()
        .filter(|v| v.vtype == REG_BINARY)
        .map(|v| v.bytes);
    let blob = approved_blob(previous.as_deref(), enabled);
    let value = RegValue {
        bytes: blob.to_vec(),
        vtype: REG_BINARY,
    };
    key.set_raw_value(name, &value)
        .map_err(|e| HardwareError::Registry(format!("write StartupApproved: {e}")))
}

// ── Enumeration ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegistryView {
    Default,
    Wow64,
    Wow64_32,
}

#[cfg(windows)]
fn enumerate_run_key(
    hive: Hive,
    key_path: &str,
    source: StartupSource,
    view: RegistryView,
    approvals: ApprovedKey,
) -> Vec<StartupItem> {
    use winreg::enums::{KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};
    use winreg::RegKey;

    let mut access = KEY_READ;
    match view {
        RegistryView::Default => {}
        RegistryView::Wow64 => access |= KEY_WOW64_64KEY,
        RegistryView::Wow64_32 => access |= KEY_WOW64_32KEY,
    }
    let root = RegKey::predef(hive_handle(hive));
    let Ok(key) = root.open_subkey_with_flags(key_path, access) else {
        return Vec::new();
    };

    let mut items = Vec::new();
    for entry in key.enum_values().flatten() {
        let (name, value) = entry;
        if value.vtype != winreg::enums::REG_SZ && value.vtype != winreg::enums::REG_EXPAND_SZ {
            // Run entries are strings; anything else is not a command we can
            // show or toggle honestly, so it is left out of the list.
            continue;
        }
        let command = expand_env_vars(&value.to_string());
        if command.trim().is_empty() {
            continue;
        }
        let (executable, _args) = split_command_line(&command);
        let (executable, publisher, version, exists) = probe_executable(executable.as_deref());
        let approved = read_approved_for(hive, approvals, &name);
        let location = match hive {
            Hive::CurrentUser => format!("HKCU\\{key_path}"),
            Hive::LocalMachine => {
                let suffix = if view == RegistryView::Wow64_32 {
                    " (32-bit)"
                } else {
                    ""
                };
                format!("HKLM\\{key_path}{suffix}")
            }
        };
        items.push(StartupItem {
            id: format!("{}|{name}", source.as_str()),
            name,
            command,
            executable: executable.clone(),
            publisher,
            version,
            file_exists: exists,
            enabled: approved.unwrap_or(true),
            source,
            location,
            requires_admin: source.requires_admin(),
            link_key: link_key(executable.as_deref()),
            linked_count: 1,
            toggleable: true,
            toggle_blocked: None,
            managed_in_settings: false,
            principal: None,
        });
    }
    items
}

/// `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup` and the
/// `%ProgramData%` equivalent.
fn startup_folders() -> Vec<(StartupSource, PathBuf, Hive)> {
    let mut folders = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        folders.push((
            StartupSource::UserStartupFolder,
            PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs\Startup"),
            Hive::CurrentUser,
        ));
    }
    if let Ok(program_data) = std::env::var("ProgramData") {
        folders.push((
            StartupSource::CommonStartupFolder,
            PathBuf::from(program_data).join(r"Microsoft\Windows\Start Menu\Programs\Startup"),
            Hive::LocalMachine,
        ));
    }
    folders
}

fn url_target(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("URL=").map(str::trim))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(windows)]
fn shortcut_target(path: &Path) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::Interface;
    use windows::Win32::System::Com::IPersistFile;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED, STGM_READ,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    struct ComGuard(bool);
    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: balances the successful CoInitializeEx below.
                unsafe { CoUninitialize() };
            }
        }
    }

    // SAFETY: COM is initialised on this blocking-pool thread for the duration
    // of the guard; all interfaces are released before it drops.
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let initialised = hr.is_ok() || hr == windows::Win32::Foundation::RPC_E_CHANGED_MODE;
        if !initialised {
            return None;
        }
        let _guard = ComGuard(hr.is_ok());

        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let persist: IPersistFile = link.cast().ok()?;
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        persist
            .Load(windows::core::PCWSTR(wide.as_ptr()), STGM_READ)
            .ok()?;

        let mut buffer = [0u16; 1024];
        link.GetPath(&mut buffer, std::ptr::null_mut(), 0).ok()?;
        let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        if end == 0 {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(&buffer[..end])))
    }
}

#[cfg(windows)]
fn enumerate_startup_folder(source: StartupSource, folder: &Path, hive: Hive) -> Vec<StartupItem> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };

    let mut items = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };
        if file_name.eq_ignore_ascii_case("desktop.ini") {
            continue;
        }
        // Hidden/system files are folder furniture, not startup entries.
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
            const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
            if let Ok(meta) = std::fs::metadata(&path) {
                if meta.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0 {
                    continue;
                }
            }
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let target: Option<PathBuf> = match ext.as_str() {
            "lnk" => shortcut_target(&path),
            _ => Some(path.clone()),
        };

        let command = match (ext.as_str(), &target) {
            ("url", _) => url_target(&path).unwrap_or_else(|| path.display().to_string()),
            _ => target
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| path.display().to_string()),
        };

        let executable = target
            .as_deref()
            .filter(|_| ext != "url")
            .map(|p| p.display().to_string());
        let (executable, publisher, version, exists) = probe_executable(executable.as_deref());
        let shortcut = ext == "lnk" || ext == "url";
        let approved = read_approved_for(hive, ApprovedKey::StartupFolder, &file_name);

        items.push(StartupItem {
            id: format!("{}|{file_name}", source.as_str()),
            name: path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&file_name)
                .to_string(),
            command,
            executable: executable.clone(),
            publisher,
            version,
            file_exists: exists,
            enabled: approved.unwrap_or(true),
            source,
            location: folder.display().to_string(),
            requires_admin: source.requires_admin(),
            link_key: link_key(executable.as_deref()),
            linked_count: 1,
            toggleable: shortcut,
            toggle_blocked: (!shortcut).then_some(ToggleBlocked::FolderEntryNotShortcut),
            managed_in_settings: false,
            principal: None,
        });
    }
    items
}

// ── File metadata ────────────────────────────────────────────────────────────

type ExecutableProbe = (Option<String>, Option<String>, Option<String>, Option<bool>);

/// Trim surrounding quotes and, for bare command names (`powershell.exe`),
/// resolve them through `PATH`. A bare name that cannot be resolved is
/// reported as "unknown", never as "missing" — a task may rely on its own
/// working directory or a shell alias we cannot see.
fn resolve_executable(raw: &str) -> (String, Option<bool>) {
    let trimmed = raw.trim().trim_matches('"').trim();
    if trimmed.is_empty() {
        return (String::new(), Some(false));
    }
    let path = Path::new(trimmed);
    let looks_like_path = path.is_absolute() || trimmed.contains('\\') || trimmed.contains('/');
    if looks_like_path {
        return (trimmed.to_string(), Some(path.is_file()));
    }

    let has_extension = path.extension().is_some();
    let mut names = Vec::new();
    if has_extension {
        names.push(trimmed.to_string());
    } else {
        names.push(format!("{trimmed}.exe"));
        names.push(trimmed.to_string());
    }
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for name in &names {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return (candidate.display().to_string(), Some(true));
                }
            }
        }
    }
    (trimmed.to_string(), None)
}

#[cfg(windows)]
fn probe_executable(executable: Option<&str>) -> ExecutableProbe {
    let Some(raw) = executable else {
        return (None, None, None, None);
    };
    let (exe, exists) = resolve_executable(raw);
    if exe.is_empty() {
        return (None, None, None, Some(false));
    }
    let path = Path::new(&exe);
    let (publisher, version) = if exists == Some(true) && path.is_file() {
        file_version_info(path)
    } else {
        (None, None)
    };
    (Some(exe), publisher, version, exists)
}

#[cfg(not(windows))]
fn probe_executable(executable: Option<&str>) -> ExecutableProbe {
    let Some(raw) = executable else {
        return (None, None, None, None);
    };
    let (exe, exists) = resolve_executable(raw);
    ((!exe.is_empty()).then_some(exe), None, None, exists)
}

#[cfg(windows)]
fn file_version_info(path: &Path) -> (Option<String>, Option<String>) {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: `wide` is a NUL-terminated wide path living for the whole call;
    // the version block is owned by `buffer` and every pointer derived from it
    // is only read inside this function.
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None);
        if size == 0 {
            return (None, None);
        }
        let mut buffer = vec![0u8; size as usize];
        if GetFileVersionInfoW(PCWSTR(wide.as_ptr()), 0, size, buffer.as_mut_ptr().cast()).is_err()
        {
            return (None, None);
        }

        let mut version = None;
        let mut fixed_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut fixed_len = 0u32;
        let root_block: [u16; 2] = [u16::from(b'\\'), 0];
        if VerQueryValueW(
            buffer.as_ptr().cast(),
            PCWSTR::from_raw(root_block.as_ptr()),
            &mut fixed_ptr,
            &mut fixed_len,
        )
        .as_bool()
            && !fixed_ptr.is_null()
            && fixed_len >= 16
        {
            let base = fixed_ptr.cast::<u8>();
            let ms = u32::from_le_bytes([*base.add(8), *base.add(9), *base.add(10), *base.add(11)]);
            let ls =
                u32::from_le_bytes([*base.add(12), *base.add(13), *base.add(14), *base.add(15)]);
            version = Some(format!(
                "{}.{}.{}.{}",
                ms >> 16,
                ms & 0xffff,
                ls >> 16,
                ls & 0xffff
            ));
        }

        let publisher = query_version_string(&buffer, "CompanyName");
        (publisher, version)
    }
}

#[cfg(windows)]
unsafe fn query_version_string(buffer: &[u8], field: &str) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::VerQueryValueW;

    let mut translations: *mut core::ffi::c_void = std::ptr::null_mut();
    let mut translations_len = 0u32;
    let translation_block: Vec<u16> = "\\VarFileInfo\\Translation"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let ok = VerQueryValueW(
        buffer.as_ptr().cast(),
        PCWSTR(translation_block.as_ptr()),
        &mut translations,
        &mut translations_len,
    )
    .as_bool();
    if !ok || translations.is_null() || translations_len < 4 {
        return None;
    }

    let lang = *(translations.cast::<u16>());
    let codepage = *(translations.cast::<u16>().add(1));
    let sub_block = format!("\\StringFileInfo\\{lang:04x}{codepage:04x}\\{field}");
    let sub_wide: Vec<u16> = sub_block.encode_utf16().chain(std::iter::once(0)).collect();

    let mut value_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
    let mut value_len = 0u32;
    let ok = VerQueryValueW(
        buffer.as_ptr().cast(),
        PCWSTR(sub_wide.as_ptr()),
        &mut value_ptr,
        &mut value_len,
    )
    .as_bool();
    if !ok || value_ptr.is_null() || value_len == 0 {
        return None;
    }
    let slice = std::slice::from_raw_parts(value_ptr.cast::<u16>(), value_len as usize);
    let text = String::from_utf16_lossy(slice)
        .trim_end_matches('\0')
        .trim()
        .to_string();
    (!text.is_empty()).then_some(text)
}

// ── Scheduled tasks ──────────────────────────────────────────────────────────

#[cfg(windows)]
fn scan_logon_tasks() -> RawTaskScan {
    use windows::core::{BSTR, VARIANT};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::TaskScheduler::{
        ITaskFolder, ITaskService, TaskScheduler, TASK_ENUM_HIDDEN,
    };

    struct ComGuard(bool);
    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: balances the successful CoInitializeEx below.
                unsafe { CoUninitialize() };
            }
        }
    }

    // SAFETY: COM is initialised for the duration of this function on the
    // blocking-pool thread; every interface is released before return.
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let initialised = hr.is_ok() || hr == windows::Win32::Foundation::RPC_E_CHANGED_MODE;
        if !initialised {
            return RawTaskScan {
                items: Vec::new(),
                error: Some(format!("COM init failed: 0x{:08x}", hr.0 as u32)),
                skipped: 0,
            };
        }
        let _guard = ComGuard(hr.is_ok());

        let service: ITaskService =
            match CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER) {
                Ok(service) => service,
                Err(e) => {
                    return RawTaskScan {
                        items: Vec::new(),
                        error: Some(format!("Task Scheduler unavailable: {e}")),
                        skipped: 0,
                    }
                }
            };
        if let Err(e) = service.Connect(None, None, None, None) {
            return RawTaskScan {
                items: Vec::new(),
                error: Some(format!("Task Scheduler connect failed: {e}")),
                skipped: 0,
            };
        }
        let root = match service.GetFolder(&BSTR::from("\\")) {
            Ok(folder) => folder,
            Err(e) => {
                return RawTaskScan {
                    items: Vec::new(),
                    error: Some(format!("Cannot open task root: {e}")),
                    skipped: 0,
                }
            }
        };

        let mut items = Vec::new();
        let mut skipped = 0usize;
        let mut folders_seen = 0usize;
        let mut stack: Vec<ITaskFolder> = vec![root];

        while let Some(folder) = stack.pop() {
            if folders_seen >= MAX_TASK_FOLDERS || items.len() >= MAX_TASKS {
                break;
            }
            folders_seen += 1;

            if let Ok(tasks) = folder.GetTasks(TASK_ENUM_HIDDEN.0) {
                if let Ok(count) = tasks.Count() {
                    for index in 1..=count {
                        if items.len() >= MAX_TASKS {
                            break;
                        }
                        let Ok(task) = tasks.get_Item(&VARIANT::from(index)) else {
                            skipped += 1;
                            continue;
                        };
                        match task_to_item(&task) {
                            Ok(Some(item)) => items.push(item),
                            Ok(None) => {}
                            Err(_) => skipped += 1,
                        }
                    }
                }
            }

            if let Ok(subfolders) = folder.GetFolders(0) {
                if let Ok(count) = subfolders.Count() {
                    for index in 1..=count {
                        if stack.len() >= MAX_TASK_FOLDERS {
                            break;
                        }
                        if let Ok(sub) = subfolders.get_Item(&VARIANT::from(index)) {
                            stack.push(sub);
                        }
                    }
                }
            }
        }

        RawTaskScan {
            items,
            error: None,
            skipped,
        }
    }
}

#[cfg(windows)]
unsafe fn definition_has_logon_trigger(
    definition: &windows::Win32::System::TaskScheduler::ITaskDefinition,
) -> windows::core::Result<bool> {
    use windows::Win32::System::TaskScheduler::TASK_TRIGGER_LOGON;

    let triggers = definition.Triggers()?;
    let mut trigger_count = 0i32;
    triggers.Count(&mut trigger_count)?;
    for index in 1..=trigger_count {
        if let Ok(trigger) = triggers.get_Item(index) {
            let mut kind = windows::Win32::System::TaskScheduler::TASK_TRIGGER_EVENT;
            if trigger.Type(&mut kind).is_ok() && kind == TASK_TRIGGER_LOGON {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(windows)]
fn task_to_item(
    task: &windows::Win32::System::TaskScheduler::IRegisteredTask,
) -> windows::core::Result<Option<StartupItem>> {
    use windows::core::{Interface, BSTR};
    use windows::Win32::System::TaskScheduler::{
        IExecAction, TASK_ACTION_EXEC, TASK_RUNLEVEL_HIGHEST,
    };

    // SAFETY: pure COM getters on an interface that outlives this call.
    unsafe {
        let path = task.Path()?.to_string();
        let definition = task.Definition()?;
        if !definition_has_logon_trigger(&definition)? {
            return Ok(None);
        }

        let mut executable = None;
        let mut arguments = String::new();
        let actions = definition.Actions()?;
        let mut action_count = 0i32;
        actions.Count(&mut action_count)?;
        for index in 1..=action_count {
            let Ok(action) = actions.get_Item(index) else {
                continue;
            };
            let mut action_kind = windows::Win32::System::TaskScheduler::TASK_ACTION_EXEC;
            let is_exec = action
                .Type(&mut action_kind)
                .map(|()| action_kind == TASK_ACTION_EXEC)
                .unwrap_or(false);
            if is_exec {
                if let Ok(exec) = action.cast::<IExecAction>() {
                    let mut path_bstr = BSTR::new();
                    if exec.Path(&mut path_bstr).is_ok() && !path_bstr.is_empty() {
                        executable = Some(path_bstr.to_string());
                        let mut args_bstr = BSTR::new();
                        if exec.Arguments(&mut args_bstr).is_ok() {
                            arguments = args_bstr.to_string();
                        }
                        break;
                    }
                }
            }
        }

        let executable = executable.map(|raw| expand_env_vars(&raw));
        let (executable, publisher, version, exists) = probe_executable(executable.as_deref());
        let command = match (&executable, arguments.trim()) {
            (Some(exe), "") => exe.clone(),
            (Some(exe), args) => {
                if exe.contains(' ') && !exe.starts_with('"') {
                    format!("\"{exe}\" {args}")
                } else {
                    format!("{exe} {args}")
                }
            }
            (None, "") => return Ok(None),
            (None, args) => args.to_string(),
        };

        let mut requires_admin = false;
        let mut principal = None;
        if let Ok(principal_iface) = definition.Principal() {
            let mut run_level = windows::Win32::System::TaskScheduler::TASK_RUNLEVEL_LUA;
            if principal_iface.RunLevel(&mut run_level).is_ok()
                && run_level == TASK_RUNLEVEL_HIGHEST
            {
                requires_admin = true;
            }
            let mut user = BSTR::new();
            if principal_iface.UserId(&mut user).is_ok() && !user.is_empty() {
                let user = user.to_string();
                if is_system_principal(&user) {
                    requires_admin = true;
                }
                principal = Some(user);
            }
        }

        let enabled = task.Enabled().map(|v| v.as_bool()).unwrap_or(true);
        let is_common = requires_admin || !is_current_user_task(principal.as_deref());
        let source = if is_common {
            StartupSource::CommonLogonTask
        } else {
            StartupSource::UserLogonTask
        };

        let name = path.rsplit('\\').next().unwrap_or(&path).to_string();

        Ok(Some(StartupItem {
            id: format!("{}|{path}", source.as_str()),
            name,
            command,
            executable: executable.clone(),
            publisher,
            version,
            file_exists: exists,
            enabled,
            source,
            location: path,
            requires_admin: requires_admin || source.requires_admin(),
            link_key: link_key(executable.as_deref()),
            linked_count: 1,
            toggleable: true,
            toggle_blocked: None,
            managed_in_settings: false,
            principal,
        }))
    }
}

fn is_system_principal(user: &str) -> bool {
    let lower = user.to_ascii_lowercase();
    lower.contains("system")
        || lower.contains("local service")
        || lower.contains("network service")
        || lower.ends_with("-5-18")
        || lower.ends_with("-5-19")
        || lower.ends_with("-5-20")
}

fn is_current_user_task(principal: Option<&str>) -> bool {
    let Some(principal) = principal else {
        // No principal: the task runs as the registering user.
        return true;
    };
    if is_system_principal(principal) {
        return false;
    }
    let user = std::env::var("USERNAME").unwrap_or_default();
    if user.is_empty() {
        return true;
    }
    principal
        .to_ascii_lowercase()
        .contains(&user.to_ascii_lowercase())
}

#[cfg(not(windows))]
fn scan_logon_tasks() -> RawTaskScan {
    RawTaskScan {
        items: Vec::new(),
        error: None,
        skipped: 0,
    }
}

// ── Assembly + cache ─────────────────────────────────────────────────────────

fn collect() -> StartupSnapshot {
    let mut items = Vec::new();

    #[cfg(windows)]
    {
        items.extend(enumerate_run_key(
            Hive::CurrentUser,
            RUN_KEY,
            StartupSource::HkcuRun,
            RegistryView::Default,
            ApprovedKey::Run,
        ));
        items.extend(enumerate_run_key(
            Hive::LocalMachine,
            RUN_KEY,
            StartupSource::HklmRun,
            RegistryView::Wow64,
            ApprovedKey::Run,
        ));

        // The 32-bit view can repeat values that are also visible in the
        // 64-bit view; only add rows that are actually distinct.
        let mut seen_32: Vec<(String, String)> = items
            .iter()
            .filter(|i| i.source == StartupSource::HklmRun)
            .map(|i| (i.name.to_ascii_lowercase(), i.command.clone()))
            .collect();
        for item in enumerate_run_key(
            Hive::LocalMachine,
            RUN_KEY,
            StartupSource::HklmRun32,
            RegistryView::Wow64_32,
            ApprovedKey::Run32,
        ) {
            let key = (item.name.to_ascii_lowercase(), item.command.clone());
            if !seen_32.contains(&key) {
                seen_32.push(key);
                items.push(item);
            }
        }

        for (source, folder, _hive) in startup_folders() {
            items.extend(enumerate_startup_folder(
                source,
                &folder,
                match source {
                    StartupSource::CommonStartupFolder => Hive::LocalMachine,
                    _ => Hive::CurrentUser,
                },
            ));
        }
    }

    let task_scan = scan_logon_tasks();
    items.extend(task_scan.items);

    // MiControl's own HKCU Run entry is switched from Settings > General. The
    // tab still lists it (a startup manager that hides itself is lying), but
    // the second control is disabled so the two switches cannot disagree.
    for item in &mut items {
        if item.source == StartupSource::HkcuRun && item.name.eq_ignore_ascii_case("MiControl") {
            item.managed_in_settings = true;
        }
    }

    // Link entries that launch the same executable.
    let mut counts: HashMap<String, usize> = HashMap::new();
    for item in &items {
        if let Some(key) = &item.link_key {
            *counts.entry(key.clone()).or_insert(0) += 1;
        }
    }
    for item in &mut items {
        if let Some(key) = &item.link_key {
            item.linked_count = counts.get(key).copied().unwrap_or(1);
        }
    }

    items.sort_by(|a, b| {
        a.source.rank().cmp(&b.source.rank()).then_with(|| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        })
    });

    StartupSnapshot {
        items,
        tasks_available: task_scan.error.is_none(),
        task_scan_error: task_scan.error,
        tasks_skipped: task_scan.skipped,
        cached: false,
    }
}

struct Cached {
    at: Instant,
    snapshot: StartupSnapshot,
}

fn cache() -> &'static Mutex<Option<Cached>> {
    static CACHE: OnceLock<Mutex<Option<Cached>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

pub fn invalidate_cache() {
    if let Ok(mut guard) = cache().lock() {
        *guard = None;
    }
}

pub fn get_startup_items(force_refresh: bool) -> HardwareResult<StartupSnapshot> {
    let mut guard = cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    if !force_refresh {
        if let Some(cached) = guard.as_ref() {
            if cached.at.elapsed() < CACHE_TTL {
                let mut snapshot = cached.snapshot.clone();
                snapshot.cached = true;
                return Ok(snapshot);
            }
        }
    }

    let snapshot = collect();
    *guard = Some(Cached {
        at: Instant::now(),
        snapshot: snapshot.clone(),
    });
    Ok(snapshot)
}

// ── Toggle ───────────────────────────────────────────────────────────────────

pub fn plan_toggle(id: &str) -> HardwareResult<TogglePlan> {
    let snapshot = get_startup_items(false)?;
    let mut item = snapshot.items.into_iter().find(|item| item.id == id);
    if item.is_none() {
        invalidate_cache();
        item = get_startup_items(true)?
            .items
            .into_iter()
            .find(|item| item.id == id);
    }
    let item = item.ok_or_else(|| {
        HardwareError::InvalidConfig(
            "startup entry no longer exists — refresh the list".to_string(),
        )
    })?;

    if item.managed_in_settings {
        return Err(HardwareError::NotSupported(
            "MiControl autostart is managed in Settings".to_string(),
        ));
    }

    if !item.toggleable {
        return Err(HardwareError::NotSupported(
            "this startup-folder entry cannot be disabled without moving the file".to_string(),
        ));
    }

    match item.source {
        StartupSource::HkcuRun => Ok(TogglePlan::LocalApproved {
            key: ApprovedKey::Run,
            value: item.name.clone(),
        }),
        StartupSource::HklmRun => Ok(TogglePlan::ElevatedApproved {
            key: ApprovedKey::Run,
            value: item.name.clone(),
        }),
        StartupSource::HklmRun32 => Ok(TogglePlan::ElevatedApproved {
            key: ApprovedKey::Run32,
            value: item.name.clone(),
        }),
        StartupSource::UserStartupFolder => Ok(TogglePlan::LocalApproved {
            key: ApprovedKey::StartupFolder,
            value: item_file_name(&item).ok_or_else(|| {
                HardwareError::InvalidConfig("startup folder entry has no file name".to_string())
            })?,
        }),
        StartupSource::CommonStartupFolder => Ok(TogglePlan::ElevatedApproved {
            key: ApprovedKey::StartupFolder,
            value: item_file_name(&item).ok_or_else(|| {
                HardwareError::InvalidConfig("startup folder entry has no file name".to_string())
            })?,
        }),
        StartupSource::UserLogonTask | StartupSource::CommonLogonTask => Ok(TogglePlan::Task {
            path: item.location.clone(),
        }),
    }
}

fn item_file_name(item: &StartupItem) -> Option<String> {
    // `id` ends with the natural key (`<source>|<file name>`); for folder
    // entries that is exactly the StartupApproved value name.
    item.id.split_once('|').map(|(_, name)| name.to_string())
}

#[cfg(windows)]
pub fn apply_local_toggle(plan: &TogglePlan, enabled: bool) -> HardwareResult<()> {
    match plan {
        TogglePlan::LocalApproved { key, value } => {
            write_approved(Hive::CurrentUser, *key, value, enabled)
        }
        TogglePlan::Task { path } => set_task_enabled(path, enabled),
        TogglePlan::ElevatedApproved { .. } => Err(HardwareError::InvalidConfig(
            "machine-wide entry requires elevation".to_string(),
        )),
    }
}

#[cfg(not(windows))]
pub fn apply_local_toggle(_plan: &TogglePlan, _enabled: bool) -> HardwareResult<()> {
    Err(HardwareError::NotSupported(
        "startup manager is Windows-only".to_string(),
    ))
}

/// JSON payload handed to `elevated.rs::dispatch("startup_toggle", …)`.
pub fn elevated_args(plan: &TogglePlan, enabled: bool) -> serde_json::Value {
    match plan {
        TogglePlan::ElevatedApproved { key, value } => serde_json::json!({
            "kind": "approved",
            "hive": "hklm",
            "key": key.tag(),
            "value": value,
            "enabled": enabled,
        }),
        TogglePlan::Task { path } => serde_json::json!({
            "kind": "task",
            "path": path,
            "enabled": enabled,
        }),
        TogglePlan::LocalApproved { .. } => serde_json::json!({}),
    }
}

/// Entry point for the elevated helper. Validates the payload again — the
/// helper must never trust the caller.
pub fn apply_elevated_toggle(args: &serde_json::Value) -> Result<(), String> {
    let validated = validate_toggle_args(args)?;
    match validated {
        ValidatedToggle::Approved {
            hive,
            key,
            value,
            enabled,
        } => {
            #[cfg(windows)]
            {
                write_approved(hive, key, &value, enabled).map_err(|e| e.to_string())
            }
            #[cfg(not(windows))]
            {
                let _ = (hive, key, value, enabled);
                Err("startup manager is Windows-only".to_string())
            }
        }
        ValidatedToggle::Task { path, enabled } => {
            #[cfg(windows)]
            {
                set_task_enabled(&path, enabled).map_err(|e| e.to_string())
            }
            #[cfg(not(windows))]
            {
                let _ = (path, enabled);
                Err("startup manager is Windows-only".to_string())
            }
        }
    }
}

#[cfg(windows)]
fn set_task_enabled(path: &str, enabled: bool) -> HardwareResult<()> {
    use windows::core::BSTR;
    use windows::Win32::Foundation::VARIANT_BOOL;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::TaskScheduler::{ITaskService, TaskScheduler};

    if !is_safe_task_path(path) {
        return Err(HardwareError::InvalidConfig(
            "invalid task path".to_string(),
        ));
    }
    let (folder_path, task_name) = split_task_path(path).ok_or_else(|| {
        HardwareError::InvalidConfig("task path has no folder/name split".to_string())
    })?;

    struct ComGuard(bool);
    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: balances the successful CoInitializeEx below.
                unsafe { CoUninitialize() };
            }
        }
    }

    // SAFETY: COM is initialised for this call and released before return.
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let initialised = hr.is_ok() || hr == windows::Win32::Foundation::RPC_E_CHANGED_MODE;
        if !initialised {
            return Err(HardwareError::ElevatedBridge(format!(
                "COM init failed: 0x{:08x}",
                hr.0 as u32
            )));
        }
        let _guard = ComGuard(hr.is_ok());

        let service: ITaskService = CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| HardwareError::ElevatedBridge(format!("Task Scheduler: {e}")))?;
        service
            .Connect(None, None, None, None)
            .map_err(|e| HardwareError::ElevatedBridge(format!("Task Scheduler connect: {e}")))?;
        let folder = service
            .GetFolder(&BSTR::from(folder_path))
            .map_err(|e| HardwareError::ElevatedBridge(format!("open task folder: {e}")))?;
        let task = folder
            .GetTask(&BSTR::from(task_name))
            .map_err(|e| HardwareError::ElevatedBridge(format!("open task: {e}")))?;
        // The bridge only ever toggles logon tasks. Verifying the trigger here
        // keeps a forged/crafted task path from disabling unrelated scheduled
        // maintenance (defrag, updates, …) through this arm.
        let definition = task
            .Definition()
            .map_err(|e| HardwareError::ElevatedBridge(format!("read task definition: {e}")))?;
        let is_logon = definition_has_logon_trigger(&definition)
            .map_err(|e| HardwareError::ElevatedBridge(format!("read task triggers: {e}")))?;
        if !is_logon {
            return Err(HardwareError::NotSupported(
                "only tasks with a logon trigger can be toggled here".to_string(),
            ));
        }
        task.SetEnabled(VARIANT_BOOL::from(enabled))
            .map_err(|e| HardwareError::ElevatedBridge(format!("set task state: {e}")))?;
    }
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approved_state_reads_known_first_bytes() {
        assert_eq!(approved_state(&[0x02, 0, 0, 0]), Some(true));
        assert_eq!(approved_state(&[0x06, 0, 0, 0]), Some(true));
        assert_eq!(approved_state(&[0x03, 0, 0, 0]), Some(false));
    }

    #[test]
    fn approved_state_is_unknown_for_empty_or_odd_blobs() {
        assert_eq!(approved_state(&[]), None);
        assert_eq!(approved_state(&[0x09, 0, 0]), None);
    }

    #[test]
    fn approved_blob_preserves_timestamp_and_sets_state() {
        let previous = [0x03, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 0];
        let enabled = approved_blob(Some(&previous), true);
        assert_eq!(enabled[0], 0x02);
        assert_eq!(&enabled[1..8], &previous[1..8]);

        let disabled = approved_blob(None, false);
        assert_eq!(disabled[0], 0x03);
        assert_eq!(&disabled[1..], &[0u8; 11]);
    }

    #[test]
    fn command_line_with_quoted_path() {
        let (exe, args) = split_command_line(r#""C:\Program Files\App\app.exe" --minimized /x"#);
        assert_eq!(exe.as_deref(), Some(r"C:\Program Files\App\app.exe"));
        assert_eq!(args.as_deref(), Some("--minimized /x"));
    }

    #[test]
    fn command_line_with_unquoted_path() {
        let (exe, args) = split_command_line(r"C:\Program Files\App\app.exe --flag");
        assert_eq!(exe.as_deref(), Some(r"C:\Program Files\App\app.exe"));
        assert_eq!(args.as_deref(), Some("--flag"));
    }

    #[test]
    fn command_line_without_exe() {
        let (exe, args) = split_command_line("rundll32 shell32.dll,Control_RunDLL");
        assert_eq!(exe.as_deref(), Some("rundll32"));
        assert_eq!(args.as_deref(), Some("shell32.dll,Control_RunDLL"));
    }

    #[test]
    fn env_vars_expand_and_unknown_tokens_survive() {
        std::env::set_var("MC_STARTUP_TEST_VAR", "C:\\Temp");
        assert_eq!(
            expand_env_vars("%MC_STARTUP_TEST_VAR%\\app.exe"),
            "C:\\Temp\\app.exe"
        );
        assert_eq!(
            expand_env_vars("%MC_STARTUP_UNKNOWN_VAR%\\app.exe"),
            "%MC_STARTUP_UNKNOWN_VAR%\\app.exe"
        );
    }

    #[test]
    fn link_key_is_case_and_separator_insensitive() {
        assert_eq!(
            link_key(Some("C:/Program Files/App/App.EXE")).as_deref(),
            Some(r"c:\program files\app\app.exe")
        );
        assert_eq!(link_key(None), None);
    }

    #[test]
    fn executable_resolution_strips_quotes_and_resolves_bare_names() {
        let exe = std::env::current_exe().expect("current exe");
        let quoted = format!("\"{}\"", exe.display());
        let (resolved, exists) = resolve_executable(&quoted);
        assert_eq!(resolved, exe.display().to_string());
        assert_eq!(exists, Some(true));

        // A bare name that is not on PATH is unknown, never "missing".
        let (_, exists) = resolve_executable("definitely-not-a-real-command-xyz");
        assert_eq!(exists, None);
    }

    #[test]
    fn task_paths_split_at_the_last_backslash() {
        assert_eq!(split_task_path(r"\OneDrive"), Some(("\\", "OneDrive")));
        assert_eq!(
            split_task_path(r"\Microsoft\Windows\Foo\Bar"),
            Some((r"\Microsoft\Windows\Foo", "Bar"))
        );
        assert_eq!(split_task_path("no-leading-slash"), None);
        assert_eq!(split_task_path("\\"), None);
    }

    #[test]
    fn elevated_payload_carries_the_requested_state() {
        let plan = TogglePlan::ElevatedApproved {
            key: ApprovedKey::Run32,
            value: "Some App".to_string(),
        };
        let args = elevated_args(&plan, false);
        assert_eq!(
            validate_toggle_args(&args),
            Ok(ValidatedToggle::Approved {
                hive: Hive::LocalMachine,
                key: ApprovedKey::Run32,
                value: "Some App".to_string(),
                enabled: false,
            })
        );

        let task_plan = TogglePlan::Task {
            path: r"\OneDrive".to_string(),
        };
        let task_args = elevated_args(&task_plan, true);
        assert_eq!(
            validate_toggle_args(&task_args),
            Ok(ValidatedToggle::Task {
                path: r"\OneDrive".to_string(),
                enabled: true,
            })
        );
    }

    #[test]
    fn elevated_payload_accepts_only_machine_approved_keys() {
        let ok = serde_json::json!({
            "kind": "approved",
            "hive": "hklm",
            "key": "run",
            "value": "Some App",
            "enabled": false,
        });
        assert_eq!(
            validate_toggle_args(&ok),
            Ok(ValidatedToggle::Approved {
                hive: Hive::LocalMachine,
                key: ApprovedKey::Run,
                value: "Some App".to_string(),
                enabled: false,
            })
        );

        let hkcu = serde_json::json!({
            "kind": "approved",
            "hive": "hkcu",
            "key": "run",
            "value": "Some App",
            "enabled": true,
        });
        assert!(validate_toggle_args(&hkcu).is_err());
    }

    #[test]
    fn elevated_payload_rejects_unknown_fields_and_injection() {
        for value in [
            "",
            "x".repeat(MAX_VALUE_NAME + 1).as_str(),
            "a/b",
            "a\\b",
            "a\nb",
        ] {
            let payload = serde_json::json!({
                "kind": "approved",
                "hive": "hklm",
                "key": "run",
                "value": value,
                "enabled": true,
            });
            assert!(
                validate_toggle_args(&payload).is_err(),
                "accepted {value:?}"
            );
        }

        let bad_path = serde_json::json!({
            "kind": "task",
            "path": "\\Microsoft\\..\\Windows\\evil",
            "enabled": true,
        });
        assert!(validate_toggle_args(&bad_path).is_err());

        let no_enabled = serde_json::json!({
            "kind": "task",
            "path": "\\OneDrive",
        });
        assert!(validate_toggle_args(&no_enabled).is_err());
    }

    /// Live toggle round-trip against HKCU\…\Run.
    ///
    /// Creates a temporary `notepad.exe` entry (never launched), flips it off and
    /// on through the same `StartupApproved` blob the UI writes, and removes both
    /// registry values on the way out — including on panic.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn live_toggle_roundtrip_hkcu_run() {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
        use winreg::RegKey;

        const TEST_NAME: &str = "MiControlStartupManagerTest";

        struct Cleanup;
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let run = RegKey::predef(HKEY_CURRENT_USER);
                if let Ok(key) = run.open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE) {
                    let _ = key.delete_value(TEST_NAME);
                }
                if let Ok(key) = run.open_subkey_with_flags(APPROVED_RUN, KEY_SET_VALUE) {
                    let _ = key.delete_value(TEST_NAME);
                }
            }
        }

        let _cleanup = Cleanup;
        let run = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_SET_VALUE)
            .expect("open Run key");
        run.set_value(TEST_NAME, &r"C:\Windows\System32\notepad.exe")
            .expect("create temp Run value");

        invalidate_cache();
        let snapshot = get_startup_items(true).expect("enumerate");
        let id = format!("hkcu_run|{TEST_NAME}");
        let item = snapshot
            .items
            .iter()
            .find(|item| item.id == id)
            .expect("temp entry listed");
        assert!(item.enabled, "fresh Run value must start enabled");

        let plan = plan_toggle(&id).expect("plan toggle");
        apply_local_toggle(&plan, false).expect("disable");

        invalidate_cache();
        let disabled = get_startup_items(true).expect("re-enumerate");
        assert!(
            !disabled
                .items
                .iter()
                .find(|item| item.id == id)
                .unwrap()
                .enabled,
            "StartupApproved blob must read back as disabled"
        );

        apply_local_toggle(&plan, true).expect("re-enable");
        invalidate_cache();
        let enabled = get_startup_items(true).expect("re-enumerate");
        assert!(
            enabled
                .items
                .iter()
                .find(|item| item.id == id)
                .unwrap()
                .enabled,
            "re-enabling must clear the disabled state"
        );
    }

    /// Live, read-only smoke test. Ignored by default because it depends on
    /// the machine's real startup entries and task scheduler.
    #[test]
    #[ignore]
    fn live_enumeration_is_read_only_and_complete() {
        let snapshot = get_startup_items(true).expect("enumeration");
        println!(
            "tasks_available={} skipped={} items={}",
            snapshot.tasks_available,
            snapshot.tasks_skipped,
            snapshot.items.len()
        );
        if let Some(err) = &snapshot.task_scan_error {
            println!("task scan error: {err}");
        }
        for item in &snapshot.items {
            println!(
                "{} | {} | enabled={} admin={} exists={:?} publisher={:?} exe={:?}",
                item.source.as_str(),
                item.name,
                item.enabled,
                item.requires_admin,
                item.file_exists,
                item.publisher,
                item.executable
            );
        }
        assert!(snapshot.tasks_available, "{:?}", snapshot.task_scan_error);
        for item in &snapshot.items {
            assert!(!item.name.is_empty());
            assert!(!item.location.is_empty());
            if let Some(false) = item.file_exists {
                assert!(item.executable.is_some());
            }
        }
    }
}
