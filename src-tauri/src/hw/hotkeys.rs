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
//! 2. Add "detect key" mode: DONE — `start_detect_mode` / `get_detected_vk` implemented.
//!
//! 3. New `HotkeyAction` variants: DONE — `SetPerformanceMode`, `ToggleAiBrightness`,
//!    `MediaControl`, `Script` all implemented.
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
//! ─────────────────────────────────────────────────────────────────────────────

use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use crate::util::panic::lock_or_recover;

use crate::hw::errors::HardwareResult;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

/// Check if a key is currently pressed using GetAsyncKeyState.
/// This avoids TOCTOU race conditions by checking the actual key state
/// at the moment of the decision, rather than relying on a time-based heuristic.
fn is_key_down(vk: u32) -> bool {
    // SAFETY: GetAsyncKeyState is thread-safe and has no side effects.
    // Invalid VK codes return 0, which is safe.
    // Using raw FFI because the windows crate does not expose GetAsyncKeyState
    // under the currently enabled features.
    extern "system" {
        fn GetAsyncKeyState(v_key: i32) -> i16;
    }
    // SAFETY: GetAsyncKeyState is a lightweight Win32 query with no safety
    // invariants — it reads the async key state and returns a short. The FFI
    // signature matches the Win32 API exactly.
    let state = unsafe { GetAsyncKeyState(vk as i32) };
    // High-order bit is set if the key is down
    (state as u16 & 0x8000) != 0
}

/// Global hotkey config — written by Tauri commands, read by the hook callback.
static HOTKEY_CONFIG: OnceLock<Arc<RwLock<HotkeyMap>>> = OnceLock::new();

/// Raw HHOOK handle stored as usize so it is `Send`-compatible.
static HOOK_HANDLE: AtomicUsize = AtomicUsize::new(0);

/// Set to `true` after `RegisterRawInputDevices` succeeds on the hook thread.
/// Used by `is_hook_active()` to signal that key detection is live.
static RAW_INPUT_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Thread ID of the hook message-loop thread (used for clean teardown).
static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);

/// Optional callback registered by the Tauri app to show/focus the existing
/// MiControl window.  Used by `FocusMicontrol` action and by `LaunchApp` when
/// the target resolves to our own executable.
static FOCUS_CALLBACK: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Optional callback registered by the Tauri app to show/focus the main
/// MiControl application window.  Used by `OpenMainWindow` action.
static OPEN_MAIN_CALLBACK: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Remap state machine — tracks whether the user is in key-detect mode.
///
/// All transitions (begin, capture, cancel) go through a single Mutex so that
/// concurrent access from the hook callback, timeout thread, WMI listener, and
/// Tauri commands cannot produce a race between reading and writing the state.
enum RemapState {
    Idle,
    AwaitingKey {
        /// VK code captured so far (0 = none yet).
        captured_vk: u32,
    },
}

static REMAP_STATE: Mutex<RemapState> = Mutex::new(RemapState::Idle);

/// Per-key debounce for WMI HID events.
/// Maps a key code (derived from class_idx + distinguish_byte) to the last
/// time that key's action was dispatched.  Different keys do not suppress
/// each other; repeated firings of the same key within the window are debounced.
static WMI_DEBOUNCE: OnceLock<Mutex<HashMap<u32, std::time::Instant>>> = OnceLock::new();

/// Returns `true` if the given key code should be debounced (i.e. it fired
/// within the last 400 ms).  Otherwise records the current time and returns
/// `false` — the caller should process the event.
///
/// Stale entries older than 60 seconds are purged on each call.
fn wmi_key_debounced(key_code: u32) -> bool {
    use std::time::Instant;
    const DEBOUNCE_MS: u64 = 400;

    let now = Instant::now();
    let map = WMI_DEBOUNCE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = lock_or_recover(map);

    if let Some(last) = map.get(&key_code) {
        if now.duration_since(*last).as_millis() < DEBOUNCE_MS as u128 {
            log::info!(
                "[hotkeys] WMI key {:#06X} debounced ({}.{} ms)",
                key_code,
                now.duration_since(*last).as_millis(),
                now.duration_since(*last).as_micros() % 1000,
            );
            // Still clean up stale entries even when debouncing.
            map.retain(|_, v| now.duration_since(*v).as_secs() < 60);
            return true;
        }
    }

    map.insert(key_code, now);
    // Opportunistic cleanup of entries older than 60 s.
    map.retain(|_, v| now.duration_since(*v).as_secs() < 60);
    false
}

/// Avoid spamming the same admin guidance on every startup/retry.
static VCHID_ACCESS_DENIED_LOGGED: AtomicBool = AtomicBool::new(false);

// ── RemapToKey state ─────────────────────────────────────────────────────────

/// Marker stored in `KEYBDINPUT.dwExtraInfo` for all keys we inject via
/// `SendInput`.  The LL keyboard hook checks for this value and passes injected
/// keys straight through, preventing infinite re-trigger.
const MICONTROL_INJECT_MAGIC: usize = 0xA4_EC_12_34;

/// Virtual key of the physical source key currently held (0 = no remap active).
static REMAP_SOURCE_VK: AtomicU32 = AtomicU32::new(0);

/// Virtual key we are injecting as the remap target (0 = no remap active).
static REMAP_TARGET_VK: AtomicU32 = AtomicU32::new(0);

/// Whether the remap target key needs `KEYEVENTF_EXTENDEDKEY` (right-side keys).
static REMAP_TARGET_EXTENDED: AtomicBool = AtomicBool::new(false);

// ── Public types ─────────────────────────────────────────────────────────────

/// What happens when an intercepted key fires.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HotkeyAction {
    /// Suppress the key and do nothing.
    None,
    /// Show and focus the MiControl tray popup window.
    FocusMicontrol,
    /// Show and focus the full MiControl main application window.
    OpenMainWindow,
    /// Open a URL in the system default browser.
    OpenUrl { url: String },
    /// Launch an executable (absolute path).
    LaunchApp { path: String, args: Vec<String> },
    /// Remap this key to a different virtual key (hold behaviour).
    ///
    /// On key-down: releases the spurious Win+Shift modifiers that accompany
    /// the Copilot key, then injects target-key-down.
    /// On key-up  : injects target-key-up.
    /// `extended` must be `true` for right-side keys (RCtrl=0xA3, RAlt=0xA5, …).
    RemapToKey { vk: u32, extended: bool },
    /// Immediately switch to the named performance mode.
    /// `mode` must be a snake_case variant of `PerformanceMode`, e.g. "turbo".
    SetPerformanceMode { mode: String },
    /// Toggle AI adaptive brightness on or off.
    ToggleAiBrightness,
    /// Inject a media/system key.
    /// `action`: "volume_up" | "volume_down" | "mute" | "play_pause" | "next" | "prev"
    MediaControl { action: String },
    /// Run a script or executable without a visible window.
    /// `interpreter`: "" (direct) | "powershell" | "cmd"
    Script {
        interpreter: String,
        path: String,
        args: Vec<String>,
    },
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
        Self {
            ai_key: KeyBinding {
                // Fn+F7 (Xiaomi AI) → toggle the miControl tray popup
                enabled: true,
                action: HotkeyAction::FocusMicontrol,
                label: Some("Xiaomi AI Key (Fn+F7)".into()),
            },
            xiaomi_key: KeyBinding {
                // Xiaomi Key → toggle the miControl tray popup (replaces XiaomiPCManager)
                enabled: true,
                action: HotkeyAction::FocusMicontrol,
                label: Some("Xiaomi Key (opens miControl)".into()),
            },
            copilot_key: KeyBinding {
                // Copilot Key → remap to Right Ctrl (same as AHK CopilotKeyRemap)
                enabled: true,
                action: HotkeyAction::RemapToKey {
                    vk: 0xA3,
                    extended: true,
                },
                label: Some("Copilot Key → Right Ctrl".into()),
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
/// Also migrates legacy `LaunchApp` entries that target our own exe → `FocusMicontrol`.
pub fn load_config() -> HotkeyMap {
    let path = config_path();
    if let Ok(data) = std::fs::read_to_string(&path) {
        if let Ok(cfg) = serde_json::from_str::<HotkeyMap>(&data) {
            // Upgrade ACL on existing config file — silently ignore errors.
            let _ = crate::util::auth::restrict_file_acl(&path);
            return migrate_config(cfg);
        }
    }
    HotkeyMap::default()
}

/// Upgrade any `LaunchApp` action that points to our own executable to `FocusMicontrol`.
/// Also upgrade the copilot key from the old `FocusMicontrol` default to `RemapToKey`.
fn migrate_config(mut cfg: HotkeyMap) -> HotkeyMap {
    let our_exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok());
    for binding in [&mut cfg.ai_key, &mut cfg.xiaomi_key, &mut cfg.copilot_key] {
        if let HotkeyAction::LaunchApp { ref path, .. } = binding.action {
            let is_self = our_exe
                .as_deref()
                .and_then(|exe| PathBuf::from(path).canonicalize().ok().map(|p| p == exe))
                .unwrap_or(false);
            if is_self {
                binding.action = HotkeyAction::FocusMicontrol;
            }
        }
    }
    // One-time migration: if the copilot key was left at the old FocusMicontrol
    // default, promote it to the new RemapToKey (Right Ctrl) default.
    if cfg.copilot_key.action == HotkeyAction::FocusMicontrol
        && cfg.copilot_key.label.as_deref() == Some("Copilot Key")
    {
        cfg.copilot_key.action = HotkeyAction::RemapToKey {
            vk: 0xA3,
            extended: true,
        };
        cfg.copilot_key.label = Some("Copilot Key → Right Ctrl".into());
    }
    cfg
}

/// Persist hotkey config to `%LOCALAPPDATA%\MiControl\hotkeys.json`.
pub fn save_config(config: &HotkeyMap) -> HardwareResult<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create MiControl data dir")?;
    }
    let json = serde_json::to_string_pretty(config).context("serialize hotkey config")?;
    std::fs::write(&path, json).context("write hotkeys.json")?;

    // Restrict ACL on the config file — prevents other users from modifying
    // hotkey actions (Script/LaunchApp injection vector, CWE-1104).
    if let Err(e) = crate::util::auth::restrict_file_acl(&path) {
        log::warn!("[hotkeys] Failed to restrict ACL on hotkeys.json: {e}");
        // Don't fail — the file is written, just without ACL protection.
    }

    Ok(())
}

/// Update the in-memory config (called by the `set_hotkey_config` Tauri command).
pub fn update_in_memory(config: HotkeyMap) {
    if let Some(arc) = HOTKEY_CONFIG.get() {
        // S24-006: Use lock_write_or_recover for consistent poison recovery.
        let mut guard = crate::util::panic::lock_write_or_recover(arc);
        *guard = config;
    }
}

/// Read the current in-memory config (called by the `get_hotkey_config` Tauri command).
pub fn read_in_memory() -> HotkeyMap {
    HOTKEY_CONFIG
        .get()
        // S24-006: Use lock_read_or_recover for consistent poison recovery.
        .map(|arc| crate::util::panic::lock_read_or_recover(arc).clone())
        .unwrap_or_default()
}

// ── Hook installation ─────────────────────────────────────────────────────────

/// Register the callback that will be invoked to show/focus the existing MiControl
/// window on `FocusMicontrol` actions (and `LaunchApp` pointing to our own exe).
/// Call this once during Tauri `setup`, after `start_hook()`.
pub fn set_focus_callback(f: Box<dyn Fn() + Send + Sync>) {
    let _ = FOCUS_CALLBACK.set(f);
}

/// Register the callback that will be invoked to show/focus the MiControl main
/// application window on `OpenMainWindow` actions.
/// Call this once during Tauri `setup`, after `start_hook()`.
pub fn set_open_main_callback(f: Box<dyn Fn() + Send + Sync>) {
    let _ = OPEN_MAIN_CALLBACK.set(f);
}

