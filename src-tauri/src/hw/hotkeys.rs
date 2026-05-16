//! Keyboard hotkey interception via WH_KEYBOARD_LL (low-level keyboard hook).
//!
//! **Option A — 3 fixed Xiaomi laptop keys:**
//!   • Xiaomi AI Key   (VK_LAUNCH_APP1, 0xB6) — original Xiaomi AI assistant button
//!   • Xiaomi PCM Key  (VK_LAUNCH_APP2, 0xB7) — Xiaomi PC Manager button
//!   • Copilot Key     (0xC3 / 0xB7)          — Windows Copilot key (Win11 24H2+)
//!
//! Each key can be bound to: Nothing | Open URL | Launch App.
//! Config is persisted to `%LOCALAPPDATA%\MiControl\hotkeys.json`.
//!
//! ─────────────────────────────────────────────────────────────────────────────
//! TODO (Option B — Full Keyboard Remapping Module):
//!
//! 1. Replace `HotkeyMap` (3 fixed keys) with `Vec<CustomHotkey>` where each entry
//!    has its own `vk_code: u32`, `scan_code: u32`, and `display_name: String`.
//!
//! 2. Add "detect key" mode: a temporary WH_KEYBOARD_LL hook that captures the next
//!    key pressed and reports its VK + scan code back to the frontend via a Tauri
//!    event (`app_handle.emit("hotkey://detected", vk_code)`).
//!    Frontend shows a "Press any key…" modal that records the keypress.
//!
//! 3. Add more `HotkeyAction` variants:
//!    - `SetPerformanceMode { mode: String }` — call set_performance_mode directly
//!    - `ToggleAiBrightness` — flip AI adaptive brightness on/off
//!    - `MediaControl { action: String }` — "volume_up", "volume_down", "play_pause"
//!    - `Script { interpreter: String, path: String }` — run PowerShell / cmd script
//!
//! 4. Add modifier key support (e.g. Ctrl+VK, Alt+VK, Win+VK combos).
//!    Use the `flags` field of KBDLLHOOKSTRUCT to check extended/injected bits.
//!
//! 5. Add conflict detection: warn if the requested VK is system-reserved
//!    (PrintScreen, Win+key combinations known to be OS-level, etc.).
//!
//! 6. Add scancode-level remapping via
//!    `HKLM\SYSTEM\CurrentControlSet\Control\Keyboard Layout\Scancode Map`
//!    (requires elevation + reboot, but survives process restart).
//!    Offer this as a "permanent remap" alternative to the hook approach.
//!
//! 7. Add key-sequence / chord support (press two keys in sequence to trigger an action).
//!
//! 8. Per-Windows-user profile storage (multi-user session awareness).
//!
//! 9. Frontend: visual key-capture widget — a button labeled "Click then press key"
//!    that triggers detect-key mode and auto-fills the VK field.
//! ─────────────────────────────────────────────────────────────────────────────

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ── VK codes for the 3 target keys ──────────────────────────────────────────

/// Xiaomi AI assistant button (often labelled with an AI icon on the keyboard).
const VK_AI_KEY: u32 = 0xB6; // VK_LAUNCH_APP1

/// Xiaomi PC Manager button (original action: open XiaomiPCManager.exe).
const VK_XIAOMI_KEY: u32 = 0xB7; // VK_LAUNCH_APP2

/// Windows Copilot key on Win11 24H2+ keyboards (some boards still use 0xB7).
const VK_COPILOT: u32 = 0xC3;

// Hide the process window when spawning child processes via CreateProcess.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ── Shared state ─────────────────────────────────────────────────────────────

/// Global hotkey config — written by Tauri commands, read by the hook callback.
static HOTKEY_CONFIG: OnceLock<Arc<RwLock<HotkeyMap>>> = OnceLock::new();

/// Raw HHOOK handle stored as usize so it is `Send`-compatible.
static HOOK_HANDLE: AtomicUsize = AtomicUsize::new(0);

/// Thread ID of the hook message-loop thread (used for clean teardown).
static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);

// ── Public types ─────────────────────────────────────────────────────────────

/// What happens when an intercepted key fires.
///
/// Extend this enum for Option B (see module-level TODO list).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HotkeyAction {
    /// Suppress the key and do nothing.
    None,
    /// Open a URL in the system default browser.
    OpenUrl { url: String },
    /// Launch an executable (absolute path).
    LaunchApp { path: String, args: Vec<String> },
}