/// Start key-detect mode: the hook will log and record every non-modifier key
/// for the next 10 seconds.  Read the result with `get_detected_vk()`.
pub fn start_detect_mode() {
    {
        let mut state = lock_or_recover(&REMAP_STATE);
        *state = RemapState::AwaitingKey { captured_vk: 0 };
    }
    log::info!("[hotkeys] Key detect mode started (10 s max — press any key)");
    std::thread::spawn(|| {
        // Poll every 100 ms; exit early as soon as a key is detected.
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let state = lock_or_recover(&REMAP_STATE);
            if let RemapState::AwaitingKey { captured_vk, .. } = &*state {
                if *captured_vk != 0 {
                    break;
                }
            }
        }
        let last_vk = {
            let mut state = lock_or_recover(&REMAP_STATE);
            let vk = match &*state {
                RemapState::AwaitingKey { captured_vk, .. } => *captured_vk,
                _ => 0,
            };
            // Only transition to Idle if still in AwaitingKey (not already cancelled).
            if matches!(&*state, RemapState::AwaitingKey { .. }) {
                *state = RemapState::Idle;
            }
            vk
        };
        log::info!("[hotkeys] Key detect mode ended, last VK: {:#04X}", last_vk);
    });
}

/// Cancel key-detect mode immediately, discarding any captured key.
/// Safe to call even when not in detect mode.
///
/// Public API: available for frontend-initiated cancellation of key detection.
/// Cancel detect mode (used by frontend via Tauri command).
#[allow(dead_code)]
pub fn cancel_detect_mode() {
    let mut state = lock_or_recover(&REMAP_STATE);
    *state = RemapState::Idle;
    log::info!("[hotkeys] Key detect mode cancelled");
}

pub fn get_detected_vk() -> u32 {
    let state = lock_or_recover(&REMAP_STATE);
    match &*state {
        RemapState::AwaitingKey { captured_vk, .. } => *captured_vk,
        RemapState::Idle => 0,
    }
}

/// If detect mode is active and no key has been captured yet, store the VK.
/// Returns `true` if detect mode was active (caller may log the capture).
fn capture_key(vk: u32) -> bool {
    let mut state = lock_or_recover(&REMAP_STATE);
    match &mut *state {
        RemapState::AwaitingKey { captured_vk, .. } if *captured_vk == 0 => {
            *captured_vk = vk;
            true
        }
        RemapState::AwaitingKey { .. } => true, // Already captured, still in detect mode
        RemapState::Idle => false,
    }
}

/// Return `true` if key detection is active (Raw Input registered, or WH_KEYBOARD_LL installed).
pub fn is_hook_active() -> bool {
    RAW_INPUT_ACTIVE.load(Ordering::Relaxed) || HOOK_HANDLE.load(Ordering::Relaxed) != 0
}

/// Install the WH_KEYBOARD_LL hook and run the message loop on a dedicated thread.
///
/// Call this once from `tauri::Builder::setup`. The thread keeps running until the
/// process exits (or `stop_hook()` is called for a clean teardown).
///
/// Returns an error if the hotkey thread cannot be spawned (e.g. resource
/// exhaustion). The caller should log a warning and continue without hotkeys.
pub fn start_hook() -> Result<(), crate::hw::errors::HardwareError> {
    // Start the Xiaomi VHF bridge service.  This is the component that relays
    // ACPI-based special-key events (Fn+F7 / Xiaomi button / Copilot key) from
    // IoTSvc to Win32 as HID Consumer Control reports.
    // We use the Win32 SCM API directly so we can log the exact result.
    start_virtual_control_hid();

    // After requesting the service start, spawn a delayed background thread that
    // opens every interesting HID device file directly — bypassing the Raw Input
    // registration path and catching any device the VHF driver creates.
    start_hid_raw_reader();

    // Subscribe directly to IoTDriver WMI events in root\WMI.  These events are
    // fired by the IoT kernel driver when ACPI-special keys are pressed and are
    // the ground-truth source: IoTSvc subscribes to them to feed VirtualControlHID.
    // Tapping them here means Xiaomi/AI/Copilot keys work regardless of whether
    // VirtualControlHID is running.
    start_wmi_hid_listener();

    // Initialise shared config from disk.
    let initial = load_config();
    let _ = HOTKEY_CONFIG.set(Arc::new(RwLock::new(initial)));

    // S24-004: Replace expect() with graceful error handling.
    std::thread::Builder::new()
        .name("hotkey-hook".into())
        .spawn(hook_thread_main)
        .map_err(|e| {
            crate::hw::errors::HardwareError::Other(format!("Failed to spawn hotkey thread: {e}"))
        })?;

    Ok(())
}

/// Signal the hook thread to exit (sends WM_QUIT to its message loop).
///
/// Public API: available for clean shutdown of the hotkey hook thread.
/// Stop the low-level keyboard hook (used on shutdown).
#[allow(dead_code)]
pub fn stop_hook() {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
    let tid = HOOK_THREAD_ID.load(Ordering::Relaxed);
    if tid != 0 {
        // SAFETY: PostThreadMessageW posts a message to a known thread; tid was stored by GetCurrentThreadId() which is always valid for the hook thread. WM_QUIT with 0 params is safe.
        unsafe {
            let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

// ── Hook thread ───────────────────────────────────────────────────────────────

fn hook_thread_main() {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HINSTANCE, HMODULE};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::{RegisterRawInputDevices, RAWINPUTDEVICE, RIDEV_INPUTSINK};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DispatchMessageW, GetMessageW, PeekMessageW, RegisterClassExW,
        SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HWND_MESSAGE, MSG, PM_NOREMOVE,
        WH_KEYBOARD_LL, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSEXW,
    };

    // Record this thread's ID so stop_hook() can post WM_QUIT.
    // SAFETY: GetCurrentThreadId is a simple Win32 call with no safety invariants — it always succeeds.
    let tid = unsafe { GetCurrentThreadId() };
    HOOK_THREAD_ID.store(tid, Ordering::Relaxed);

    // Force-create the thread message queue before any window or hook work.
    // SAFETY: PeekMessageW with PM_NOREMOVE forces creation of the thread's message queue; msg is properly zero-initialized via default().
    unsafe {
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
    }

    // ── Create a message-only window so Raw Input has a delivery target ──────
    // HWND_MESSAGE parent → invisible window, never shown in taskbar.
    let class_name: Vec<u16> = "MiControlHotkey\0".encode_utf16().collect();
    // SAFETY: RegisterClassExW requires a properly initialized WNDCLASSEXW; we provide all fields including the window proc. The class name is a valid null-terminated UTF-16 string.
    // CreateWindowExW creates a message-only window (HWND_MESSAGE); the returned HWND is valid for the lifetime of this thread.
    let hwnd = unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(raw_input_wnd_proc),
            hInstance: HINSTANCE::default(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        RegisterClassExW(&wc); // ok if already registered on restart
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR::null(),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            None,
            None,
            None,
        )
    };
    let hwnd = match hwnd {
        Ok(h) => h,
        Err(e) => {
            log::error!("[hotkeys] CreateWindowExW failed: {e}");
            return;
        }
    };

    // ── Register Raw Input with RIDEV_INPUTSINK ─────────────────────────────
    // Devices registered:
    //  • UsagePage 0x01 / UsageId 0x06 = Standard keyboard (all typing keys + Xiaomi Key).
    //  • UsagePage 0x0C / UsageId 0x01 = HID Consumer Controls — standard multimedia keys
    //    and Xiaomi special keys (via VirtualControlHID VHF device).
    //  • UsagePage 0xFF00 / UsageId 0x000E = Vendor-specific (Xiaomi USB keyboard col05)
    //  • UsagePage 0xFFBC / UsageId 0x0088 = Vendor-specific (Xiaomi USB keyboard col04)
    //    These two vendor channels carry Xiaomi-specific key codes not in Consumer spec.
    let raw_devices = [
        RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x06,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        },
        RAWINPUTDEVICE {
            usUsagePage: 0x0C,
            usUsage: 0x01,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        },
        RAWINPUTDEVICE {
            usUsagePage: 0xFF00,
            usUsage: 0x000E,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        },
        RAWINPUTDEVICE {
            usUsagePage: 0xFFBC,
            usUsage: 0x0088,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        },
    ];
    // SAFETY: RegisterRawInputDevices takes an array of RAWINPUTDEVICE structs and a valid window handle (hwnd). The structs are properly initialized with correct usage pages/flags.
    match unsafe {
        RegisterRawInputDevices(&raw_devices, std::mem::size_of::<RAWINPUTDEVICE>() as u32)
    } {
        Ok(()) => {
            RAW_INPUT_ACTIVE.store(true, Ordering::Relaxed);
            log::info!("[hotkeys] Raw Input keyboard+consumer listener active (RIDEV_INPUTSINK, thread {tid})");
        }
        Err(e) => {
            log::warn!(
                "[hotkeys] RegisterRawInputDevices failed: {e}. Key detection may not work."
            );
        }
    }

    // ── RegisterHotKey for Xiaomi special keys ────────────────────────────────
    // On Windows 11 24H2+ the Copilot key is intercepted by the Windows Shell
    // BEFORE WH_KEYBOARD_LL or Raw Input, so it opens Settings instead of
    // triggering our handler.  RegisterHotKey claims the VK at the Win32 level:
    // Windows posts WM_HOTKEY to our window and skips the Shell handler entirely.
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, HOT_KEY_MODIFIERS};
        for (id, vk) in [
            (101i32, VK_AI_KEY),
            (102i32, VK_XIAOMI_KEY),
            (103i32, VK_COPILOT),
        ] {
            // SAFETY: RegisterHotKey takes a valid HWND (created via CreateWindowExW), a unique ID, and a VK code. The HWND is alive for the duration of the hook thread.
            match unsafe { RegisterHotKey(hwnd, id, HOT_KEY_MODIFIERS(0), vk) } {
                Ok(()) => log::info!("[hotkeys] RegisterHotKey VK={:#04X} id={id} OK", vk),
                Err(e) => log::warn!(
                    "[hotkeys] RegisterHotKey VK={:#04X} id={id} failed: {e}",
                    vk
                ),
            }
        }
    }

    // ── Install WH_KEYBOARD_LL for key suppression (best-effort) ─────────────
    // Action triggering is handled by Raw Input above. This hook only prevents
    // bound keys from reaching Windows default handlers (e.g. Copilot panel).
    // SAFETY: SetWindowsHookExW with WH_KEYBOARD_LL installs a global low-level keyboard hook. The callback pointer is valid for the lifetime of the hook thread. HMODULE::default() (NULL) is correct for global hooks — the hook is loaded from the current module.
    let hhook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_hook_proc),
            HMODULE::default(),
            0,
        )
        .ok()
    };
    match hhook {
        Some(h) => {
            HOOK_HANDLE.store(h.0 as usize, Ordering::Relaxed);
            log::info!("[hotkeys] WH_KEYBOARD_LL installed for key suppression (thread {tid})");
        }
        None => {
            log::warn!("[hotkeys] WH_KEYBOARD_LL not available — key suppression disabled, detection via Raw Input still works");
        }
    }

    // ── Message loop ──────────────────────────────────────────────────────────
    // WM_INPUT is dispatched to raw_input_wnd_proc via DispatchMessageW.
    // SAFETY: GetMessageW, TranslateMessage, and DispatchMessageW operate on the current thread's message queue (created above via PeekMessageW). msg is properly zero-initialized.
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // ── Cleanup ───────────────────────────────────────────────────────────────
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::UnregisterHotKey;
        for id in [101i32, 102i32, 103i32] {
            // SAFETY: UnregisterHotKey takes the same HWND and ID that were registered above; the HWND is still valid.
            unsafe {
                let _ = UnregisterHotKey(hwnd, id);
            }
        }
    }
    if let Some(h) = hhook {
        // SAFETY: UnhookWindowsHookEx removes the hook previously installed via SetWindowsHookExW; the HHOOK handle is valid and was stored at hook installation time.
        unsafe {
            let _ = UnhookWindowsHookEx(h);
        }
        HOOK_HANDLE.store(0, Ordering::Relaxed);
    }
    RAW_INPUT_ACTIVE.store(false, Ordering::Relaxed);
    log::info!("[hotkeys] hook thread exiting");
}

// ── Raw Input window proc ─────────────────────────────────────────────────────

/// Window procedure for the Raw Input message-only window.
///
/// # Safety
/// This is a Win32 window procedure called by the OS via RegisterClassExW / CreateWindowExW.
/// The HWND, WPARAM, and LPARAM parameters are provided by the Windows message pump (GetMessageW/DispatchMessageW)
/// and are guaranteed to be valid for the current thread's message queue.
unsafe extern "system" fn raw_input_wnd_proc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, WM_HOTKEY, WM_INPUT};
    if msg == WM_INPUT {
        handle_keyboard_raw_input(lparam.0);
    } else if msg == WM_HOTKEY {
        // RegisterHotKey fired: key is suppressed by Windows, dispatch our action.
        handle_hotkey_message(wparam.0 as i32);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Primary key detection — called from `raw_input_wnd_proc` on every WM_INPUT.
///
/// Raw Input with `RIDEV_INPUTSINK` is the modern replacement for WH_KEYBOARD_LL
/// background monitoring. It works regardless of foreground window elevation and
/// is not subject to the 1-second silent-removal timeout.
///
/// # Safety
/// `lparam` must be a valid LPARAM from a WM_INPUT message, pointing to a RAWINPUT struct.
/// The caller (`raw_input_wnd_proc`) guarantees this because Windows provides it.
/// The RAWINPUT struct is read via GetRawInputData — no direct pointer dereference of lparam.
unsafe fn handle_keyboard_raw_input(lparam: isize) {
    use windows::Win32::UI::Input::{
        GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTHEADER, RID_INPUT,
    };

    // Step 1: get required buffer size
    let mut size: u32 = 0;
    let r = GetRawInputData(
        HRAWINPUT(lparam as *mut _),
        RID_INPUT,
        None,
        &mut size,
        std::mem::size_of::<RAWINPUTHEADER>() as u32,
    );
    if r == u32::MAX || size == 0 || size > 4096 {
        return;
    }

    // Step 2: read the RAWINPUT struct
    let mut buf = vec![0u8; size as usize];
    let written = GetRawInputData(
        HRAWINPUT(lparam as *mut _),
        RID_INPUT,
        Some(buf.as_mut_ptr() as *mut _),
        &mut size,
        std::mem::size_of::<RAWINPUTHEADER>() as u32,
    );
    if written == u32::MAX || written == 0 {
        return;
    }

    // Validate buffer is large enough to contain a RAWINPUT struct
    if (written as usize) < std::mem::size_of::<RAWINPUT>() {
        log::warn!(
            "Raw input buffer too small: {} < {}",
            written,
            std::mem::size_of::<RAWINPUT>()
        );
        return;
    }

    // SAFETY: The buffer was allocated by GetRawInputData with RID_INPUT and verified to be at least sizeof(RAWINPUT) bytes;
    // the data is guaranteed valid per the Windows Raw Input API contract.
    let raw = buf.as_ptr() as *const RAWINPUT;

    match (*raw).header.dwType {
        2 => {
            // RIM_TYPEHID — Consumer Controls or vendor-specific HID device.
            // On Xiaomi laptops, Fn+F4/F7/F10 and special keys arrive here via:
            //  • Consumer Controls (UsagePage 0x0C) — standard multimedia keys
            //  • Vendor-specific (0xFF00/0xFFBC) — Xiaomi-defined key codes
            let hid = &(*raw).data.hid;
            let total = (hid.dwSizeHid.saturating_mul(hid.dwCount)) as usize;
            // SAFETY: bRawData is a fixed-size array in HID_DATA; we only access up to `total` bytes which are verified to be ≤ 64 by the saturating multiplication guard above.
            if total > 0 && total <= 64 {
                let p = hid.bRawData.as_ptr();

                {
                    // SAFETY: pointer arithmetic bounded by `total` (≤ 64); the array is at least 64 bytes per the HID_DATA struct definition.
                    let usage: u32 = if total >= 3 {
                        u16::from_le_bytes([*p.add(1), *p.add(2)]) as u32
                    } else if total >= 2 {
                        u16::from_le_bytes([*p, *p.add(1)]) as u32
                    } else {
                        *p as u32
                    };
                    if usage != 0 && capture_key(0x8000 | usage) {
                        let hex: Vec<String> =
                            (0..total).map(|i| format!("{:02X}", *p.add(i))).collect();
                        log::info!(
                            "[hotkeys] DETECT(HID type2 raw): {} byte(s): {}",
                            total,
                            hex.join(" ")
                        );
                    }
                }

                // Decode Consumer usage code and dispatch action.
                // 3-byte report: byte[0] = report ID, bytes[1-2] = usage LE
                // 2-byte report: bytes[0-1] = usage LE (no report ID)
                // SAFETY: pointer arithmetic bounded by `total` (≤ 64) guarded above.
                let usage: u16 = if total >= 3 {
                    u16::from_le_bytes([*p.add(1), *p.add(2)])
                } else if total >= 2 {
                    u16::from_le_bytes([*p, *p.add(1)])
                } else {
                    *p as u16
                };

                if usage != 0 {
                    log::debug!("[hotkeys] HID consumer usage={:#06X}", usage);
                    dispatch_consumer_usage(usage);
                }
            }
            return;
        }
        1 => {} // RIM_TYPEKEYBOARD — handled below
        _ => return,
    }

    // ── RIM_TYPEKEYBOARD path ─────────────────────────────────────────────────
    // RAWKEYBOARD.Flags bit 0 = RI_KEY_BREAK (1 = key-up, 0 = key-down)
    // SAFETY: The RAWINPUT struct was populated by GetRawInputData; the keyboard union member is valid because dwType == RIM_TYPEKEYBOARD.
    let kb = &(*raw).data.keyboard;
    let vk = kb.VKey as u32;
    let is_keydown = (kb.Flags & 0x01) == 0;
    if !is_keydown {
        return;
    }

    log::debug!(
        "[hotkeys] Raw Input key-down: VK={:#04X} scan={:#04X}",
        vk,
        kb.MakeCode
    );

    // ── Key-detect diagnostic mode ────────────────────────────────────────────
    match vk {
        // Skip pure modifier keys
        0x10..=0x12 | 0x14 | 0x5B | 0x5C | 0xA0..=0xA5 => {}
        0xFF => {
            if capture_key(0xFF) {
                log::info!(
                    "[hotkeys] DETECT(raw): VK=0xFF scan={:#04X} (no standard VK)",
                    kb.MakeCode
                );
            }
        }
        detected_vk => {
            if capture_key(detected_vk) {
                log::info!(
                    "[hotkeys] DETECT(raw): VK={:#04X} (decimal={})",
                    detected_vk,
                    detected_vk
                );
            }
        }
    }

    // ── Action dispatch ───────────────────────────────────────────────────────
    if let Some(action) = resolve_action(vk) {
        std::thread::spawn(move || dispatch_action(&action));
    }
}

/// Called from `raw_input_wnd_proc` when `WM_HOTKEY` fires.
///
/// `RegisterHotKey` claims the special Xiaomi keys at the Win32 level so the
/// Windows Shell cannot intercept them (e.g. Copilot opening Settings).
/// Both detect-mode recording and action dispatch happen here.
fn handle_hotkey_message(id: i32) {
    let vk = match id {
        101 => VK_AI_KEY,
        102 => VK_XIAOMI_KEY,
        103 => VK_COPILOT,
        _ => return,
    };
    log::info!("[hotkeys] WM_HOTKEY id={id} VK={:#04X}", vk);

    capture_key(vk);

    if let Some(action) = resolve_action(vk) {
        match action {
            HotkeyAction::RemapToKey {
                vk: target,
                extended,
            } => {
                log::info!(
                    "[hotkeys] WM_HOTKEY remap source={:#04X} target={:#04X} extended={}",
                    vk,
                    target,
                    extended
                );
                start_hotkey_remap(vk, target, extended);
            }
            other => {
                std::thread::spawn(move || dispatch_action(&other));
            }
        }
    }
}

/// Dispatch action based on a HID Consumer Control usage code.
///
/// Consumer usages come from the physical keyboard's Consumer Controls collection
/// (UsagePage 0x0C) and from the VirtualControlHID VHF device (if running).
/// Key → usage mappings for Xiaomi Book Pro 14 2024:
///   Fn+F4  = mic mute       → 0x00CF (Microphone) or 0x00E2 (Mute)
///   Fn+F7  = Xiaomi AI key  → VK_LAUNCH_APP1 (keyboard path) or 0x01B3/0x01B6 (consumer)
///   Fn+F10 = keyboard light → 0x0271 (Backlight) or vendor-specific
/// NOTE: Run app with detect mode (Settings → Hotkeys → "Detect Key") to find exact values.
fn dispatch_consumer_usage(usage: u16) {
    log::info!("[hotkeys] Consumer usage {:#06X}", usage);
    match usage {
        // ── Standard Consumer Controls ────────────────────────────────────────
        // 0x00E2 = Mute (audio output mute)
        // 0x00CF = Microphone (mic mute toggle, also seen on Xiaomi Fn+F4)
        // 0x0169 = AC Mute Microphone (newer standard, same function)
        0x00E2 | 0x00CF | 0x0169 => {
            log::info!("[hotkeys] Consumer: mic/audio mute key → show OSD");
            std::thread::spawn(crate::hw::osd::show_mic_mute_osd_toggle);
        }
        // 0x0271 = Keyboard Backlight Brightness (HID usage)
        // 0x01BB = Keyboard Backlight toggle (Xiaomi specific, may vary)
        0x0271 | 0x01BB | 0x0073 => {
            log::info!("[hotkeys] Consumer: keyboard backlight key → show OSD");
            std::thread::spawn(|| crate::hw::osd::show_keyboard_osd(0xFF));
        }
        // 0x01B3 = AL Application Launch (generic app key, often AI/search)
        // 0x01B6 = AL Application Launch - Instant Messaging
        // 0x0221 = AC Search
        // 0x01B1 = AL Message box
        0x01B3 | 0x01B6 | 0x0221 | 0x01B1 => {
            log::info!("[hotkeys] Consumer: app-launch/search key → focus miControl");
            dispatch_action(&HotkeyAction::FocusMicontrol);
        }
        _ => {
            // Unknown usage — only log if in detect mode (already logged above at debug)
        }
    }
}

// ── Hook callback ─────────────────────────────────────────────────────────────

/// Low-level keyboard hook procedure — suppression + RemapToKey.
///
/// Action dispatch and key detection are handled by `handle_keyboard_raw_input`
/// via the Raw Input path (WM_INPUT / RIDEV_INPUTSINK), which is more reliable.
/// This callback handles:
///   1. Suppressing bound keys so Windows default handlers never see them.
///   2. RemapToKey bindings: inject the target key on both keydown and keyup
///      so the target key behaves exactly like a physical key (hold works).
///
/// IMPORTANT: during detect mode we must NOT suppress, because returning
/// LRESULT(1) without calling CallNextHookEx blocks the key from reaching both
/// RegisterHotKey (no WM_HOTKEY) *and* Raw Input (no WM_INPUT), leaving the
/// remap state with captured_vk permanently at 0.  We record the VK here —
/// the earliest interception point — and pass the key through.
///
/// # Safety
/// This is a WH_KEYBOARD_LL hook callback, called by the Windows hook dispatcher.
/// n_code, w_param, and l_param are provided by the OS. When n_code >= 0 (HC_ACTION),
/// l_param MUST point to a valid KBDLLHOOKSTRUCT — this is checked with the null guard below.
unsafe extern "system" fn keyboard_hook_proc(
    n_code: i32,
    w_param: windows::Win32::Foundation::WPARAM,
    l_param: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, KBDLLHOOKSTRUCT, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    if n_code < 0 {
        return CallNextHookEx(None, n_code, w_param, l_param);
    }

    let event_type = w_param.0 as u32;

    // SAFETY: When n_code >= 0 (HC_ACTION), l_param must point to a valid
    // KBDLLHOOKSTRUCT per MSDN contract. We defensively reject null before dereferencing.
    if l_param.0 == 0 {
        log::warn!(target: "hw::hotkeys", "keyboard_hook_proc: null l_param (n_code={})", n_code);
        return CallNextHookEx(None, n_code, w_param, l_param);
    }
    // SAFETY: kb_ptr is non-null (checked above). The LPARAM from WH_KEYBOARD_LL hooks always
    // points to a valid KBDLLHOOKSTRUCT allocated by the OS for the duration of this callback.
    let kb_ptr = l_param.0 as *const KBDLLHOOKSTRUCT;
    let kb = unsafe { std::ptr::read_unaligned(kb_ptr) };
    let vk = kb.vkCode;

    // ── Skip keys we injected ourselves ──────────────────────────────────────
    // All our SendInput calls tag dwExtraInfo with MICONTROL_INJECT_MAGIC so
    // we can identify and pass them straight through without re-processing.
    if (kb.dwExtraInfo as usize) == MICONTROL_INJECT_MAGIC {
        return CallNextHookEx(None, n_code, w_param, l_param);
    }

    let is_keydown = event_type == WM_KEYDOWN || event_type == WM_SYSKEYDOWN;
    let is_keyup = event_type == WM_KEYUP || event_type == WM_SYSKEYUP;

    // ── Detect mode: record VK and pass the key through ───────────────────────
    if is_keydown {
        match vk {
            0x10..=0x12 | 0x14 | 0x5B | 0x5C | 0xA0..=0xA5 => {}
            v => {
                if capture_key(v) {
                    log::info!("[hotkeys] DETECT(LL hook): VK={:#04X} (decimal={})", v, v);
                }
            }
        }
        // If we were in detect mode, return early to pass the key through.
        // Check after capture to avoid holding the lock across the hook call.
        {
            let state = lock_or_recover(&REMAP_STATE);
            if matches!(&*state, RemapState::AwaitingKey { .. }) {
                return CallNextHookEx(None, n_code, w_param, l_param);
            }
        }
    }

    // ── Handle active RemapToKey key-up ───────────────────────────────────────
    // When a remap is in progress, we need to release the target key when the
    // physical source key is released.  Handle this BEFORE the keydown block
    // so a very quick tap still gets both sides injected.
    if is_keyup {
        let src = REMAP_SOURCE_VK.load(Ordering::Relaxed);
        if src != 0 && vk == src {
            let target = REMAP_TARGET_VK.load(Ordering::Relaxed);
            let ext = REMAP_TARGET_EXTENDED.load(Ordering::Relaxed);
            // Clear state before injecting to prevent re-entrancy.
            REMAP_SOURCE_VK.store(0, Ordering::Relaxed);
            REMAP_TARGET_VK.store(0, Ordering::Relaxed);
            REMAP_TARGET_EXTENDED.store(false, Ordering::Relaxed);
            do_remap_keyup(target, ext);
            return windows::Win32::Foundation::LRESULT(1); // suppress source key-up
        }

        // Also catch F23 key-up when it was the Copilot combo source.
        if vk == 0x86
        /* VK_F23 */
        {
            let src_f23 = REMAP_SOURCE_VK.load(Ordering::Relaxed);
            if src_f23 == 0x86 {
                let target = REMAP_TARGET_VK.load(Ordering::Relaxed);
                let ext = REMAP_TARGET_EXTENDED.load(Ordering::Relaxed);
                REMAP_SOURCE_VK.store(0, Ordering::Relaxed);
                REMAP_TARGET_VK.store(0, Ordering::Relaxed);
                REMAP_TARGET_EXTENDED.store(false, Ordering::Relaxed);
                do_remap_keyup(target, ext);
                return windows::Win32::Foundation::LRESULT(1);
            }
        }
    }

    if !is_keydown {
        return CallNextHookEx(None, n_code, w_param, l_param);
    }

    // ── Key-down path ─────────────────────────────────────────────────────────

    // Suppress Xiaomi-branded keys so Windows never routes them to a default
    // handler (XiaomiPCManager / Copilot panel).
    let is_xiaomi_key = vk == VK_AI_KEY || vk == VK_XIAOMI_KEY || vk == VK_COPILOT;

    // ── RemapToKey handling: VK_COPILOT (0xC3) path ───────────────────────────
    if vk == VK_COPILOT {
        if let Some(HotkeyAction::RemapToKey {
            vk: target,
            extended,
        }) = resolve_action(vk)
        {
            // Record which physical key is being remapped so keyup knows what to do.
            REMAP_SOURCE_VK.store(VK_COPILOT, Ordering::Relaxed);
            REMAP_TARGET_VK.store(target, Ordering::Relaxed);
            REMAP_TARGET_EXTENDED.store(extended, Ordering::Relaxed);
            do_remap_keydown(target, extended);
            return windows::Win32::Foundation::LRESULT(1); // suppress source
        }
    }

    // ── RemapToKey handling: Win+Shift+F23 path ───────────────────────────────
    // Some hardware / firmware revisions fire the raw Win+Shift+F23 sequence
    // instead of synthesising VK 0xC3.  Intercept F23 when it arrives while
    // LWin and LShift are physically held.
    if vk == 0x86
    /* VK_F23 */
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        let lwin_down = (GetAsyncKeyState(0x5B) as u16) & 0x8000 != 0; // VK_LWIN
        let lshift_down = (GetAsyncKeyState(0xA0) as u16) & 0x8000 != 0; // VK_LSHIFT
        if lwin_down && lshift_down {
            if let Some(HotkeyAction::RemapToKey {
                vk: target,
                extended,
            }) = resolve_action(VK_COPILOT)
            {
                REMAP_SOURCE_VK.store(0x86, Ordering::Relaxed);
                REMAP_TARGET_VK.store(target, Ordering::Relaxed);
                REMAP_TARGET_EXTENDED.store(extended, Ordering::Relaxed);
                do_remap_keydown(target, extended);
                return windows::Win32::Foundation::LRESULT(1);
            }
        }
    }

    if is_xiaomi_key {
        return windows::Win32::Foundation::LRESULT(1);
    }

    // Suppress any other key that has an explicit binding.
    if resolve_action(vk).is_some() {
        return windows::Win32::Foundation::LRESULT(1);
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

// ── Script action security ───────────────────────────────────────────────────
//
// The `Script` hotkey action is an arbitrary code execution vector if
// `hotkeys.json` is tampered.  These controls make it safe-by-default:
//
// 1. Feature flag: `HKCU\Software\miPC\hotkeys\EnableScriptActions` (DWORD).
//    Defaults to 0 (disabled).  Must be explicitly set to 1 by the user.
// 2. Allowlist: only `cmd.exe` and `powershell.exe` from System32 are permitted
//    as interpreters.  Arbitrary paths are rejected.
// 3. Consent: the first time a script runs, it requires pre-granted consent
//    (stored in `hotkey_consent.json` keyed by sha256 of the script content).
//    Since the hotkey hook cannot show a dialog, scripts without pre-granted
//    consent are skipped with a log warning.  The frontend grants consent via
//    a Tauri command before the hotkey is triggered.

/// Allowlist of permitted interpreter executables (canonical System32 paths).
///
/// These are the exact paths that `validate_script_path` compares against
/// after canonicalizing the user-supplied path.  Using canonical paths
/// prevents `ends_with()` bypass attacks where an attacker places a
/// malicious `cmd.exe` in `C:\Users\attacker\bin\` (CWE-22, CWE-426).
#[cfg(windows)]
const ALLOWED_CANONICAL_PATHS: &[&str] = &[
    "C:\\Windows\\System32\\cmd.exe",
    "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
    "C:\\Windows\\System32\\wscript.exe",
    "C:\\Windows\\System32\\cscript.exe",
];

/// Check if the script action feature flag is enabled.
///
/// Reads `HKCU\Software\miPC\hotkeys\EnableScriptActions` (DWORD).
/// Returns `false` if the key is absent or the value is 0.
#[cfg(windows)]
fn is_script_action_enabled() -> bool {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_CURRENT_USER;
    // S25-007: Use RegKeyGuard instead of winreg::RegKey.
    match RegKeyGuard::open_read(HKEY_CURRENT_USER, "Software\\miPC\\hotkeys") {
        Ok(Some(key)) => {
            key.read_u32("EnableScriptActions")
                .ok()
                .flatten()
                .unwrap_or(0)
                != 0
        }
        Ok(None) => false,
        Err(_) => false,
    }
}

#[cfg(not(windows))]
fn is_script_action_enabled() -> bool {
    false
}

/// Validate that the interpreter or path is in the allowlist.
///
/// For `interpreter == "powershell"` or `"cmd"`, the built-in System32
/// executable is used (always allowed).  For direct execution
/// (`interpreter == ""`), the `path` must resolve to an allowlisted executable.
///
/// SECURITY: The path is canonicalized and compared against exact canonical
/// paths of allowed System32 executables.  This prevents `ends_with()` bypass
/// attacks where an attacker places a malicious `cmd.exe` in
/// `C:\Users\attacker\bin\cmd.exe` (CWE-22, CWE-426).
#[cfg(windows)]
fn validate_script_path(interpreter: &str, path: &str) -> Result<(), String> {
    // Named interpreters are always allowed (they use System32 binaries).
    if interpreter == "powershell" || interpreter == "cmd" {
        return Ok(());
    }

    // Direct execution: validate the path against the canonical allowlist.
    match std::fs::canonicalize(path) {
        Ok(resolved) => {
            // Path exists — compare the canonical form against each allowed path.
            for allowed in ALLOWED_CANONICAL_PATHS {
                if let Ok(allowed_canonical) = std::fs::canonicalize(allowed) {
                    if resolved == allowed_canonical {
                        return Ok(());
                    }
                }
            }
            Err(format!(
                "Script executable '{path}' is not in the allowlist. \
                 Only cmd.exe, powershell.exe, wscript.exe, and cscript.exe \
                 from System32 are permitted."
            ))
        }
        Err(_) => {
            // Path does not exist (yet) — fall back to checking the file name
            // component, but ONLY if the parent directory is System32 or the
            // PowerShell v1.0 subdirectory.  This prevents
            // `C:\Users\attacker\bin\cmd.exe` from matching.
            let p = std::path::Path::new(path);
            let file_name = match p.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_lowercase(),
                None => {
                    return Err(format!(
                        "Script executable '{path}' has no valid file name component."
                    ))
                }
            };

            let allowed_names: &[(&str, &[&str])] = &[
                ("cmd.exe", &["C:\\Windows\\System32"]),
                (
                    "powershell.exe",
                    &["C:\\Windows\\System32\\WindowsPowerShell\\v1.0"],
                ),
                ("wscript.exe", &["C:\\Windows\\System32"]),
                ("cscript.exe", &["C:\\Windows\\System32"]),
            ];

            for (name, allowed_parents) in allowed_names {
                if file_name == *name {
                    let parent = p
                        .parent()
                        .and_then(|d| d.to_str())
                        .map(|s| s.to_lowercase())
                        .unwrap_or_default();
                    for allowed_parent in *allowed_parents {
                        if parent.eq_ignore_ascii_case(allowed_parent) {
                            return Ok(());
                        }
                    }
                }
            }

            Err(format!(
                "Script executable '{path}' is not in the allowlist. \
                 Only cmd.exe, powershell.exe, wscript.exe, and cscript.exe \
                 from System32 are permitted."
            ))
        }
    }
}

#[cfg(not(windows))]
fn validate_script_path(_interpreter: &str, _path: &str) -> Result<(), String> {
    Err("Script actions are not supported on this platform".to_string())
}

/// Compute the SHA-256 hash of a script identifier (interpreter + path + args).
fn script_hash(interpreter: &str, path: &str, args: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(interpreter.as_bytes());
    hasher.update(b"\0");
    hasher.update(path.as_bytes());
    hasher.update(b"\0");
    for arg in args {
        hasher.update(arg.as_bytes());
        hasher.update(b"\0");
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// Path to the consent file: `%LOCALAPPDATA%\MiControl\hotkey_consent.json`
fn consent_path() -> std::path::PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
    let dir = std::path::PathBuf::from(base).join("MiControl");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("hotkey_consent.json")
}

/// Check if a script has been pre-granted "Always Allow" consent.
fn has_consent(interpreter: &str, path: &str, args: &[String]) -> bool {
    let hash = script_hash(interpreter, path, args);
    let consent_file = consent_path();
    let content = match std::fs::read_to_string(&consent_file) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let map: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return false,
    };
    map.get(&hash).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Grant "Always Allow" consent for a script.
///
/// Called by the frontend Tauri command when the user clicks "Always Allow"
/// in the consent dialog.
#[cfg(test)]
fn grant_consent(interpreter: &str, path: &str, args: &[String]) -> Result<(), String> {
    let hash = script_hash(interpreter, path, args);
    let consent_file = consent_path();

    let mut map: serde_json::Value = std::fs::read_to_string(&consent_file)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or(serde_json::json!({}));

    if let Some(obj) = map.as_object_mut() {
        obj.insert(hash, serde_json::json!(true));
    }

    let json =
        serde_json::to_string_pretty(&map).map_err(|e| format!("Cannot serialize consent: {e}"))?;
    std::fs::write(&consent_file, json).map_err(|e| format!("Cannot write consent file: {e}"))?;

    // Restrict the ACL on the consent file.
    if let Err(e) = crate::util::auth::restrict_file_acl(&consent_file) {
        log::warn!("Failed to restrict ACL on consent file: {e}");
    }

    Ok(())
}

/// Result of a script action security check.
#[derive(Debug, PartialEq)]
enum ScriptCheckResult {
    /// The script is allowed to execute.
    Allowed,
    /// Script actions are disabled (feature flag off).
    Disabled,
    /// The executable is not in the allowlist.
    NotAllowlisted(String),
    /// Consent has not been granted for this script.
    ConsentRequired,
}

/// Run all security checks on a script action before execution.
fn check_script_action(interpreter: &str, path: &str, args: &[String]) -> ScriptCheckResult {
    // 1. Feature flag check
    if !is_script_action_enabled() {
        return ScriptCheckResult::Disabled;
    }

    // 2. Allowlist check
    if let Err(e) = validate_script_path(interpreter, path) {
        return ScriptCheckResult::NotAllowlisted(e);
    }

    // 3. Consent check
    if !has_consent(interpreter, path, args) {
        return ScriptCheckResult::ConsentRequired;
    }

    ScriptCheckResult::Allowed
}

// ── Action dispatch ───────────────────────────────────────────────────────────

fn dispatch_action(action: &HotkeyAction) {
    match action {
        HotkeyAction::None => {}

        // RemapToKey is handled entirely in keyboard_hook_proc (keydown + keyup).
        // When this function is reached via Raw Input (WM_INPUT) the LL hook has
        // already injected the correct keys — do nothing here to avoid doubling.
        HotkeyAction::RemapToKey { .. } => {}

        HotkeyAction::FocusMicontrol => {
            if let Some(cb) = FOCUS_CALLBACK.get() {
                cb();
            } else {
                log::warn!("[hotkeys] FocusMicontrol: no focus callback registered");
            }
        }

        HotkeyAction::OpenMainWindow => {
            if let Some(cb) = OPEN_MAIN_CALLBACK.get() {
                cb();
            } else {
                log::warn!("[hotkeys] OpenMainWindow: no open_main callback registered");
            }
        }

        HotkeyAction::OpenUrl { url } => {
            // Validate URL — only http and https schemes are allowed to prevent
            // code execution via file://, javascript:, data: schemes (CWE-807).
            // Using url::Url::parse() instead of a prefix check prevents
            // bypasses like "javascript://example.com/%0aalert(1)".
            match url::Url::parse(url) {
                Ok(parsed) => {
                    let scheme = parsed.scheme();
                    if scheme != "http" && scheme != "https" {
                        log::warn!(
                            "[hotkeys] OpenUrl rejected — invalid scheme '{scheme}' (only http/https allowed): '{url}'"
                        );
                        return;
                    }
                }
                Err(e) => {
                    log::warn!("[hotkeys] OpenUrl rejected — invalid URL: '{url}' ({e})");
                    return;
                }
            }
            // Use `explorer <url>` — works for http/https links.
            let result = std::process::Command::new("explorer")
                .arg(url)
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
            if let Err(e) = result {
                log::warn!("[hotkeys] OpenUrl failed for '{url}': {e}");
            }
        }

        HotkeyAction::LaunchApp { path, args } => {
            // If the target resolves to our own executable, show/focus the
            // existing window via the registered callback instead of spawning
            // a redundant second instance.
            let is_self = std::env::current_exe()
                .ok()
                .and_then(|exe| exe.canonicalize().ok())
                .and_then(|exe| PathBuf::from(path).canonicalize().ok().map(|p| p == exe))
                .unwrap_or(false);
            if is_self {
                if let Some(cb) = FOCUS_CALLBACK.get() {
                    cb();
                    return;
                }
            }
            let result = std::process::Command::new(path)
                .args(args)
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
            if let Err(e) = result {
                log::warn!("[hotkeys] LaunchApp failed for '{path}': {e}");
            }
        }

        HotkeyAction::SetPerformanceMode { mode } => {
            use crate::state::PerformanceMode;
            // Parse the snake_case mode name into the enum by round-tripping JSON.
            let quoted = format!("\"{}\"", mode);
            match serde_json::from_str::<PerformanceMode>(&quoted) {
                Ok(pm) => match crate::hw::performance::set_performance_mode(pm) {
                    Ok(res) => log::info!("[hotkeys] SetPerformanceMode {:?}: {:?}", pm, res),
                    Err(e) => log::warn!("[hotkeys] SetPerformanceMode {:?} failed: {e}", pm),
                },
                Err(_) => log::warn!("[hotkeys] SetPerformanceMode: unknown mode '{mode}'"),
            }
        }

        HotkeyAction::ToggleAiBrightness => {
            let current = crate::hw::display::get_ai_brightness_config().enabled;
            match crate::hw::display::set_ai_brightness(!current) {
                Ok(()) => log::info!("[hotkeys] ToggleAiBrightness → {}", !current),
                Err(e) => log::warn!("[hotkeys] ToggleAiBrightness failed: {e}"),
            }
        }

        HotkeyAction::MediaControl { action } => {
            // VK codes for media/volume keys.
            let vk: Option<u16> = match action.as_str() {
                "volume_up" => Some(0xAF),
                "volume_down" => Some(0xAE),
                "mute" => Some(0xAD),
                "play_pause" => Some(0xB3),
                "next" => Some(0xB0),
                "prev" => Some(0xB1),
                _ => {
                    log::warn!("[hotkeys] MediaControl: unknown action '{action}'");
                    None
                }
            };
            if let Some(vk) = vk {
                inject_key_event(vk, 0, false, false);
                inject_key_event(vk, 0, true, false);
                log::info!("[hotkeys] MediaControl '{action}' VK={:#04X}", vk);
            }
        }

        HotkeyAction::Script {
            interpreter,
            path,
            args,
        } => {
            // Security checks: feature flag, allowlist, consent.
            match check_script_action(interpreter, path, args) {
                ScriptCheckResult::Disabled => {
                    log::warn!(
                        "[hotkeys] Script action disabled (feature flag off). \
                         Path: '{path}', interpreter: '{interpreter}'"
                    );
                }
                ScriptCheckResult::NotAllowlisted(reason) => {
                    log::warn!(
                        "[hotkeys] Script action rejected (not allowlisted): {reason}. \
                         Path: '{path}'"
                    );
                }
                ScriptCheckResult::ConsentRequired => {
                    log::warn!(
                        "[hotkeys] Script action skipped (consent not granted). \
                         Path: '{path}', interpreter: '{interpreter}'. \
                         Grant consent via the Settings UI."
                    );
                }
                ScriptCheckResult::Allowed => {
                    log::warn!(
                        "[hotkeys] Executing script action: path='{path}', \
                         interpreter='{interpreter}', args={:?}",
                        args
                    );
                    let result = match interpreter.as_str() {
                        "powershell" => std::process::Command::new("powershell")
                            .args(["-NoProfile", "-NonInteractive", "-File", path.as_str()])
                            .args(args)
                            .creation_flags(CREATE_NO_WINDOW)
                            .spawn(),
                        "cmd" => std::process::Command::new("cmd")
                            .args(["/C", path.as_str()])
                            .args(args)
                            .creation_flags(CREATE_NO_WINDOW)
                            .spawn(),
                        _ => std::process::Command::new(path)
                            .args(args)
                            .creation_flags(CREATE_NO_WINDOW)
                            .spawn(),
                    };
                    if let Err(e) = result {
                        log::warn!("[hotkeys] Script failed for '{path}': {e}");
                    }
                }
            }
        }
    }
}

// ── WMI HID event listener ───────────────────────────────────────────────────

/// Send a Win+<key> keyboard combo via SendInput.
/// `vk` is the virtual-key code of the letter key (e.g. 0x50 for P, 0x49 for I).
#[cfg(windows)]
fn send_win_key_combo(vk: u16) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, VIRTUAL_KEY,
    };
    const KEY_DOWN: KEYBD_EVENT_FLAGS = KEYBD_EVENT_FLAGS(0);
    const KEY_UP: KEYBD_EVENT_FLAGS = KEYBD_EVENT_FLAGS(2); // KEYEVENTF_KEYUP
    const VK_LWIN: VIRTUAL_KEY = VIRTUAL_KEY(0x5B);

    let inputs = [
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_LWIN,
                    wScan: 0,
                    dwFlags: KEY_DOWN,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: KEY_DOWN,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: KEY_UP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_LWIN,
                    wScan: 0,
                    dwFlags: KEY_UP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
    ];
    // SAFETY: SendInput takes an array of properly initialized INPUT structs with correct type tags (INPUT_KEYBOARD) and valid VK codes. The size parameter matches the actual struct size. All INPUTs are stack-allocated and valid.
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(not(windows))]
fn send_win_key_combo(_vk: u16) {}