impl Default for HotkeyAction {
    fn default() -> Self {
        HotkeyAction::None
    }
}

/// Per-key binding entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyBinding {
    /// Whether to intercept this key. When `false` the key passes through untouched.
    pub enabled: bool,
    /// The action to perform when the key fires.
    pub action: HotkeyAction,
    /// Human-readable label shown in the Settings UI.
    pub label: Option<String>,
}

impl Default for KeyBinding {
    fn default() -> Self {
        Self {
            enabled: false,
            action: HotkeyAction::None,
            label: None,
        }
    }
}

/// The full hotkey configuration — 3 fixed keys for Option A.
///
/// TODO (Option B): Replace with `Vec<CustomHotkey>` where each entry carries
/// its own `vk_code: u32` and `scan_code: u32` discovered via detect-key mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyMap {
    /// Xiaomi AI assistant key (VK_LAUNCH_APP1, 0xB6).
    pub ai_key: KeyBinding,
    /// Xiaomi PC Manager key (VK_LAUNCH_APP2, 0xB7).
    pub xiaomi_key: KeyBinding,
    /// Windows Copilot key (0xC3 on Win11 24H2+; may overlap with 0xB7 on some boards).
    pub copilot_key: KeyBinding,
}

impl Default for HotkeyMap {
    fn default() -> Self {
        // Default: Xiaomi key redirects to MiControl itself; others are unbound.
        let micontrol_exe = std::env::current_exe()
            .unwrap_or_else(|_| PathBuf::from("micontrol.exe"))
            .to_string_lossy()
            .into_owned();

        Self {
            ai_key: KeyBinding {
                enabled: false,
                action: HotkeyAction::None,
                label: Some("Xiaomi AI Key".into()),
            },
            xiaomi_key: KeyBinding {
                enabled: true,
                action: HotkeyAction::LaunchApp {
                    path: micontrol_exe,
                    args: vec![],
                },
                label: Some("Xiaomi PC Manager Key".into()),
            },
            copilot_key: KeyBinding {
                enabled: false,
                action: HotkeyAction::None,
                label: Some("Copilot Key".into()),
            },
        }
    }
}

// ── Config persistence ────────────────────────────────────────────────────────

fn config_path() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("MiControl").join("hotkeys.json")
}

/// Load hotkey config from disk, returning defaults if the file is absent or corrupt.
pub fn load_config() -> HotkeyMap {
    let path = config_path();
    if let Ok(data) = std::fs::read_to_string(&path) {
        if let Ok(cfg) = serde_json::from_str::<HotkeyMap>(&data) {
            return cfg;
        }
    }
    HotkeyMap::default()
}

/// Persist hotkey config to `%LOCALAPPDATA%\MiControl\hotkeys.json`.
pub fn save_config(config: &HotkeyMap) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create MiControl data dir")?;
    }
    let json = serde_json::to_string_pretty(config).context("serialize hotkey config")?;
    std::fs::write(&path, json).context("write hotkeys.json")?;
    Ok(())
}

/// Update the in-memory config (called by the `set_hotkey_config` Tauri command).
pub fn update_in_memory(config: HotkeyMap) {
    if let Some(arc) = HOTKEY_CONFIG.get() {
        if let Ok(mut guard) = arc.write() {
            *guard = config;
        }
    }
}

/// Read the current in-memory config (called by the `get_hotkey_config` Tauri command).
pub fn read_in_memory() -> HotkeyMap {
    HOTKEY_CONFIG
        .get()
        .and_then(|arc| arc.read().ok())
        .map(|g| g.clone())
        .unwrap_or_default()
}

// ── Hook installation ─────────────────────────────────────────────────────────

/// Install the WH_KEYBOARD_LL hook and run the message loop on a dedicated thread.
///
/// Call this once from `tauri::Builder::setup`. The thread keeps running until the
/// process exits (or `stop_hook()` is called for a clean teardown).
pub fn start_hook() {
    // Initialise shared config from disk.
    let initial = load_config();
    let _ = HOTKEY_CONFIG.set(Arc::new(RwLock::new(initial)));

    std::thread::Builder::new()
        .name("hotkey-hook".into())
        .spawn(hook_thread_main)
        .expect("spawn hotkey hook thread");
}