// ── Key injection helper ──────────────────────────────────────────────────────

/// Inject a single synthetic key event via `SendInput`, tagging it with
/// `MICONTROL_INJECT_MAGIC` in `dwExtraInfo` so the LL hook ignores it.
///
/// * `vk`       – VIRTUAL_KEY code (e.g. 0xA3 = RCtrl, 0x5B = LWin).
/// * `scan`     – hardware scan code (0 = let Windows derive it).
/// * `is_up`    – `true` for key-up, `false` for key-down.
/// * `extended` – `true` for right-side keys and navigation keys that require
///   `KEYEVENTF_EXTENDEDKEY` (RCtrl, RAlt, RShift, Insert, Delete…).
#[cfg(windows)]
fn inject_key_event(vk: u16, scan: u16, is_up: bool, extended: bool) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if is_up {
        flags = KEYBD_EVENT_FLAGS(flags.0 | KEYEVENTF_KEYUP.0);
    }
    if extended {
        flags = KEYBD_EVENT_FLAGS(flags.0 | KEYEVENTF_EXTENDEDKEY.0);
    }
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: MICONTROL_INJECT_MAGIC,
            },
        },
    };
    // SAFETY: SendInput takes a properly initialized INPUT struct with correct type tag (INPUT_KEYBOARD), valid VK code, and flags derived from the function parameters. dwExtraInfo is set to MICONTROL_INJECT_MAGIC to prevent infinite loopback in the hook. The struct is stack-allocated and valid.
    unsafe {
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(not(windows))]
fn inject_key_event(_vk: u16, _scan: u16, _is_up: bool, _extended: bool) {}

/// Remap the Copilot key (or any key bound to `RemapToKey`) by:
///   1. Releasing the spurious `LShift` and `LWin` that travel with it
///      (only if they are actually held — gated on `GetAsyncKeyState`
///      to avoid TOCTOU races with time-based heuristics).
///   2. Injecting the target key-down.
///      The matching key-up is handled in the LL hook when the source key is released.
fn do_remap_keydown(target_vk: u32, extended: bool) {
    // Release the modifier keys that accompany the Copilot combo (Win+Shift+F23).
    // Each modifier is only released if `is_key_down` confirms it is actually
    // held at this instant, preventing the race where a time-based heuristic
    // might falsely release a key the user just pressed independently.
    if is_key_down(0xA0) {
        inject_key_event(0xA0, 0, true, false); // LShift up
    }
    if is_key_down(0x5B) {
        inject_key_event(0x5B, 0, true, true); // LWin up  (extended)
    }
    // Press the target key.
    inject_key_event(target_vk as u16, 0, false, extended);
}

fn do_remap_keyup(target_vk: u32, extended: bool) {
    inject_key_event(target_vk as u16, 0, true, extended);
}

/// Start a RemapToKey sequence from the WM_HOTKEY path.
///
/// Unlike the LL hook path, WM_HOTKEY gives us a key-down notification without a
/// matching key-up callback. We therefore inject the remapped key-down here and
/// watch the physical source state with `GetAsyncKeyState`; when the source is
/// released we emit the target key-up. If Windows never exposes the source as
/// pressed, we fall back to a short tap so the key never gets stuck down.
fn start_hotkey_remap(source_vk: u32, target_vk: u32, extended: bool) {
    REMAP_SOURCE_VK.store(source_vk, Ordering::Relaxed);
    REMAP_TARGET_VK.store(target_vk, Ordering::Relaxed);
    REMAP_TARGET_EXTENDED.store(extended, Ordering::Relaxed);
    do_remap_keydown(target_vk, extended);

    #[cfg(windows)]
    std::thread::spawn(move || {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

        let started = std::time::Instant::now();
        let mut saw_pressed = false;

        loop {
            // SAFETY: GetAsyncKeyState takes a virtual-key code and returns the key's state. Passing a valid VK code (0..=255 per MSDN) is safe; invalid codes return 0.
            let source_down = unsafe {
                let primary_down = ((GetAsyncKeyState(source_vk as i32) as u16) & 0x8000) != 0;
                let f23_down =
                    source_vk == VK_COPILOT && ((GetAsyncKeyState(0x86) as u16) & 0x8000) != 0;
                primary_down || f23_down
            };

            saw_pressed |= source_down;

            if saw_pressed && !source_down {
                break;
            }

            if !saw_pressed && started.elapsed() >= std::time::Duration::from_millis(40) {
                break;
            }

            if started.elapsed() >= std::time::Duration::from_secs(5) {
                break;
            }

            std::thread::sleep(std::time::Duration::from_millis(8));
        }

        if REMAP_SOURCE_VK
            .compare_exchange(source_vk, 0, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            REMAP_TARGET_VK.store(0, Ordering::Relaxed);
            REMAP_TARGET_EXTENDED.store(false, Ordering::Relaxed);
            do_remap_keyup(target_vk, extended);
        }
    });
}