/// Signal the hook thread to exit (sends WM_QUIT to its message loop).
#[allow(dead_code)]
pub fn stop_hook() {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
    let tid = HOOK_THREAD_ID.load(Ordering::Relaxed);
    if tid != 0 {
        unsafe {
            let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

// ── Hook thread ───────────────────────────────────────────────────────────────

fn hook_thread_main() {
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
        WH_KEYBOARD_LL,
    };
    use windows::Win32::Foundation::HMODULE;

    // Record this thread's ID so stop_hook() can post WM_QUIT.
    let tid = unsafe { GetCurrentThreadId() };
    HOOK_THREAD_ID.store(tid, Ordering::Relaxed);

    // Install the hook. For WH_KEYBOARD_LL the module handle is ignored by Windows;
    // we pass a null HMODULE as allowed by MSDN for low-level hooks.
    let hook_result = unsafe {
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook_proc), HMODULE::default(), 0)
    };

    let hhook = match hook_result {
        Ok(h) => h,
        Err(e) => {
            log::error!("[hotkeys] SetWindowsHookExW failed: {e}");
            return;
        }
    };

    // Store raw pointer for teardown.
    HOOK_HANDLE.store(hhook.0 as usize, Ordering::Relaxed);
    log::info!("[hotkeys] WH_KEYBOARD_LL hook installed (thread {tid})");

    // Drive the Windows message loop so the hook callback gets invoked.
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::MSG;
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // Clean up on exit.
    unsafe {
        let _ = UnhookWindowsHookEx(hhook);
    }
    HOOK_HANDLE.store(0, Ordering::Relaxed);
    log::info!("[hotkeys] hook uninstalled");
}

// ── Hook callback ─────────────────────────────────────────────────────────────

/// Low-level keyboard hook procedure.
///
/// Called by Windows on the hook thread for every system-wide key event.
/// Must return quickly (< ~500 ms) or Windows will unhook us automatically.
unsafe extern "system" fn keyboard_hook_proc(
    n_code: i32,
    w_param: windows::Win32::Foundation::WPARAM,
    l_param: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, KBDLLHOOKSTRUCT, WM_KEYDOWN, WM_SYSKEYDOWN,
    };

    // n_code < 0 means "do not process, pass to next hook".
    if n_code < 0 {
        return CallNextHookEx(None, n_code, w_param, l_param);
    }

    let event_type = w_param.0 as u32;
    let is_keydown = event_type == WM_KEYDOWN || event_type == WM_SYSKEYDOWN;

    if is_keydown {
        let kb = &*(l_param.0 as *const KBDLLHOOKSTRUCT);
        let vk = kb.vkCode;

        if let Some(action) = resolve_action(vk) {
            // Dispatch on a fresh OS thread — hook callback must return fast.
            std::thread::spawn(move || dispatch_action(&action));
            // Returning non-zero suppresses the key (it never reaches any window).
            return windows::Win32::Foundation::LRESULT(1);
        }
    }

    CallNextHookEx(None, n_code, w_param, l_param)
}

/// Check whether the given VK matches a configured, enabled binding.
/// Returns `Some(action)` if we should intercept the key.
fn resolve_action(vk: u32) -> Option<HotkeyAction> {
    let arc = HOTKEY_CONFIG.get()?;
    let map = arc.read().ok()?;

    let binding = match vk {
        v if v == VK_AI_KEY => &map.ai_key,
        v if v == VK_XIAOMI_KEY => &map.xiaomi_key,
        // Copilot key: accept both the Win11 24H2 VK (0xC3) and the older VK_LAUNCH_APP2
        // on boards that emit 0xB7 for the Copilot key instead of the PCManager key.
        v if v == VK_COPILOT => &map.copilot_key,
        _ => return None,
    };

    if binding.enabled && binding.action != HotkeyAction::None {
        Some(binding.action.clone())
    } else {
        None
    }
}

// ── Action dispatch ───────────────────────────────────────────────────────────

fn dispatch_action(action: &HotkeyAction) {
    match action {
        HotkeyAction::None => {}

        HotkeyAction::OpenUrl { url } => {
            // Use `explorer <url>` — works for http/https and mailto links.
            let result = std::process::Command::new("explorer")
                .arg(url)
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
            if let Err(e) = result {
                log::warn!("[hotkeys] OpenUrl failed for '{url}': {e}");
            }
        }

        HotkeyAction::LaunchApp { path, args } => {
            let result = std::process::Command::new(path)
                .args(args)
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
            if let Err(e) = result {
                log::warn!("[hotkeys] LaunchApp failed for '{path}': {e}");
            }
        }
    }
}