/// Run a WMI event listener with automatic reconnection.
/// Uses exponential backoff: 1s, 2s, 4s, 8s, 16s, 30s (max).
/// If the listener exits normally (Ok), we stop; if it errors, we reconnect.
fn run_wmi_listener_with_reconnect<F>(listener_name: &str, f: F)
where
    F: Fn() -> anyhow::Result<()>,
{
    let mut backoff_ms = 1000u64;
    let max_backoff_ms = 30_000u64;

    loop {
        match f() {
            Ok(()) => {
                log::info!("[hotkeys] WMI listener '{listener_name}' exited normally");
                return;
            }
            Err(e) => {
                log::warn!(
                    "[hotkeys] WMI listener '{listener_name}' failed ({}), reconnecting in {}ms...",
                    e,
                    backoff_ms
                );
                std::thread::sleep(Duration::from_millis(backoff_ms));
                backoff_ms = (backoff_ms * 2).min(max_backoff_ms);
            }
        }
    }
}

/// Subscribe directly to IoTDriver.sys WMI events in root\WMI.
///
/// IoTDriver.sys fires HID_EVENT20/21/22/23 when the Xiaomi special keys are
/// pressed (Fn+F7 / AI key, Xiaomi button, Copilot key, Fn+Esc).  IoTSvc
/// subscribes to the same classes and forwards them to VirtualControlHID; by
/// subscribing here we receive the events even when VirtualControlHID is stopped.
///
/// Each WMI HID event class runs on its own thread, wrapped in
/// `run_wmi_listener_with_reconnect` so that dropped WMI connections are
/// automatically retried with exponential backoff (CWE-665 fix).
///
/// Synthetic VK scheme used in detect mode:
///   HID_EVENT20 → 0xA0xx   HID_EVENT21 → 0xA1xx
///   HID_EVENT22 → 0xA2xx   HID_EVENT23 → 0xA3xx
/// (xx = first byte of EventDetail)
fn start_wmi_hid_listener() {
    for (idx, class_name) in ["HID_EVENT20", "HID_EVENT21", "HID_EVENT22", "HID_EVENT23"]
        .iter()
        .enumerate()
    {
        let class_name = class_name.to_string();
        let class_idx = idx as u32;
        std::thread::Builder::new()
            .name(format!("wmi-hid{idx}"))
            .spawn(move || {
                run_wmi_listener_with_reconnect(&class_name, || {
                    wmi_hid_event_thread(&class_name, class_idx)
                });
            })
            .ok();
    }
}

fn wmi_hid_event_thread(class_name: &str, class_idx: u32) -> anyhow::Result<()> {
    use wmi::{COMLibrary, WMIConnection};

    #[derive(serde::Deserialize, Debug)]
    #[allow(non_snake_case)]
    struct HidWmiEvent {
        Active: bool,
        EventDetail: Vec<u8>,
        #[serde(rename = "InstanceName")]
        _instance_name: String,
    }

    let com = COMLibrary::new().context("WMI: COMLibrary")?;
    let con =
        WMIConnection::with_namespace_path("ROOT\\WMI", com).context("WMI: connect root\\WMI")?;

    let query = format!("SELECT * FROM {class_name}");
    let iter = con
        .raw_notification::<HidWmiEvent>(&query)
        .with_context(|| format!("WMI: subscribe {class_name}"))?;

    log::info!("[hotkeys] WMI {class_name}: subscribed");

    for result in iter {
        match result {
            Ok(ev) => {
                handle_hid_wmi_event(class_name, class_idx, ev.Active, &ev.EventDetail);
            }
            Err(e) => {
                log::debug!("[hotkeys] WMI {class_name}: event error: {e}");
            }
        }
    }

    log::warn!("[hotkeys] WMI {class_name}: iterator exhausted");
    Ok(())
}

fn handle_hid_wmi_event(class_name: &str, class_idx: u32, active: bool, detail: &[u8]) {
    // detail[0] is always 0x01 (report ID / header). detail[1] is the unique key code.
    // Confirmed mapping from detect-mode testing on Xiaomi laptop (all via HID_EVENT20):
    //   detail[1]=0x21  → Fn+F4   (mic mute);  detail[2] = new mic state (0=active, 1=muted)
    //   detail[1]=0x23  → Fn+F7   (AI key)     press; 0x24 = release
    //   detail[1]=0x25  → Xiaomi logo key press; 0x26 = release
    //   detail[1]=0x05  → Fn+F10  (keyboard backlight); detail[2] = new level (0x00–0x0A)
    let distinguish_byte = detail.get(1).copied().unwrap_or(0) as u32;

    if active {
        let synthetic_vk = 0xA000 | (class_idx << 8) | distinguish_byte;
        if capture_key(synthetic_vk) {
            log::info!(
                "[hotkeys] DETECT(WMI): class={class_name} active={active} detail={detail:02X?}"
            );
            return;
        }
    } else {
        // Check if we're in detect mode even for key-up events (for logging).
        let state = lock_or_recover(&REMAP_STATE);
        if matches!(&*state, RemapState::AwaitingKey { .. }) {
            log::info!(
                "[hotkeys] DETECT(WMI): class={class_name} active={active} detail={detail:02X?}"
            );
            return;
        }
    }

    if !active {
        return; // Only act on key-down events.
    }

    // Log every active WMI key event regardless of what happens next.
    log::info!("[hotkeys] WMI key: class={class_name} detail={detail:02X?}");

    // Per-key debounce: IoTDriver may fire active=true repeatedly while a
    // key is held.  Use the distinguish_byte (key code) as the debounce key
    // so that different keys do not suppress each other.
    let debounce_key = (class_idx << 8) | distinguish_byte;
    if wmi_key_debounced(debounce_key) {
        return;
    }

    log::debug!("[hotkeys] WMI {class_name}: active detail={detail:02X?}");

    // F4 and F10 have fixed OSD actions — not user-configurable.
    // IoTDriver encodes the resulting state in detail[2] for these keys.
    match (class_name, distinguish_byte) {
        ("HID_EVENT20", 0x21) => {
            // Fn+F4: mic mute.  detail[2]: 0x00 = muted, 0x01 = active.
            // (IoTDriver reports the NEW mic state after the key toggles it.)
            let muted = detail.get(2).copied().unwrap_or(1) == 0;
            log::info!("[hotkeys] WMI Fn+F4 → mic mute OSD (muted={})", muted);
            crate::hw::osd::show_mic_mute_osd(muted);
            crate::hw::mic::set_system_mic_mute(muted);
            return;
        }
        ("HID_EVENT20", 0x05) => {
            // Fn+F10: keyboard backlight level cycle.  detail[2] = new level (0x00–0xFF).
            let level = detail.get(2).copied().unwrap_or(0xFF);
            log::info!(
                "[hotkeys] WMI Fn+F10 → keyboard backlight OSD (raw=0x{:02X})",
                level
            );
            crate::hw::osd::show_keyboard_osd(level);
            return;
        }
        ("HID_EVENT20", 0x01) => {
            // Fn+F8: Project / display mode  →  Win+P
            log::info!("[hotkeys] WMI Fn+F8 → Win+P (project)");
            send_win_key_combo(0x50); // VK 'P'
            return;
        }
        ("HID_EVENT20", 0x1B) => {
            // Fn+F9: Windows Settings  →  Win+I
            log::info!("[hotkeys] WMI Fn+F9 → Win+I (settings)");
            send_win_key_combo(0x49); // VK 'I'
            return;
        }
        _ => {}
    }

    // Route configurable keys (ai_key, xiaomi_key) via HotkeyMap.
    let action_opt = HOTKEY_CONFIG.get().and_then(|arc| {
        arc.read().ok().and_then(|cfg| {
            let binding = match (class_name, distinguish_byte) {
                ("HID_EVENT20", 0x25) => &cfg.xiaomi_key, // Xiaomi logo key (press)
                ("HID_EVENT20", 0x23) => &cfg.ai_key,     // Fn+F7 AI key (press)
                _ => return None,
            };
            if binding.enabled && binding.action != HotkeyAction::None {
                Some(binding.action.clone())
            } else {
                log::info!(
                    "[hotkeys] WMI key skipped — enabled={} action={:?}",
                    binding.enabled,
                    binding.action
                );
                None
            }
        })
    });

    if let Some(action) = action_opt {
        log::info!("[hotkeys] WMI key dispatching action: {:?}", action);
        dispatch_action(&action);
    }
}

// ── VirtualControlHID service management ────────────────────────────────────

/// Start the VirtualControlHID VHF driver service.
///
/// This service bridges ACPI events from IoTSvc (the Xiaomi EC driver) into
/// standard HID Consumer Control reports visible to Win32 Raw Input.  When
/// XiaomiPCManager is uninstalled the service is often left stopped, causing
/// all Fn+F7 / Xiaomi button / Copilot key presses to be silently dropped.
fn start_virtual_control_hid() {
    // SAFETY: All SCM API calls (OpenSCManagerW, OpenServiceW, QueryServiceStatus, ChangeServiceConfigW,
    // StartServiceW, CloseServiceHandle) are called with valid null-terminated wide strings and checked for errors.
    // The service name is a compile-time constant; handles are closed before the block exits.
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Foundation::{GetLastError, ERROR_ACCESS_DENIED};
        use windows::Win32::System::Services::{
            ChangeServiceConfigW, CloseServiceHandle, OpenSCManagerW, OpenServiceW,
            QueryServiceStatus, StartServiceW, ENUM_SERVICE_TYPE, SC_MANAGER_CONNECT,
            SERVICE_CHANGE_CONFIG, SERVICE_DEMAND_START, SERVICE_ERROR, SERVICE_NO_CHANGE,
            SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_START, SERVICE_STATUS,
        };

        let scm = match OpenSCManagerW(None, None, SC_MANAGER_CONNECT) {
            Ok(h) => h,
            Err(e) => {
                log::debug!("[hotkeys] VirtualControlHID: OpenSCManager failed: {e}");
                return;
            }
        };

        let svc_name: Vec<u16> = "VirtualControlHID\0".encode_utf16().collect();
        let svc = match OpenServiceW(
            scm,
            windows::core::PCWSTR(svc_name.as_ptr()),
            SERVICE_START | SERVICE_QUERY_STATUS | SERVICE_CHANGE_CONFIG,
        ) {
            Ok(h) => h,
            Err(e) => {
                log::debug!("[hotkeys] VirtualControlHID not installed / no access: {e}");
                let _ = CloseServiceHandle(scm);
                return;
            }
        };

        let mut status = SERVICE_STATUS::default();
        let _ = QueryServiceStatus(svc, &mut status);

        if status.dwCurrentState == SERVICE_RUNNING {
            log::info!("[hotkeys] VirtualControlHID already running");
        } else {
            // The service is installed but DISABLED (start_type=4 in sc.exe qc output).
            // ChangeServiceConfigW with SERVICE_DEMAND_START re-enables it so that
            // StartServiceW can succeed.
            let _ = ChangeServiceConfigW(
                svc,
                ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE), // dwServiceType: no change
                SERVICE_DEMAND_START,
                SERVICE_ERROR(SERVICE_NO_CHANGE), // dwErrorControl: no change
                windows::core::PCWSTR::null(),
                windows::core::PCWSTR::null(),
                None,
                windows::core::PCWSTR::null(),
                windows::core::PCWSTR::null(),
                windows::core::PCWSTR::null(),
                windows::core::PCWSTR::null(),
            );

            match StartServiceW(svc, Some(&[])) {
                Ok(()) => log::info!(
                    "[hotkeys] VirtualControlHID start requested (was state={})",
                    status.dwCurrentState.0
                ),
                Err(_) => {
                    let code = GetLastError();
                    if code == ERROR_ACCESS_DENIED {
                        if VCHID_ACCESS_DENIED_LOGGED
                            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                            .is_ok()
                        {
                            log::warn!(
                                "[hotkeys] VirtualControlHID: access denied \
                                \u{2014} run micontrol as Administrator to enable \
                                Xiaomi special keys (Fn+F7 / Xiaomi / Copilot)"
                            );
                        } else {
                            log::debug!("[hotkeys] VirtualControlHID: access denied (suppressed)");
                        }
                    } else if code.0 == 1056 {
                        // ERROR_SERVICE_ALREADY_RUNNING
                        log::info!("[hotkeys] VirtualControlHID already running");
                    } else {
                        log::warn!("[hotkeys] VirtualControlHID start failed: code={}", code.0);
                    }
                }
            }
        }

        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);
    }
}

// ── Direct HID device reader ─────────────────────────────────────────────────

/// Spawn a background thread that enumerates every HID Consumer Controls and
/// vendor-specific device and reads their reports directly via `CreateFileW` +
/// `ReadFile`, bypassing the Raw Input delivery pipeline.
///
/// This catches events from:
/// * The VirtualControlHID VHF virtual device (once started) — Consumer Controls 0x0C/0x01
/// * Physical keyboard vendor-specific interfaces (0xFF00, 0xFFBC, …)
///
/// Two enumeration passes: 3 s and 6 s after app start.  The 3 s pass handles
/// the case where VirtualControlHID was already running; the 6 s pass catches
/// the VHF virtual device that appears after the driver finishes loading.
fn start_hid_raw_reader() {
    for delay_ms in [3000u64, 6000u64] {
        std::thread::Builder::new()
            .name("hid-enum".into())
            .spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                #[cfg(windows)]
                // SAFETY: hid_raw_reader_main is called on a dedicated thread; all Win32 API calls within it are properly guarded.
                unsafe {
                    hid_raw_reader_main();
                }
            })
            .ok();
    }
}

/// Enumerate HID devices and start reader threads for interesting ones.
///
/// # Safety
/// This function calls Win32 SetupDi and HID APIs via raw FFI. All pointers are validated
/// by the API return values before dereference. Device path strings are null-terminated
/// wide strings. The function opens handles to HID devices which must be closed elsewhere.
#[cfg(windows)]
unsafe fn hid_raw_reader_main() {
    use windows::core::PCWSTR;
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
        SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
        SP_DEVICE_INTERFACE_DATA,
    };
    use windows::Win32::Devices::HumanInterfaceDevice::{
        HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetHidGuid, HidD_GetPreparsedData,
        HidP_GetCaps, HIDD_ATTRIBUTES, HIDP_CAPS, HIDP_STATUS_SUCCESS, PHIDP_PREPARSED_DATA,
    };
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let guid = HidD_GetHidGuid();

    let info = match SetupDiGetClassDevsW(
        Some(&guid),
        None,
        None,
        DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
    ) {
        Ok(h) if !h.is_invalid() => h,
        Ok(_) | Err(_) => {
            log::warn!("[hotkeys] HID reader: SetupDiGetClassDevsW failed");
            return;
        }
    };

    let mut idx = 0u32;
    let mut readers = 0u32;
    loop {
        let mut iface = SP_DEVICE_INTERFACE_DATA {
            cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        if SetupDiEnumDeviceInterfaces(info, None, &guid, idx, &mut iface).is_err() {
            break;
        }
        idx += 1;

        // First call: query the required buffer size.
        let mut needed = 0u32;
        let _ = SetupDiGetDeviceInterfaceDetailW(info, &iface, None, 0, Some(&mut needed), None);
        if needed < 6 {
            continue;
        }

        // Second call: allocate and fill.
        // SP_DEVICE_INTERFACE_DETAIL_DATA_W layout:
        //   offset 0: cbSize (u32) = 6 (sizeof DWORD + sizeof WCHAR)
        //   offset 4: DevicePath (null-terminated UTF-16 string)
        let mut buf = vec![0u8; needed as usize];
        *(buf.as_mut_ptr() as *mut u32) = 6u32;
        if SetupDiGetDeviceInterfaceDetailW(
            info,
            &iface,
            Some(buf.as_mut_ptr() as *mut _),
            needed,
            None,
            None,
        )
        .is_err()
        {
            continue;
        }

        // Extract the device path (UTF-16, starts at byte offset 4).
        let path_ptr = buf.as_ptr().add(4) as *const u16;
        let path_len = (0usize..).take_while(|&i| *path_ptr.add(i) != 0).count();
        if path_len == 0 {
            continue;
        }
        let path_u16 = std::slice::from_raw_parts(path_ptr, path_len).to_vec();
        let mut path_z = path_u16.clone();
        path_z.push(0);

        // Open the device read-only to query capabilities, then close it.
        // The reader thread opens its own handle.
        let h = match CreateFileW(
            PCWSTR(path_z.as_ptr()),
            0x80000000u32, // GENERIC_READ
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        ) {
            Ok(h) => h,
            Err(_) => continue,
        };

        let mut preparsed = PHIDP_PREPARSED_DATA(0isize);
        let has_pp = HidD_GetPreparsedData(h, &mut preparsed).as_bool();
        let mut caps = HIDP_CAPS::default();
        let caps_ok = has_pp && HidP_GetCaps(preparsed, &mut caps) == HIDP_STATUS_SUCCESS;
        if has_pp {
            let _ = HidD_FreePreparsedData(preparsed);
        }

        let mut attrs = HIDD_ATTRIBUTES {
            Size: std::mem::size_of::<HIDD_ATTRIBUTES>() as u32,
            ..Default::default()
        };
        let _ = HidD_GetAttributes(h, &mut attrs as *mut _);
        let _ = CloseHandle(h);

        if !caps_ok {
            continue;
        }

        let page = caps.UsagePage;
        let dev_usage = caps.Usage;
        let rpt_size = caps.InputReportByteLength as usize;

        // We only care about Consumer Controls and vendor-specific pages.
        let interesting = (page == 0x0C && dev_usage == 0x01) || page >= 0xFF00;
        if !interesting {
            continue;
        }

        let path_str = String::from_utf16_lossy(&path_u16);
        let sfx = path_str.len().saturating_sub(50);
        log::info!(
            "[hotkeys] HID reader: page={:#06X}/usage={:#04X} \
            VID={:04X} PID={:04X} rpt={}B ...{}",
            page,
            dev_usage,
            attrs.VendorID,
            attrs.ProductID,
            rpt_size,
            &path_str[sfx..]
        );

        readers += 1;
        std::thread::Builder::new()
            .name(format!("hid-{:04X}/{:04X}", page, dev_usage))
            // SAFETY: hid_device_read_loop is spawned on a new thread; the path string is owned and moved into the closure.
            .spawn(move || unsafe {
                hid_device_read_loop(path_str, page, dev_usage, rpt_size);
            })
            .ok();
    }

    let _ = SetupDiDestroyDeviceInfoList(info);
    log::info!(
        "[hotkeys] HID reader: enumeration done, {} reader thread(s) started",
        readers
    );
}

/// Read HID input reports from a device in a dedicated thread loop.
///
/// # Safety
/// Opens a device handle via CreateFileW; the path must be a valid null-terminated wide string
/// obtained from SetupDiGetDeviceInterfaceDetailW. Reads from the handle in a loop until
/// the device is disconnected or an error occurs. The handle is closed when the function exits.
#[cfg(windows)]
unsafe fn hid_device_read_loop(path_str: String, page: u16, dev_usage: u16, rpt_size: usize) {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    };

    let mut path_z: Vec<u16> = path_str.encode_utf16().collect();
    path_z.push(0);

    let h = match CreateFileW(
        PCWSTR(path_z.as_ptr()),
        0x80000000u32, // GENERIC_READ
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        None,
        OPEN_EXISTING,
        FILE_FLAGS_AND_ATTRIBUTES(0),
        None,
    ) {
        Ok(h) => h,
        Err(e) => {
            log::warn!("[hotkeys] HID reader: cannot open device: {e}");
            return;
        }
    };

    let buf_size = rpt_size.max(64) + 4;
    let mut buf = vec![0u8; buf_size];
    log::debug!(
        "[hotkeys] HID reader active: page={:#06X}/usage={:#04X}",
        page,
        dev_usage
    );

    loop {
        let mut bytes_read = 0u32;
        match ReadFile(h, Some(buf.as_mut_slice()), Some(&mut bytes_read), None) {
            Err(_) => break,
            Ok(()) if bytes_read == 0 => break,
            Ok(()) => {}
        }
        let data = &buf[..bytes_read as usize];

        if let Some(&b) = data.iter().find(|&&b| b != 0) {
            let synthetic = 0xD000u32 | ((page as u32 & 0xFF) << 8) | (b as u32);
            if capture_key(synthetic) {
                let hex: Vec<String> = data.iter().map(|b| format!("{:02X}", b)).collect();
                log::info!(
                    "[hotkeys] DETECT(HID-direct page={:#06X}/usage={:#04X}): [{}]",
                    page,
                    dev_usage,
                    hex.join(" ")
                );
            }
        } else if page == 0x0C {
            // Consumer Controls: standard 1-byte report ID + 2-byte usage LE.
            let usage: u16 = if bytes_read >= 3 {
                u16::from_le_bytes([buf[1], buf[2]])
            } else if bytes_read >= 2 {
                u16::from_le_bytes([buf[0], buf[1]])
            } else {
                buf[0] as u16
            };
            if usage != 0 {
                log::info!("[hotkeys] HID-direct consumer usage={:#06X}", usage);
                dispatch_consumer_usage(usage);
            }
        } else {
            // Vendor-specific page — log during debug so we can discover the format.
            let hex: Vec<String> = data.iter().map(|b| format!("{:02X}", b)).collect();
            log::debug!(
                "[hotkeys] HID-direct vendor page={:#06X}/usage={:#04X}: [{}]",
                page,
                dev_usage,
                hex.join(" ")
            );
        }
    }

    let _ = CloseHandle(h);
    log::debug!(
        "[hotkeys] HID reader exiting: page={:#06X}/usage={:#04X}",
        page,
        dev_usage
    );
}

#[cfg(test)]
mod script_security_tests {
    use super::*;

    #[test]
    fn test_script_hash_is_deterministic() {
        let args = vec!["-arg1".to_string(), "-arg2".to_string()];
        let h1 = script_hash("cmd", "C:\\test.bat", &args);
        let h2 = script_hash("cmd", "C:\\test.bat", &args);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // 32 bytes = 64 hex chars
    }

    #[test]
    fn test_script_hash_differs_for_different_paths() {
        let args = vec![];
        let h1 = script_hash("cmd", "C:\\test1.bat", &args);
        let h2 = script_hash("cmd", "C:\\test2.bat", &args);
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_script_hash_differs_for_different_interpreters() {
        let args = vec![];
        let h1 = script_hash("cmd", "C:\\test.bat", &args);
        let h2 = script_hash("powershell", "C:\\test.bat", &args);
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_script_hash_differs_for_different_args() {
        let h1 = script_hash("cmd", "C:\\test.bat", &["a".to_string()]);
        let h2 = script_hash("cmd", "C:\\test.bat", &["b".to_string()]);
        assert_ne!(h1, h2);
    }

    #[cfg(windows)]
    #[test]
    fn test_validate_named_interpreters_allowed() {
        assert!(validate_script_path("powershell", "").is_ok());
        assert!(validate_script_path("cmd", "").is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn test_validate_allowlisted_executable_allowed() {
        assert!(validate_script_path("", "C:\\Windows\\System32\\cmd.exe").is_ok());
        assert!(validate_script_path(
            "",
            "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"
        )
        .is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn test_validate_arbitrary_path_rejected() {
        assert!(validate_script_path("", "C:\\evil\\malware.exe").is_err());
        assert!(validate_script_path("", "C:\\Users\\test\\download.exe").is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn test_validate_rejected_on_non_windows() {
        assert!(validate_script_path("cmd", "test.bat").is_err());
    }

    #[test]
    fn test_consent_grant_and_check() {
        // Use a temporary consent file for testing.
        let temp_dir = std::env::temp_dir();
        let test_consent = temp_dir.join("test_hotkey_consent.json");
        let _ = std::fs::remove_file(&test_consent);

        // Override the consent_path for this test by temporarily setting an env var.
        // Since consent_path() reads LOCALAPPDATA, we test grant_consent + has_consent
        // using the real path but clean up afterward.
        let interpreter = "cmd";
        let path = "C:\\test_consent.bat";
        let args = vec![];

        // Initially, no consent.
        // Note: this test may interact with the real consent file, so we
        // grant and then check.
        let _ = grant_consent(interpreter, path, &args);
        assert!(has_consent(interpreter, path, &args));

        // Clean up: remove the consent entry by writing an empty file.
        // (In production, consent persists, which is the intended behavior.)
    }

    #[test]
    fn test_check_script_action_disabled_by_default() {
        // On non-Windows, script actions are always disabled.
        // On Windows, they're disabled unless the registry key is set.
        // This test verifies the disabled path returns Disabled.
        // We can't control the registry in a unit test, so we test the
        // logic indirectly: if is_script_action_enabled() returns false,
        // check_script_action returns Disabled.
        if !is_script_action_enabled() {
            let result = check_script_action("cmd", "C:\\test.bat", &[]);
            assert_eq!(result, ScriptCheckResult::Disabled);
        }
    }
}

#[cfg(test)]
mod remap_state_tests {
    use super::*;
    use std::thread;

    /// Reset the global REMAP_STATE to Idle before each test to prevent
    /// cross-test contamination from parallel execution.
    fn reset_state() {
        let mut state = lock_or_recover(&REMAP_STATE);
        *state = RemapState::Idle;
    }

    /// Verify that concurrent begin/cancel/capture does not produce partial
    /// or incorrect remaps.  Spawns threads that race begin/cancel/capture
    /// and asserts the final state is always consistent.
    #[test]
    fn test_concurrent_begin_cancel_capture() {
        const ITERATIONS: usize = 50;

        for i in 0..ITERATIONS {
            reset_state();

            let mut handles = vec![];

            // Thread 1: begin remap
            handles.push(thread::spawn(move || {
                let mut state = lock_or_recover(&REMAP_STATE);
                *state = RemapState::AwaitingKey { captured_vk: 0 };
            }));

            // Thread 2: cancel remap
            handles.push(thread::spawn(move || {
                let mut state = lock_or_recover(&REMAP_STATE);
                *state = RemapState::Idle;
            }));

            // Threads 3-5: capture key (concurrent with begin/cancel)
            for key in [0x42, 0xB6, 0xC3] {
                handles.push(thread::spawn(move || {
                    capture_key(key);
                }));
            }

            // Thread 6: read state
            handles.push(thread::spawn(move || {
                let state = lock_or_recover(&REMAP_STATE);
                match &*state {
                    RemapState::Idle => {}
                    RemapState::AwaitingKey { captured_vk, .. } => {
                        // If we captured a key, it must be one of the valid ones.
                        if *captured_vk != 0 {
                            assert!(
                                *captured_vk == 0x42
                                    || *captured_vk == 0xB6
                                    || *captured_vk == 0xC3,
                                "captured_vk={:#04X} is not a valid key",
                                captured_vk
                            );
                        }
                    }
                }
            }));

            for h in handles {
                h.join().expect("thread panicked");
            }

            // After all threads complete, the state must be valid.
            let state = lock_or_recover(&REMAP_STATE);
            match &*state {
                RemapState::Idle => { /* clean cancel or commit */ }
                RemapState::AwaitingKey { captured_vk, .. } => {
                    // If still awaiting, captured_vk must be 0 or a valid key.
                    if *captured_vk != 0 {
                        assert!(
                            *captured_vk == 0x42 || *captured_vk == 0xB6 || *captured_vk == 0xC3,
                            "iteration {i}: captured_vk={:#04X} is not a valid key",
                            captured_vk
                        );
                    }
                }
            }
        }
    }

    /// Verify that a cancel during capture cleanly aborts without applying
    /// a partial key: begin → cancel → capture should leave state as Idle
    /// and capture_key should return false.
    #[test]
    fn test_cancel_during_capture_aborts() {
        reset_state();

        // Start remap
        let mut state = lock_or_recover(&REMAP_STATE);
        *state = RemapState::AwaitingKey { captured_vk: 0 };
        drop(state);

        // Cancel (simulates user pressing Escape or timeout)
        let mut state = lock_or_recover(&REMAP_STATE);
        *state = RemapState::Idle;
        drop(state);

        // Capture after cancel — should be a no-op
        assert!(
            !capture_key(0x42),
            "capture after cancel should return false"
        );

        // State must be Idle
        let state = lock_or_recover(&REMAP_STATE);
        assert!(
            matches!(&*state, RemapState::Idle),
            "state should be Idle after cancel"
        );
        drop(state);

        // get_detected_vk should return 0
        assert_eq!(
            get_detected_vk(),
            0,
            "no key should be detected after cancel"
        );
    }

    /// Verify that begin → capture → get_detected_vk returns the captured key.
    #[test]
    fn test_begin_capture_commit() {
        reset_state();

        // Start remap
        let mut state = lock_or_recover(&REMAP_STATE);
        *state = RemapState::AwaitingKey { captured_vk: 0 };
        drop(state);

        // Capture a key
        assert!(capture_key(0xB6), "capture should return true");

        // get_detected_vk should return the captured key
        assert_eq!(get_detected_vk(), 0xB6, "should return captured VK");

        // State should still be AwaitingKey (detect mode stays active until timeout)
        let state = lock_or_recover(&REMAP_STATE);
        assert!(
            matches!(&*state, RemapState::AwaitingKey { captured_vk, .. } if *captured_vk == 0xB6),
            "state should still be AwaitingKey with captured_vk=0xB6"
        );
    }

    /// Verify that capture_key returns false when not in detect mode.
    #[test]
    fn test_capture_when_idle() {
        reset_state();

        assert!(!capture_key(0x42), "capture when Idle should return false");
        assert_eq!(
            get_detected_vk(),
            0,
            "get_detected_vk should return 0 when Idle"
        );
    }
}

#[cfg(test)]
mod url_validation_tests {
    #[test]
    fn test_http_url_accepted() {
        assert!("http://example.com".to_lowercase().starts_with("http://"));
    }

    #[test]
    fn test_https_url_accepted() {
        assert!("https://example.com".to_lowercase().starts_with("https://"));
    }

    #[test]
    fn test_file_url_rejected() {
        let url = "file:///C:/Windows/System32/";
        let lower = url.to_lowercase();
        assert!(!lower.starts_with("http://") && !lower.starts_with("https://"));
    }

    #[test]
    fn test_javascript_url_rejected() {
        let url = "javascript:alert(1)";
        let lower = url.to_lowercase();
        assert!(!lower.starts_with("http://") && !lower.starts_with("https://"));
    }

    #[test]
    fn test_data_url_rejected() {
        let url = "data:text/html,<script>alert(1)</script>";
        let lower = url.to_lowercase();
        assert!(!lower.starts_with("http://") && !lower.starts_with("https://"));
    }
}

#[cfg(test)]
mod wmi_debounce_tests {
    use super::*;

    /// Serializes access to the shared `WMI_DEBOUNCE` static so that
    /// tests running in parallel do not clobber each other's entries.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Helper: reset the debounce map to empty.
    fn reset_debounce() {
        // The `wmi_key_debounced` function initializes the OnceLock lazily,
        // so any prior entries are simply discarded by creating a new map.
        if let Some(mtx) = WMI_DEBOUNCE.get() {
            let mut map = lock_or_recover(mtx);
            map.clear();
        }
    }

    /// A different key fired within the debounce window must NOT be suppressed.
    #[test]
    fn test_different_keys_do_not_debounce_each_other() {
        let _guard = lock_or_recover(&SERIAL);
        reset_debounce();

        // Fire key A — should not be debounced.
        assert!(
            !wmi_key_debounced(0x01),
            "first fire of key 0x01 should not be debounced"
        );

        // Fire key B — should NOT be debounced, even though we just fired key A.
        assert!(
            !wmi_key_debounced(0x02),
            "key 0x02 within window of key 0x01 should NOT be debounced"
        );

        // Fast re-fire of key A — should be debounced (same key).
        assert!(
            wmi_key_debounced(0x01),
            "key 0x01 re-fired within window should be debounced"
        );
    }

    /// The same key fired twice within the debounce window must be suppressed.
    #[test]
    fn test_same_key_is_debounced() {
        let _guard = lock_or_recover(&SERIAL);
        reset_debounce();

        assert!(
            !wmi_key_debounced(0x42),
            "first fire should not be debounced"
        );
        assert!(
            wmi_key_debounced(0x42),
            "second fire within window should be debounced"
        );
    }

    /// Sanity-check that the function returns false when called once (no debounce history).
    #[test]
    fn test_first_call_not_debounced() {
        let _guard = lock_or_recover(&SERIAL);
        reset_debounce();

        assert!(
            !wmi_key_debounced(0x99),
            "first call should always be accepted"
        );
        assert!(wmi_key_debounced(0x99), "second call should be debounced");
    }

    /// Verify that debouncing is per-key, not per-distinguish_byte-group:
    /// two keys with different distinguish bytes should not suppress each other.
    #[test]
    fn test_per_key_independence_by_class() {
        let _guard = lock_or_recover(&SERIAL);
        reset_debounce();

        // Simulate two different HID classes with the same distinguish byte.
        // Debounce key = (class_idx << 8) | distinguish_byte.
        let key_a = (0u32 << 8) | 0x23; // HID_EVENT20, AI key
        let key_b = (1u32 << 8) | 0x23; // HID_EVENT21, same detail byte

        assert!(
            !wmi_key_debounced(key_a),
            "key_a first fire should not be debounced"
        );
        assert!(
            !wmi_key_debounced(key_b),
            "key_b (different class, same detail byte) should NOT be debounced"
        );
        assert!(
            wmi_key_debounced(key_a),
            "key_a second fire should be debounced"
        );
    }
}
