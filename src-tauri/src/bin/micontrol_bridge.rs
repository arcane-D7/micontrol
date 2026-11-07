//! MiControl Bridge — autonomous elevated service.
//!
//! A Windows service installed at install time (as `NT AUTHORITY\SYSTEM`).
//! It exposes a named pipe `\\.\pipe\micontrol_bridge` and executes privileged
//! hardware commands on behalf of the main (unprivileged) MiControl process.
//!
//! Because the service runs elevated from installation onward, the main app
//! NEVER needs to prompt UAC for privileged operations — it simply sends the
//! HMAC-authenticated command over the pipe and reads the response.
//!
//! Protocol (JSON line over named pipe):
//!   Request:  { "cmd": "...", "args": {...}, "request_id": "...",
//!               "created_at_ms": 1234, "nonce": "...", "hmac": "..." }
//!   Response: { "ok": true, "data": {...}, "request_id": "...", "hmac": "..." }
//!
//! Auth: HMAC-SHA256 over the JSON body, shared key in
//! `%LOCALAPPDATA%\MiControl\elev_key.bin` (same key as the scheduled-task
//! bridge). The pipe itself is also ACL-restricted to the current user +
//! SYSTEM, and the key file is ACL-restricted, so only the MiControl app can
//! talk to the bridge.
//!
//! CLI usage:
//!   micontrol_bridge service              Run as a Windows service (via SCM)
//!   micontrol_bridge console              Run pipe server in console (testing)
//!   micontrol_bridge install              Install the service (admin required)
//!   micontrol_bridge uninstall            Uninstall the service (admin required)
//!   micontrol_bridge self-test            Validate the pipe DACL (no admin)
//!   micontrol_bridge install-update <file>  Relay a silent update install to the
//!                                       SYSTEM service (no admin — "Ponte Elevada")
//!   micontrol_bridge auto-update [<url>] Download a beta installer (persisted
//!                                       feed or explicit URL) + install via the
//!                                       bridge (dev/CLI/MCP hook — hidden option)

#![cfg(windows)]

use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ── Constants ────────────────────────────────────────────────────────────────

/// Named pipe name for the bridge service.
pub const BRIDGE_PIPE_NAME: &str = r"\\.\pipe\micontrol_bridge";
/// Windows service name registered at install time.
pub const SERVICE_NAME: &str = "MiControlBridge";
/// Service display name.
pub const SERVICE_DISPLAY: &str = "MiControl Elevated Bridge";
/// Pipe buffer size.
const PIPE_BUF_SIZE: u32 = 16384;
/// Max command payload size (JSON).
const MAX_MSG_BYTES: usize = 16384;

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("service");

    match mode {
        "service" => {
            if let Err(e) = service::run() {
                eprintln!("[micontrol_bridge] Service error: {e}");
                std::process::exit(1);
            }
        }
        "console" => {
            // Testing mode: run the pipe server in the foreground.
            eprintln!("[micontrol_bridge] Console mode — pipe server on {BRIDGE_PIPE_NAME}");
            let shutdown = Arc::new(AtomicBool::new(false));
            pipe_server::run(shutdown);
            eprintln!("[micontrol_bridge] Console mode exiting");
        }
        "install" => match install_service() {
            Ok(()) => println!("MiControlBridge service installed and started."),
            Err(e) => {
                eprintln!("Failed to install MiControlBridge: {e}");
                std::process::exit(1);
            }
        },
        "uninstall" => match uninstall_service() {
            Ok(()) => println!("MiControlBridge service removed."),
            Err(e) => {
                eprintln!("Failed to remove MiControlBridge: {e}");
                std::process::exit(1);
            }
        },
        "self-test" => {
            // Validation mode (no admin required): create a test pipe with the
            // exact DACL used by the server, then connect as the current user.
            // Proves the Everyone RW DACL works end-to-end WITHOUT reinstalling
            // the service (which needs elevation).
            let code = pipe_server::self_test();
            std::process::exit(code);
        }
        "install-update" => {
            // S45-001: "Ponte Elevada" self-update trigger.
            //
            // Front-of-house entry point with NO elevation: signs an
            // `install_update` pipe command with the same HMAC key the app
            // uses, sends it to the MiControlBridge service (SYSTEM) over the
            // named pipe, and prints the bridge's reply. The bridge launches
            // the NSIS installer (silent /S /UPDATE /R) which kills the old
            // app + old bridge, installs the new files, re-creates the
            // MiControlBridge service and relaunches the app for the
            // interactive user — all without any UAC prompt.
            //
            // Usage:
            //   micontrol_bridge install-update <C:\path\to\MiControl_setup.exe>
            let installer_path = args.get(2).cloned();
            match relay_install_update(installer_path) {
                Ok(printed) => {
                    println!("{printed}");
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("install-update failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        "auto-update" => {
            // S45-002: dev-driven auto-update trigger (hidden option).
            //
            // Fetches the beta feed URL from HKCU (set via the app UI, the
            // `set_auto_update_beta_feed` Tauri command, or this CLI's
            // `--url` argument), downloads the installer to %TEMP%\MiControl\
            // and relays `install_update` to the SYSTEM service — all in one
            // step. Intended for development / MCP: point this CLI at a
            // freshly built setup.exe URL and MiControl updates itself.
            //
            // Usage:
            //   micontrol_bridge auto-update [<url>]
            //     <url> optional — overrides the persisted beta feed URL.
            //   micontrol_bridge auto-update --url <url>
            let url_arg = args.get(2).map(|s| s.as_str());
            let url = if let Some(a) = url_arg {
                if a == "--url" {
                    args.get(3).cloned()
                } else {
                    Some(a.to_string())
                }
            } else {
                micontrol_lib::util::auto_update::beta_feed_url()
            };
            match relay_auto_update(url) {
                Ok(printed) => {
                    println!("{printed}");
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("auto-update failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        other => {
            eprintln!("[micontrol_bridge] Unknown mode: {other}");
            eprintln!(
                "Usage: micontrol_bridge <service|console|install|uninstall|self-test|install-update <setup.exe>|auto-update [<url>]>"
            );
            std::process::exit(1);
        }
    }
}

// ── Service install helpers ──────────────────────────────────────────────────

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Find the current executable path (this binary).
fn current_exe() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "micontrol_bridge.exe".to_string())
}

/// Stop + delete a possibly-running service, then attempt to wait for the SCM
/// entry to actually disappear. `sc delete` is ASYNC: the entry lingers
/// (marked for deletion, 1072) until every open service handle closes — which
/// may take much longer than 15 s on an upgrade (the old bridge process /
/// installer service-control race). This is BEST-EFFORT: it never aborts the
/// install. The long `sc create` retry loop below is the actual wait mechanism
/// (1072 is transient and retried), so failing to observe deletion here must
/// not be fatal — aborting guarantees a broken install, retrying usually wins.
fn remove_service_wait(name: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    // S58 FIX: disable SCM crash auto-restart BEFORE stopping. The service is
    // installed with `restart/5000` — on a degraded SCM (1061 busy, 1072
    // delete-pending) the stop request may be delayed or ignored, and the SCM
    // revives the process between our stop and the binary overwrite, re-locking
    // the exe ("Error opening file for writing"). A stopped service must STAY
    // stopped until the install finishes; the fresh failure actions are
    // re-applied by install_service() after `sc create`.
    let _ = Command::new("sc")
        .args(["failure", name, "reset=", "0", "actions=", ""])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let _ = Command::new("sc")
        .args(["stop", name])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    // Brief pause so the stop reaches the SCM before the delete.
    std::thread::sleep(std::time::Duration::from_millis(700));
    let _ = Command::new("sc")
        .args(["delete", name])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    // Poll until the entry is gone (exit != 0 → 1060 does not exist) or ~30 s.
    // NOTE: while marked-for-delete the entry STILL queries successfully
    // (exit 0), so we keep polling. Best effort — see the doc comment above.
    for _ in 0..60 {
        let q = Command::new("sc")
            .args(["query", name])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("sc query failed to spawn: {e}"))?;
        if !q.status.success() {
            let text = String::from_utf8_lossy(&q.stdout).to_lowercase();
            if text.contains("does not exist") || text.contains("1060") {
                return Ok(());
            }
            // Some non-zero exit with an unexpected body — treat as gone enough
            // to proceed (sc query failed = SCM has no usable entry).
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    // Never fatal: the retry loop in install_service() absorbs the remaining
    // time a marked-for-delete entry needs to vanish (1072 is retried).
    eprintln!(
        "[install_service] service {name} still visible after 30s (delete pending) — \
         continuing; sc create will retry 1072"
    );
    Ok(())
}

/// Normalize a sc.exe command's combined output for parsing (sc writes its
/// error text to stdout; join with any stderr to be safe).
fn sc_text(out: &std::process::Output) -> String {
    format!(
        "{} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Install the MiControlBridge Windows service (requires admin).
fn install_service() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    let exe = current_exe();
    let bin_path = format!("\"{exe}\" service");

    // Stop + delete any existing service first (best-effort; see doc comment
    // above — a 1072-marked-for-delete entry must NOT abort the install, the
    // retry loop below absorbs it).
    if let Err(e) = remove_service_wait(SERVICE_NAME) {
        eprintln!("[install_service] {e}");
    }

    // Create the service with a LONG retry window. `sc delete` is async: the
    // old entry can stay marked-for-delete (1072) well past a fixed sleep,
    // especially when service-control handles linger. 1072/1073 are
    // transient upgrade states — keep retrying; only fail on a definite error
    // (e.g. access denied 5, invalid param 87).
    const MAX_CREATE_ATTEMPTS: usize = 30; // ~30 s window
    const RETRY_DELAY_MS: u64 = 1000;
    let mut out = Command::new("sc")
        .args([
            "create",
            SERVICE_NAME,
            "binPath=",
            &bin_path,
            "start=",
            "auto",
            "DisplayName=",
            SERVICE_DISPLAY,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("sc create failed to spawn: {e}"))?;

    let mut attempt: usize = 0;
    loop {
        if out.status.success() {
            break;
        }
        let text = sc_text(&out);
        let lower = text.to_lowercase();
        // 1072 (marked-for-delete) and 1073 (already exists) are the ONLY
        // transient upgrade states — the async delete is still settling.
        // Any other code (5 access denied, 87 invalid param, ...) is a definite
        // failure that retrying will never fix: fail fast with a clear error.
        let transient = lower.contains("1072") || lower.contains("1073");
        attempt += 1;
        if transient && attempt < MAX_CREATE_ATTEMPTS {
            eprintln!(
                "[install_service] sc create attempt {attempt} retrying \
                 (delete of old entry still settling): {}",
                text.trim()
            );
            std::thread::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS));
        } else {
            return Err(format!("sc create failed: {text}"));
        }
        out = Command::new("sc")
            .args([
                "create",
                SERVICE_NAME,
                "binPath=",
                &bin_path,
                "start=",
                "auto",
                "DisplayName=",
                SERVICE_DISPLAY,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("sc create failed to spawn: {e}"))?;
    }

    // Run as LocalSystem with failure auto-restart.
    let _ = Command::new("sc")
        .args(["config", SERVICE_NAME, "obj=", "LocalSystem"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let _ = Command::new("sc")
        .args([
            "failure",
            SERVICE_NAME,
            "reset=",
            "86400",
            "actions=",
            "restart/5000/restart/10000/restart/30000",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    // No desktop interaction needed.
    let _ = Command::new("sc")
        .args(["config", SERVICE_NAME, "type=", "own", "start=", "auto"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    // Start the service.
    let start = Command::new("sc")
        .args(["start", SERVICE_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("sc start failed to spawn: {e}"))?;
    if !start.status.success() {
        // Might already be running — check.
        let q = Command::new("sc")
            .args(["query", SERVICE_NAME])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("sc query failed: {e}"))?;
        let qtext = String::from_utf8_lossy(&q.stdout);
        if !qtext.contains("RUNNING") {
            return Err(format!(
                "sc start failed: {}",
                String::from_utf8_lossy(&start.stderr)
            ));
        }
    }

    Ok(())
}

/// Uninstall the MiControlBridge Windows service (requires admin).
fn uninstall_service() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    let _ = Command::new("sc")
        .args(["stop", SERVICE_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    std::thread::sleep(std::time::Duration::from_millis(700));
    let out = Command::new("sc")
        .args(["delete", SERVICE_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("sc delete failed to spawn: {e}"))?;
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stderr);
        // "service does not exist" is fine.
        if !text.to_lowercase().contains("does not exist") {
            return Err(format!("sc delete failed: {text}"));
        }
    }
    // Wait for the entry to actually disappear (handles may linger).
    let _ = remove_service_wait(SERVICE_NAME);
    Ok(())
}

// ── Pipe relay (unprivileged "Ponte Elevada" front) ──────────────────────────

/// S45-001: Relay an `install_update` command to the MiControlBridge service
/// (SYSTEM) over the named pipe, using the same HMAC-keyed protocol the app
/// uses. Called from the CLI mode `install-update <setup.exe>`.
///
/// This is the "Ponte Elevada" the user asked for: running
///   `micontrol_bridge install-update path\to\MiControl_setup.exe`
/// (or the app's install-invocation) talks to the ALREADY-RUNNING SYSTEM
/// service — no admin, no UAC — and the service launches the silent installer
/// which replaces the app + bridge, re-creates the service, and relaunches the
/// app for the logged-on user.
///
/// Returns the bridge's JSON reply (pretty-printed) on success.
fn relay_install_update(installer_path: Option<String>) -> Result<String, String> {
    // leak-check: audited — every exit path closes the handle explicitly (S67).
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{
        CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OVERLAPPED,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult};

    let installer_path = installer_path.ok_or_else(|| {
        "install-update requires an installer path: micontrol_bridge install-update <setup.exe>"
            .to_string()
    })?;

    // ── Build the signed request payload (mirrors elev_bridge::run_via_service_pipe) ──
    let request_id = format!("cli-{}", now_unix_ms());
    let nonce = micontrol_lib::util::auth::generate_nonce();
    let mut payload = serde_json::json!({
        "protocol_version": 3,
        "request_id": request_id,
        "created_at_ms": micontrol_lib::util::auth::now_ms(),
        "nonce": nonce,
        "caller_pid": std::process::id(),
        "cmd": "install_update",
        "args": { "installer": installer_path },
    });
    let key = micontrol_lib::util::auth::read_key()
        .map_err(|e| format!("Cannot read bridge auth key: {e}"))?;
    micontrol_lib::util::auth::sign_payload(&mut payload, &key);

    // ── Open the pipe (same DACL semantics as the app's pipe_request) ───────
    let pipe_path_w: Vec<u16> = OsStr::new(BRIDGE_PIPE_NAME)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            PCWSTR(pipe_path_w.as_ptr()),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
            HANDLE::default(),
        )
        .map_err(|e| format!("Open bridge pipe: {e}"))?
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err("INVALID_HANDLE_VALUE opening bridge pipe".to_string());
    }

    let mut written = 0u32;
    let body = payload.to_string();
    // Overlapped write (bounded wait so a wedged bridge can't hang us).
    let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
        .map_err(|e| format!("CreateEventW: {e}"))?;
    let mut write_ov: windows::Win32::System::IO::OVERLAPPED = unsafe { std::mem::zeroed() };
    write_ov.hEvent = event;
    let mut write_pending = false;
    let mut write_result: Result<(), windows::core::Error> = Ok(());
    unsafe {
        let op = WriteFile(
            handle,
            Some(body.as_bytes()),
            Some(&mut written),
            Some(&mut write_ov),
        );
        if op.is_err() {
            let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if code == 997 {
                write_pending = true; // ERROR_IO_PENDING
            } else {
                write_result = Err(windows::core::Error::from_win32());
            }
        }
        if write_pending {
            if WaitForSingleObject(event, 8_000) == WAIT_TIMEOUT {
                let _ = CancelIoEx(handle, Some(&write_ov));
                write_result = Err(windows::core::Error::from_win32());
            } else {
                let mut transferred = 0u32;
                if GetOverlappedResult(handle, &write_ov, &mut transferred, false).is_err() {
                    write_result = Err(windows::core::Error::from_win32());
                }
            }
        }
    }
    write_result.map_err(|e| format!("WriteFile to bridge pipe: {e}"))?;

    // ── Read the reply ───────────────────────────────────────────────────────
    let mut buf = [0u8; MAX_MSG_BYTES];
    let mut read_ov: windows::Win32::System::IO::OVERLAPPED = unsafe { std::mem::zeroed() };
    read_ov.hEvent = event;
    let mut read_result = String::new();
    loop {
        let mut bytes_read = 0u32;
        let op = unsafe {
            ReadFile(
                handle,
                Some(&mut buf),
                Some(&mut bytes_read),
                Some(&mut read_ov),
            )
        };
        if op.is_err() {
            let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if code != 997 {
                break; // real error → stop reading
            }
            // ERROR_IO_PENDING → wait bounded
            let wait = unsafe { WaitForSingleObject(event, 8_000) };
            if wait == WAIT_TIMEOUT {
                let _ = unsafe { CancelIoEx(handle, Some(&read_ov)) };
                break;
            }
            let mut transferred = 0u32;
            let ok = unsafe { GetOverlappedResult(handle, &read_ov, &mut transferred, false) };
            if ok.is_err() {
                break;
            }
            bytes_read = transferred;
        }
        if bytes_read == 0 {
            break;
        }
        read_result.push_str(&String::from_utf8_lossy(&buf[..bytes_read as usize]));
        if read_result.trim_end().ends_with('}') {
            break;
        }
    }

    unsafe {
        CloseHandle(event).ok();
        CloseHandle(handle).ok();
    }

    if read_result.is_empty() {
        return Err("Bridge returned no response".to_string());
    }

    // Pretty-print the bridge's JSON reply.
    let v: serde_json::Value = serde_json::from_str(&read_result)
        .map_err(|e| format!("Invalid bridge reply JSON: {e}"))?;
    Ok(serde_json::to_string_pretty(&v).unwrap_or(read_result))
}

/// Milliseconds since Unix epoch (small local helper — avoids importing chrono here).
fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// S45-002: dev-driven auto-update — download a beta installer from `url`
/// (or the persisted beta feed) into `%TEMP%\MiControl\updates\`, then relay
/// `install_update` to the SYSTEM bridge exactly like `auto-update`.
///
/// This is the CLI/MCP hook for "instalar a versão beta que queremos e o
/// instalador faz isso sozinho": point this at a fresh setup.exe URL and the
/// whole update runs silently via the bridge.
fn relay_auto_update(url: Option<String>) -> Result<String, String> {
    let url = url.ok_or_else(|| {
        "auto-update requires a URL: micontrol_bridge auto-update <installer_url> (or set the beta feed)".to_string()
    })?;
    let url = url.trim();
    if url.is_empty() {
        return Err("auto-update: empty installer URL".to_string());
    }

    // ── Download the installer (blocking via a local tokio runtime) ─────────
    let temp_dir = std::env::temp_dir().join("MiControl").join("updates");
    std::fs::create_dir_all(&temp_dir).map_err(|e| format!("Cannot create update dir: {e}"))?;

    let filename = url
        .rsplit('/')
        .next()
        .filter(|f| !f.is_empty())
        .ok_or_else(|| "auto-update: cannot derive installer filename from URL".to_string())?;

    if !filename.to_lowercase().ends_with(".exe")
        && !filename.to_lowercase().ends_with("_payload.zip")
    {
        return Err(format!(
            "auto-update: downloaded file is not a MiControl installer (.exe) or payload (_payload.zip): {filename}"
        ));
    }

    let dest = temp_dir.join(filename);
    eprintln!("[auto-update] Downloading {url} -> {}", dest.display());

    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("tokio runtime: {e}"))?;
    let result: Result<(), String> = rt.block_on(async {
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) MiControl/0.2")
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .map_err(|e| format!("HTTP client: {e}"))?;
        let bytes = client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("Download failed: {e}"))?
            .error_for_status()
            .map_err(|e| format!("HTTP error: {e}"))?
            .bytes()
            .await
            .map_err(|e| format!("Read bytes: {e}"))?;
        std::fs::write(&dest, &bytes).map_err(|e| format!("Cannot write update: {e}"))?;
        log::info!(
            "[auto-update] Downloaded {} ({} bytes)",
            dest.display(),
            bytes.len()
        );
        Ok(())
    });
    result?;

    eprintln!("[auto-update] Downloaded — relaying install_update to the bridge...");
    relay_install_update(Some(dest.to_string_lossy().to_string()))
}

// ── Windows Service plumbing ─────────────────────────────────────────────────

mod service {
    use super::*;
    use std::ffi::OsString;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::System::Services::{
        RegisterServiceCtrlHandlerW, SetServiceStatus, StartServiceCtrlDispatcherW,
        LPHANDLER_FUNCTION, SERVICE_ACCEPT_STOP, SERVICE_CONTROL_STOP, SERVICE_RUNNING,
        SERVICE_START_PENDING, SERVICE_STATUS, SERVICE_STOPPED, SERVICE_TABLE_ENTRYW,
        SERVICE_WIN32_OWN_PROCESS,
    };

    static SHUTDOWN: AtomicBool = AtomicBool::new(false);

    pub fn run() -> Result<(), String> {
        let mut name_w: Vec<u16> = OsString::from(SERVICE_NAME)
            .encode_wide()
            .chain(Some(0))
            .collect();

        let table = [
            SERVICE_TABLE_ENTRYW {
                lpServiceName: PWSTR(name_w.as_mut_ptr()),
                lpServiceProc: Some(service_main),
            },
            SERVICE_TABLE_ENTRYW {
                lpServiceName: PWSTR::null(),
                lpServiceProc: None,
            },
        ];

        unsafe {
            StartServiceCtrlDispatcherW(table.as_ptr())
                .map_err(|e| format!("StartServiceCtrlDispatcherW: {e}"))?;
        }
        Ok(())
    }

    unsafe extern "system" fn service_main(_argc: u32, _argv: *mut windows::core::PWSTR) {
        let name_w: Vec<u16> = OsString::from(SERVICE_NAME)
            .encode_wide()
            .chain(Some(0))
            .collect();

        let handler: LPHANDLER_FUNCTION = Some(service_control_handler);
        let status_handle = RegisterServiceCtrlHandlerW(PCWSTR(name_w.as_ptr()), handler);

        match status_handle {
            Ok(h) => {
                set_service_state(h, SERVICE_START_PENDING);

                let shutdown = Arc::new(AtomicBool::new(false));
                let shutdown_clone = shutdown.clone();

                let pipe_thread = std::thread::spawn(move || {
                    pipe_server::run(shutdown_clone);
                });

                // S55 FIX 19 disabled the watchdog unconditionally after an
                // access violation in the Win32 launch path (~40 s after every
                // start), which left `ui_ok` with NO consumer: the 06-Nov-2026
                // starvation ran 2 h 46 min with zero restarts. Arming now
                // follows the user's autostart mirror (`watchdog_enabled`,
                // written by the app since 2026-11-06 22:19) or the
                // `MCONTROL_WATCHDOG` env override; `MCONTROL_WATCHDOG=0`
                // forces it off without touching the marker. The poll body is
                // wrapped in catch_unwind and only touches the Win32 launch
                // path when the app is actually unhealthy, so a healthy system
                // never executes it.
                let watchdog_enabled = watchdog::start_enabled();
                let watchdog_thread = if watchdog_enabled {
                    let wd_shutdown = shutdown.clone();
                    Some(std::thread::spawn(move || {
                        watchdog::run(wd_shutdown, 30_000);
                    }))
                } else {
                    eprintln!(
                        "[micontrol_bridge] watchdog disabled (no watchdog_enabled marker, \
                         MCONTROL_WATCHDOG not set to 1)"
                    );
                    None
                };

                set_service_state(h, SERVICE_RUNNING);
                eprintln!("[micontrol_bridge] Service running — pipe: {BRIDGE_PIPE_NAME}");

                while !SHUTDOWN.load(Ordering::SeqCst) {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }

                shutdown.store(true, Ordering::SeqCst);
                let _ = pipe_thread.join();
                if let Some(wd) = watchdog_thread {
                    let _ = wd.join();
                }
                set_service_state(h, SERVICE_STOPPED);
                eprintln!("[micontrol_bridge] Service stopped");
            }
            Err(e) => {
                eprintln!("[micontrol_bridge] RegisterServiceCtrlHandlerW failed: {e}");
            }
        }
    }

    extern "system" fn service_control_handler(control: u32) {
        if control == SERVICE_CONTROL_STOP {
            SHUTDOWN.store(true, Ordering::SeqCst);
        }
    }

    fn set_service_state(
        handle: windows::Win32::System::Services::SERVICE_STATUS_HANDLE,
        state: windows::Win32::System::Services::SERVICE_STATUS_CURRENT_STATE,
    ) {
        unsafe {
            let accept = if state == SERVICE_RUNNING {
                SERVICE_ACCEPT_STOP
            } else {
                0u32
            };
            let status = SERVICE_STATUS {
                dwServiceType: SERVICE_WIN32_OWN_PROCESS,
                dwCurrentState: state,
                dwControlsAccepted: accept,
                dwWin32ExitCode: 0,
                dwServiceSpecificExitCode: 0,
                dwCheckPoint: 0,
                dwWaitHint: 3000,
            };
            SetServiceStatus(handle, &status).ok();
        }
    }
}

// ── Pipe server ──────────────────────────────────────────────────────────────

mod pipe_server {
    use super::*;
    use std::ffi::OsStr;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{
        CloseHandle, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
        WAIT_OBJECT_0,
    };
    use windows::Win32::Storage::FileSystem::{
        FlushFileBuffers, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
        PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};
    use windows::Win32::System::IO::{CancelIoEx, OVERLAPPED};

    /// Build a SECURITY_ATTRIBUTES with a DACL that grants Everyone read/write
    /// access to the named pipe.
    ///
    /// When the service runs as `NT AUTHORITY\SYSTEM`, pipes created without
    /// explicit security inherit a DACL that only SYSTEM can access — the
    /// unprivileged MiControl app would then fail to open
    /// `\\.\pipe\micontrol_bridge` (ERROR_ACCESS_DENIED, error 5) and every
    /// elevated command would fall back to the scheduled task / UAC prompt at
    /// startup. Granting Everyone access is safe because the bridge protocol
    /// is HMAC-SHA256-authenticated (shared key in `%LocalAppData%\MiControl`
    /// with a freshness window), so an arbitrary process cannot forge commands.
    ///
    /// Implementation: build the DACL from the canonical SDDL string
    /// `D:(A;;GA;;;WD)` (Everyone/World → Generic All) via
    /// `ConvertStringSecurityDescriptorToSecurityDescriptorW`. This is the
    /// deterministic, language-independent way to grant Everyone access and it
    /// works identically whether the process runs as the interactive user or
    /// as SYSTEM. Previous iterations used `BuildExplicitAccessWithNameW`
    /// (silently failed on non-English/restricted Windows) and then
    /// `CreateWellKnownSid` + `SetEntriesInAclW` — which can return errors
    /// under SYSTEM and leave the pipe SYSTEM-only; SDDL parsing avoids all of
    /// that. The returned SECURITY_DESCRIPTOR is heap-allocated by Windows and
    /// intentionally leaked (must outlive the SECURITY_ATTRIBUTES usages).
    fn build_pipe_security_attributes() -> Option<windows::Win32::Security::SECURITY_ATTRIBUTES> {
        use windows::core::PCWSTR;
        use windows::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };
        use windows::Win32::Security::SECURITY_ATTRIBUTES;

        unsafe {
            use windows::Win32::Security::PSECURITY_DESCRIPTOR;
            let sddl: Vec<u16> = "D:(A;;GA;;;WD)"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let mut psd: PSECURITY_DESCRIPTOR = PSECURITY_DESCRIPTOR(std::ptr::null_mut());

            let result = ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut psd,
                None,
            );
            let psd_ptr = psd.0;
            if result.is_err() || psd_ptr.is_null() {
                let last_err = std::io::Error::last_os_error();
                eprintln!(
                    "[micontrol_bridge] ConvertStringSecurityDescriptorToSecurityDescriptorW failed: {last_err} — pipe will be SYSTEM-only"
                );
                return None;
            }

            // psd is a heap allocation owned by the caller now; intentionally
            // never freed (lives for the whole pipe-server process).
            Some(SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: psd_ptr,
                bInheritHandle: false.into(),
            })
        }
    }

    // ── Accept-step primitives ──────────────────────────────────────────────

    /// `HANDLE` wraps a raw pointer, so it is `!Send`. These newtypes carry a
    /// handle across a thread boundary where ownership is 1:1 and unambiguous.
    ///
    /// SAFETY (both impls): a handle is an opaque kernel identifier, not memory.
    /// The value is owned by exactly one thread at a time — the accept loop
    /// stops touching it after the handoff and an unbounded channel has a single
    /// consumer per item — so no two threads ever use the same handle
    /// concurrently.
    struct SendHandle(HANDLE);
    unsafe impl Send for SendHandle {}
    struct SendEvent(HANDLE);
    unsafe impl Send for SendEvent {}

    impl SendHandle {
        /// Raw handle, consuming the newtype. Taking `self` is deliberate:
        /// a `move` closure that only reads `.0` would capture the `!Send`
        /// `HANDLE` field alone (edition 2021 disjoint captures), so callers go
        /// through this method and capture the whole newtype instead.
        fn into_handle(self) -> HANDLE {
            self.0
        }
    }

    impl SendEvent {
        /// See [`SendHandle::into_handle`].
        fn into_event(self) -> HANDLE {
            self.0
        }
    }

    /// Outcome of one accept step ([`accept_client`]).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum AcceptOutcome {
        /// A client connected while `ConnectNamedPipe` was pending: the
        /// overlapped event fired. The normal path.
        Connected,
        /// A client had ALREADY connected when `ConnectNamedPipe` ran, so the
        /// call failed with `ERROR_PIPE_CONNECTED` (535) *without queueing any
        /// overlapped I/O*. The event is never signalled — but the pipe IS
        /// connected and the client must be served.
        AlreadyConnected,
        /// `ConnectNamedPipe` (or the wait) failed hard; the instance is
        /// unusable and must be destroyed. Carries the Win32 error code.
        Failed(u32),
    }

    /// One accept step: issue `ConnectNamedPipe` on `handle` and block until a
    /// client is connected.
    ///
    /// `handle` must be a pipe instance created with `FILE_FLAG_OVERLAPPED`;
    /// `event` must be the manual-reset event stored in the `OVERLAPPED` passed
    /// to `ConnectNamedPipe`.
    ///
    /// C-01 (deadlock, production 2026-11-02 → 2026-11-07): the previous code
    /// discarded the `ConnectNamedPipe` result (`let _ = …`) and always waited
    /// `INFINITE` on the event. For an overlapped pipe, a client that connects
    /// BEFORE the call makes `ConnectNamedPipe` return FALSE with
    /// `ERROR_PIPE_CONNECTED` **without queueing I/O**, so the event can never
    /// be signalled and the accept loop parked forever: the service stayed
    /// RUNNING, the single listening instance stayed connected to a client
    /// nobody served, and every later open failed with `ERROR_PIPE_BUSY`
    /// (0x800700E7) — the app then fell back to one elevated helper process per
    /// hardware read. `bridge.log` stops right after such a client
    /// ("client connected" / "read 0 bytes").
    ///
    /// `ERROR_PIPE_CONNECTED` means "there is already a client on the other
    /// end": the correct response is to proceed to the handoff, never to wait.
    fn accept_client(handle: HANDLE, event: HANDLE) -> AcceptOutcome {
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        overlapped.hEvent = event;

        let connect = unsafe { ConnectNamedPipe(handle, Some(&mut overlapped)) };
        // Grab GetLastError before anything else on this thread can clobber it.
        // `windows` builds its Err from the same value, but reading it here is
        // what makes the ERROR_PIPE_CONNECTED branch explicit.
        let last_error = std::io::Error::last_os_error();

        if connect.is_ok() {
            // Connected synchronously — there is no pending I/O to wait for.
            return AcceptOutcome::Connected;
        }

        let code = last_error.raw_os_error().unwrap_or(0) as u32;
        if code == ERROR_PIPE_CONNECTED.0 {
            // The client raced in between CreateNamedPipeW and this call. No
            // I/O was queued, so waiting on the event would hang forever.
            return AcceptOutcome::AlreadyConnected;
        }
        if code != ERROR_IO_PENDING.0 {
            return AcceptOutcome::Failed(code);
        }

        // Normal overlapped path: park until a client actually connects.
        // INFINITE is deliberate (S55 FIX 21): a client connection is the only
        // wake-up, and timing the instance out orphaned connected clients.
        match unsafe { WaitForSingleObject(event, INFINITE) } {
            WAIT_OBJECT_0 => AcceptOutcome::Connected,
            _ => AcceptOutcome::Failed(
                std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32
            ),
        }
    }

    /// One listening instance of the bridge pipe plus the manual-reset event
    /// used for its overlapped accept.
    ///
    /// Owns both HANDLEs: `Drop` closes them, so every `continue` in the accept
    /// loop is leak-free (S67 RAII discipline — the loop used to close handles
    /// by hand at four different exits). [`PipeInstance::into_handles`] releases
    /// them to the worker that serves the client.
    struct PipeInstance {
        handle: HANDLE,
        event: HANDLE,
    }

    impl PipeInstance {
        /// `CreateNamedPipeW` + `CreateEventW` with the exact flags the service
        /// uses. Returns `None` (closing anything already created) on failure.
        fn create(
            name_w: &[u16],
            max_instances: u32,
            sec_attr: Option<&windows::Win32::Security::SECURITY_ATTRIBUTES>,
        ) -> Option<Self> {
            // SAFETY: `name_w` is NUL-terminated and outlives the call;
            // `sec_attr`, when present, outlives the call and points to a
            // descriptor the kernel copies into the pipe object (so it does not
            // have to outlive the instance).
            let handle = unsafe {
                CreateNamedPipeW(
                    PCWSTR(name_w.as_ptr()),
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                    max_instances,
                    PIPE_BUF_SIZE,
                    PIPE_BUF_SIZE,
                    0,
                    sec_attr.map(|s| s as *const windows::Win32::Security::SECURITY_ATTRIBUTES),
                )
            };
            if handle == INVALID_HANDLE_VALUE {
                eprintln!(
                    "[micontrol_bridge] CreateNamedPipeW failed: {}",
                    std::io::Error::last_os_error()
                );
                return None;
            }

            match unsafe { CreateEventW(None, true, false, PCWSTR::null()) } {
                Ok(event) => Some(Self { handle, event }),
                Err(_) => {
                    eprintln!(
                        "[micontrol_bridge] CreateEventW failed: {}",
                        std::io::Error::last_os_error()
                    );
                    // SAFETY: created above, not owned anywhere else yet.
                    unsafe {
                        CloseHandle(handle).ok();
                    }
                    None
                }
            }
        }

        /// Block until a client is connected on this instance (see
        /// [`accept_client`] for the `ERROR_PIPE_CONNECTED` race).
        fn accept(&self) -> AcceptOutcome {
            accept_client(self.handle, self.event)
        }

        /// Release both handles to the worker that will serve the client. The
        /// instance no longer closes them on drop.
        fn into_handles(mut self) -> (HANDLE, HANDLE) {
            let handles = (self.handle, self.event);
            self.handle = INVALID_HANDLE_VALUE;
            self.event = INVALID_HANDLE_VALUE;
            handles
        }
    }

    impl Drop for PipeInstance {
        fn drop(&mut self) {
            unsafe {
                if self.handle != INVALID_HANDLE_VALUE {
                    CloseHandle(self.handle).ok();
                }
                if self.event != INVALID_HANDLE_VALUE {
                    CloseHandle(self.event).ok();
                }
            }
        }
    }

    pub fn run(shutdown: Arc<AtomicBool>) {
        let pipe_name_w: Vec<u16> = OsStr::new(BRIDGE_PIPE_NAME)
            .encode_wide()
            .chain(Some(0))
            .collect();

        // S55 FIX 23b: fixed worker pool — see the long comment at the send
        // site below for why thread-per-client had to go (std Windows TLS
        // dtor AV on thread exit crashing the whole SYSTEM service).
        // 4 workers absorb realistic bursts (app serialized lock + pollers);
        // the unbounded channel queue absorbs spikes with zero thread churn.
        //
        // SAFETY: HANDLEs crossing the channel are owned 1:1 by whichever
        // worker pops the job; the accept loop never touches them again and
        // no other thread can see them (unbounded channel, single consumer
        // per item). The module-level SendHandle/SendEvent newtypes assert
        // `Send` only — see their definition above.

        const PIPE_WORKERS: usize = 4;
        let (worker_tx, worker_rx) = std::sync::mpsc::channel::<(SendHandle, SendEvent)>();
        let worker_rx = Arc::new(std::sync::Mutex::new(worker_rx));
        for worker_id in 0..PIPE_WORKERS {
            let rx = Arc::clone(&worker_rx);
            let spawned = std::thread::Builder::new()
                .name(format!("pipe-worker-{worker_id}"))
                .spawn(move || loop {
                    // Park until the accept loop hands us a connected client.
                    // The lock is held only for the pop — handle_client runs
                    // OUTSIDE the lock so workers serve in parallel.
                    let job = {
                        let guard = rx.lock().expect("worker rx mutex poisoned");
                        guard.recv()
                    };
                    let Ok((h, ev)) = job else {
                        // Channel closed (sender dropped) — only happens at
                        // process teardown. Exit is fine here: the process is
                        // dying anyway.
                        break;
                    };
                    let (handle, event) = (h.0, ev.0);
                    handle_client(handle);
                    unsafe {
                        // S55 FIX 24c: the pipe handle was created with
                        // FILE_FLAG_OVERLAPPED (needed for the async accept).
                        // handle_client's sync WriteFile(…, None) on an async
                        // handle leaves the response in flight — disconnecting
                        // immediately can discard it, and the client sees
                        // EOF/"No response". FlushFileBuffers blocks until
                        // all buffered data is delivered to the client.
                        FlushFileBuffers(handle).ok();
                        DisconnectNamedPipe(handle).ok();
                        CloseHandle(handle).ok();
                        CloseHandle(event).ok();
                    }
                });
            if let Err(e) = spawned {
                eprintln!("[micontrol_bridge] failed to spawn pipe-worker-{worker_id}: {e}");
            }
        }
        let worker_tx = Arc::new(worker_tx);

        // S32-005c FIX: this server used to serve ONE connection at a time in
        // this loop (create instance → connect → handle_client inline →
        // recreate). While a client was being served (even briefly, but
        // especially when its request took time — HMAC verify, EC access,
        // PowerShell helpers), the second instance did not exist, so a second
        // concurrent client got ERROR_PIPE_BUSY (0x800700E7). The app's
        // elevated calls serialize on one lock, but background pollers
        // (thermal/battery via other processes/threads) and the app's own
        // health checks raced, producing the storm of
        // "All pipe instances are busy" fallbacks to schtasks in the app log.
        // FIX: the accept loop creates a fresh pipe instance, hands the
        // connected client to the fixed worker pool, and immediately recreates
        // the listener — effectively unlimited concurrent clients.
        //
        // C-01: `last_accept_error` throttles accept-failure logging to one line
        // per distinct error code, so a stuck listener is visible in bridge.log
        // without flooding a 3 × 8 MiB rotation.
        let mut last_accept_error: Option<u32> = None;
        while !shutdown.load(Ordering::SeqCst) {
            // The SECURITY_ATTRIBUTES only has to outlive each CreateNamedPipeW
            // call — the kernel copies the descriptor into the pipe object — so
            // it is built ONCE here. Building it inside the loop (as before)
            // leaked one heap SECURITY_DESCRIPTOR per accepted connection:
            // ConvertStringSecurityDescriptorToSecurityDescriptorW allocates
            // and nothing ever freed it.
            let sec_attr = build_pipe_security_attributes();

            // The accept step (create instance → connect → wait) lives in
            // `PipeInstance` so it can be tested without a service: see
            // `accept_client` for the C-01 ERROR_PIPE_CONNECTED fix.
            let Some(instance) =
                PipeInstance::create(&pipe_name_w, PIPE_UNLIMITED_INSTANCES, sec_attr.as_ref())
            else {
                // CreateNamedPipeW / CreateEventW failed. Back off instead of
                // spinning: a persistent failure (handle exhaustion, a stuck
                // name) would otherwise burn 100% CPU.
                std::thread::sleep(std::time::Duration::from_secs(1));
                continue;
            };

            match instance.accept() {
                // C-01: BOTH success shapes hand the instance off. The
                // pre-connected client (ERROR_PIPE_CONNECTED) never signals the
                // event, so treating it as "keep waiting" is what parked this
                // loop for five days in production.
                AcceptOutcome::Connected | AcceptOutcome::AlreadyConnected => {
                    if last_accept_error.take().is_some() {
                        bridge_log("accept step recovered — client connected");
                    }
                }
                AcceptOutcome::Failed(code) => {
                    // Log the FIRST failure of a given code only: a persistent
                    // failure must be visible in bridge.log without flooding a
                    // file that keeps only 3 × 8 MiB.
                    if last_accept_error != Some(code) {
                        bridge_log(&format!(
                            "accept step failed: win32 error {code} — destroying the instance and listening again"
                        ));
                        last_accept_error = Some(code);
                    }
                    // Bounded backoff: never turn a hard failure into a
                    // create/destroy storm. `instance` is closed by Drop.
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
            }

            let (handle, event) = instance.into_handles();

            // S55 FIX 21 (history): this loop used to wait only 500 ms for a
            // client and then CANCELLED + destroyed the instance. A client that
            // connected inside that window was orphaned: its CreateFileW had
            // succeeded but the instance (and its pending ConnectNamedPipe I/O)
            // was destroyed before handle_client could run, so the app saw
            // "No response from bridge service (timed out or empty)" and fell
            // back to the (broken) scheduled-task path. The wait is therefore
            // INFINITE — a client connection is the only wake-up. That wait now
            // lives in `accept_client`, which also handles the
            // ERROR_PIPE_CONNECTED case where no wait must happen at all.

            // S55 FIX 23b: serve on a worker from a FIXED pool instead of
            // spawning a short-lived thread per client.
            //
            // WHY: every OS thread that exits runs the Rust std Windows TLS
            // destructor path (tls_callback → destructors::list::run →
            // eager::destroy), which crashed this SYSTEM service with
            // 0xc0000005 under load (see dumps: AV at eager::destroy+0x18).
            // The bug exists in rustc 1.95 AND 1.98.1 (PR #148799 changed the
            // storage model but the callback path is still unsafe here). With
            // thread-per-client, the service created/destroyed dozens of
            // threads per minute — each exit a lottery ticket for the crash.
            //
            // FIX: N long-lived worker threads (created ONCE, never exit) pull
            // client handles from a channel. Zero thread churn → the buggy
            // exit path is never exercised. If all workers are busy, the
            // channel queue holds the client briefly (each request completes
            // in ms; the queue absorbs bursts).
            let job = (SendHandle(handle), SendEvent(event));
            if let Err(e) = worker_tx.send(job) {
                // All workers gone (should never happen — they never exit).
                // Serve inline as a last resort so the request is not lost.
                eprintln!("[micontrol_bridge] worker pool dead ({e}); serving inline");
                handle_client(handle);
                unsafe {
                    FlushFileBuffers(handle).ok();
                    DisconnectNamedPipe(handle).ok();
                    CloseHandle(handle).ok();
                    CloseHandle(event).ok();
                }
            }
        }
    }

    /// S55 FIX 25b: service-side request log (ProgramData\MiControl\logs\
    /// bridge.log). eprintln is invisible for a windowed service, and without
    /// this there was NO way to see whether a client request even reached the
    /// worker — the recurring "No response" loop was undiagnosable.
    ///
    /// S64: size-based rotation (8 MiB x 3) — this file reached 18 MB in one
    /// release cycle with no cap.
    fn bridge_log(msg: &str) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let dir = std::path::Path::new("C:\\ProgramData\\MiControl\\logs");
        let path = dir.join("bridge.log");
        if let Err(e) = micontrol_lib::util::log_rotation::append_line(
            &path,
            &format!("{ts} {msg}"),
            micontrol_lib::util::log_rotation::DEFAULT_MAX_BYTES,
            micontrol_lib::util::log_rotation::DEFAULT_MAX_FILES,
        ) {
            // Never let logging break the service; surface it on stderr only.
            eprintln!("[bridge_log] write failed: {e}");
        }
    }

    fn handle_client(handle: windows::Win32::Foundation::HANDLE) {
        let t0 = std::time::Instant::now();
        bridge_log("client connected");
        // S55 FIX 24d: the pipe instance was created with FILE_FLAG_OVERLAPPED
        // (required by the async accept). Doing SYNCHRONOUS ReadFile/WriteFile
        // (overlapped=None) on an async handle is invalid: the call starts the
        // I/O and returns ERROR_IO_PENDING, which the old loop treated as an
        // error and bailed out — the client then saw EOF / "No response from
        // bridge service (timed out or empty)" even though the service was
        // healthy. This is why the app kept falling back to the (missing)
        // scheduled task while raw probes (which happened to race differently)
        // succeeded.
        //
        // FIX: proper overlapped I/O with a dedicated event and bounded waits,
        // mirroring the client-side logic in `send_pipe_request`.
        use windows::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};

        const IO_TIMEOUT_MS: u32 = 15_000;

        let io_event_raw = match unsafe { CreateEventW(None, true, false, PCWSTR::null()) } {
            Ok(ev) => ev,
            Err(_) => return,
        };
        // S67: RAII — the event was closed manually at two of the three exits;
        // the `return` in the read loop (read 0 bytes) leaked it on every
        // client that disconnected without sending a request.
        let _io_event_guard = micontrol_lib::util::win_handle::OwnedEvent::new(io_event_raw);
        let io_event = io_event_raw;

        let do_io = |buf: &mut [u8], write: bool| -> Option<usize> {
            let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
            ov.hEvent = io_event;
            let mut transferred = 0u32;
            unsafe {
                let op = if write {
                    WriteFile(handle, Some(buf), Some(&mut transferred), Some(&mut ov))
                } else {
                    ReadFile(handle, Some(buf), Some(&mut transferred), Some(&mut ov))
                };
                if op.is_err() {
                    let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                    if code != 997 {
                        // ERROR_IO_PENDING (997) is expected; anything else is fatal.
                        return None;
                    }
                    match WaitForSingleObject(io_event, IO_TIMEOUT_MS) {
                        WAIT_OBJECT_0 => {}
                        _ => {
                            let _ = CancelIoEx(handle, Some(&ov));
                            return None;
                        }
                    }
                    let mut done = 0u32;
                    if GetOverlappedResult(handle, &ov, &mut done, false).is_err() {
                        return None;
                    }
                    return Some(done as usize);
                }
                // Completed synchronously.
                Some(transferred as usize)
            }
        };

        // ── Read the full request ────
        let mut read_buf = [0u8; MAX_MSG_BYTES];
        let mut total_read = 0usize;
        loop {
            if total_read >= read_buf.len() {
                break;
            }
            let Some(n) = do_io(&mut read_buf[total_read..], false) else {
                break;
            };
            if n == 0 {
                break; // client closed / EOF
            }
            total_read += n;
            // Stop when the JSON object appears complete (ends with }).
            if total_read > 0 && read_buf[total_read - 1] == b'}' {
                break;
            }
        }

        if total_read == 0 {
            bridge_log("read 0 bytes (client closed before sending)");
            return;
        }

        let request_str = String::from_utf8_lossy(&read_buf[..total_read]).to_string();
        let cmd_desc = serde_json::from_str::<serde_json::Value>(&request_str)
            .ok()
            .and_then(|v| v.get("cmd").and_then(|c| c.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| "<unparseable>".to_string());
        bridge_log(&format!("request received: cmd={cmd_desc}"));
        let response = process_request(&request_str);
        bridge_log(&format!(
            "dispatch done: cmd={cmd_desc} elapsed_ms={}",
            t0.elapsed().as_millis()
        ));

        // ── Write the response (overlapped, bounded) ────
        let mut resp_bytes = response.into_bytes();
        let wrote = do_io(&mut resp_bytes, true);
        bridge_log(&format!(
            "response written: cmd={cmd_desc} ok={} total_elapsed_ms={}",
            wrote.is_some(),
            t0.elapsed().as_millis()
        ));

        unsafe {
            // Push the response fully to the client before tearing the pipe
            // down (see FIX 24c comment at the worker cleanup sites).
            FlushFileBuffers(handle).ok();
        }
    }

    /// Authenticate + dispatch one bridge request.
    fn process_request(request_str: &str) -> String {
        // Parse the request to extract the request_id for the response.
        let request_id = serde_json::from_str::<serde_json::Value>(request_str)
            .ok()
            .and_then(|v| v.get("request_id").cloned())
            .unwrap_or(serde_json::Value::Null);

        let mut response = match dispatch_with_auth(request_str) {
            Ok(data) => serde_json::json!({ "ok": true, "data": data }),
            Err(e) => serde_json::json!({ "ok": false, "error": e }),
        };
        if !request_id.is_null() {
            response["request_id"] = request_id;
        }
        response["created_at_ms"] = serde_json::json!(micontrol_lib::util::auth::now_ms());

        // Sign the response so the caller can verify integrity (same protocol
        // as the scheduled-task bridge).
        if let Ok(key) = micontrol_lib::util::auth::read_key() {
            micontrol_lib::util::auth::sign_payload(&mut response, &key);
        }
        response.to_string()
    }

    /// Verify HMAC + freshness, then execute the command via the library
    /// elevated dispatcher. Returns the `data` value on success.
    fn dispatch_with_auth(request_str: &str) -> Result<serde_json::Value, String> {
        let mut payload: serde_json::Value =
            serde_json::from_str(request_str).map_err(|e| format!("Invalid JSON request: {e}"))?;

        let cmd: String = payload
            .get("cmd")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing cmd field".to_string())?
            .to_string();
        let args = payload
            .get("args")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        // ── Auth: HMAC + freshness ────────────────────────────────────────
        let key = micontrol_lib::util::auth::read_key()
            .map_err(|e| format!("Bridge auth key unavailable: {e}"))?;

        // The library's verify_payload expects a `created_at_ms` field and an
        // `hmac` field; it verifies HMAC and timestamp freshness.
        micontrol_lib::util::auth::verify_payload(&mut payload, &key)
            .map_err(|e| format!("Bridge request authentication failed: {e}"))?;

        // ── Dispatch ───────────────────────────────────────────────────────
        // S66-004: bound the dispatch wall-clock. One read_thermal_readings
        // dispatch blocked a worker for 652 s (2026-10-21 17:42) when the
        // ESIF/ACPI WMI provider hung — the client had timed out long before,
        // and new requests piled up behind the stuck worker. A bounded wait
        // returns an error response to the client instead of blocking the
        // worker indefinitely. Healthy dispatches are single-digit ms, so
        // 30 s is a generous ceiling that only trips on genuine hangs.
        //
        // S68 FIX (TLS crash): the first version of this fix spawned a
        // short-lived thread PER DISPATCH — and every thread exit runs the
        // buggy Rust std Windows TLS destructor path
        // (std::sys::thread_local::native::eager::destroy), which crashes
        // this SYSTEM service with 0xc0000005 (same class as S55 FIX 23b:
        // the fixed worker pool exists precisely to avoid thread churn).
        // The crash was observed live: the freshly installed bridge crashed
        // at startup 3x (offset 0x175668 = eager::destroy+0x18) and the
        // service never reached RUNNING.
        //
        // Correct design: a FIXED dispatch pool (2 long-lived threads that
        // NEVER exit) + a bounded channel receive. The pipe worker blocks on
        // recv_timeout instead of on the WMI call; a hung dispatch occupies
        // one dispatch thread (bounded by pool size) while the pipe worker
        // returns the deadline error to the client. Zero thread churn.
        const DISPATCH_DEADLINE_SECS: u64 = 30;
        const DISPATCH_POOL: usize = 2;
        static DISPATCH_TX: std::sync::OnceLock<
            std::sync::mpsc::Sender<(
                String,
                serde_json::Value,
                std::sync::mpsc::Sender<serde_json::Value>,
            )>,
        > = std::sync::OnceLock::new();
        let dispatch_tx = DISPATCH_TX.get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::channel::<(
                String,
                serde_json::Value,
                std::sync::mpsc::Sender<serde_json::Value>,
            )>();
            let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
            for id in 0..DISPATCH_POOL {
                let rx = std::sync::Arc::clone(&rx);
                // Long-lived dispatch threads — they NEVER exit, so the
                // buggy TLS destructor path is never exercised (S55 23b).
                let _ = std::thread::Builder::new()
                    .name(format!("bridge-dispatch-{id}"))
                    .spawn(move || loop {
                        let job = {
                            let guard = rx.lock().unwrap_or_else(|e| e.into_inner());
                            guard.recv()
                        };
                        let Ok((cmd, args, reply_tx)) = job else {
                            break; // process teardown
                        };
                        let r = micontrol_lib::elevated::dispatch_cmd(&cmd, args);
                        let _ = reply_tx.send(r);
                    });
            }
            tx
        });

        let (reply_tx, reply_rx) = std::sync::mpsc::channel::<serde_json::Value>();
        let _ = dispatch_tx.send((cmd.clone(), args.clone(), reply_tx));
        let result =
            match reply_rx.recv_timeout(std::time::Duration::from_secs(DISPATCH_DEADLINE_SECS)) {
                Ok(r) => r,
                Err(_) => {
                    // The dispatch thread is still running the hung call — it
                    // will finish (or not) on its own; its reply is dropped when
                    // the channel sender goes out of scope. The dispatch pool
                    // being fixed-size bounds how many hung calls can stack.
                    bridge_log(&format!(
                        "dispatch deadline exceeded ({DISPATCH_DEADLINE_SECS}s): cmd={cmd} — \
                     returning error to client, dispatch thread continues in background"
                    ));
                    serde_json::json!({
                        "ok": false,
                        "error": format!(
                            "Bridge dispatch for '{cmd}' exceeded the internal \
                             {DISPATCH_DEADLINE_SECS}s deadline (hung WMI provider?)"
                        ),
                    })
                }
            };
        if result["ok"].as_bool().unwrap_or(false) {
            Ok(result["data"].clone())
        } else {
            Err(result["error"]
                .as_str()
                .unwrap_or("Bridge dispatch failed")
                .to_string())
        }
    }

    /// Validate the Everyone-RW DACL WITHOUT admin: create a throwaway pipe
    /// using the exact same security attributes as the real server, connect as
    /// the current (unprivileged) user, and do a tiny read/write round-trip.
    /// Exit code 0 = DACL works; 1 = DACL broken (ACCESS_DENIED would hit the
    /// real app, exactly the v0.1.16 bug).
    pub fn self_test() -> i32 {
        // leak-check: audited — every exit path closes the handle explicitly (S67).
        use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, ReadFile, WriteFile, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };
        use windows::Win32::System::Pipes::DisconnectNamedPipe;
        use windows::Win32::System::Threading::CreateEventW;

        const TEST_PIPE: &str = r"\\.\pipe\micontrol_bridge_selftest";
        let pipe_name_w: Vec<u16> = OsStr::new(TEST_PIPE).encode_wide().chain(Some(0)).collect();

        // Spawn a minimal server in this process (same DACL path). Keep the
        // SECURITY_ATTRIBUTES alive explicitly (see run() for rationale).
        let sec_attr = build_pipe_security_attributes();
        let server_handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(pipe_name_w.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                PIPE_BUF_SIZE,
                PIPE_BUF_SIZE,
                0,
                sec_attr
                    .as_ref()
                    .map(|s| s as *const windows::Win32::Security::SECURITY_ATTRIBUTES),
            )
        };
        if server_handle == INVALID_HANDLE_VALUE {
            eprintln!(
                "[self-test] CreateNamedPipeW failed: {}",
                std::io::Error::last_os_error()
            );
            return 1;
        }

        // Connect the server side. NOTE THE ORDER: the CLIENT is opened first,
        // which is the case the service hits in production (a client that
        // connects between CreateNamedPipeW and ConnectNamedPipe). The accept
        // step therefore takes the ERROR_PIPE_CONNECTED path deterministically,
        // so this self-test doubles as a field canary for C-01 — the deadlock
        // that parked the accept loop in production from 2026-11-02.
        let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) };
        let Ok(event) = event else {
            eprintln!("[self-test] CreateEventW failed");
            unsafe {
                CloseHandle(server_handle).ok();
            }
            return 1;
        };

        // Now connect as the current (unprivileged) user → the whole point.
        let client_handle = unsafe {
            CreateFileW(
                PCWSTR(pipe_name_w.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
        };
        if client_handle.is_err() {
            eprintln!(
                "[self-test] FAIL: client CreateFileW -> ACCESS_DENIED ({}) — DACL is SYSTEM-only; \
                 the real app would hit the same wall (v0.1.16 bug).",
                std::io::Error::last_os_error()
            );
            unsafe {
                CloseHandle(server_handle).ok();
                CloseHandle(event).ok();
            }
            return 1;
        }
        let client_handle = client_handle.unwrap();

        // Run the real accept step (same code as the service's accept loop) on
        // a helper thread with a bounded wait: if the ERROR_PIPE_CONNECTED
        // handling ever regresses, the self-test fails loudly in 5 s instead of
        // hanging forever on an event that can never be signalled.
        let (accept_tx, accept_rx) = std::sync::mpsc::channel();
        // HANDLE is !Send: hand it over through the same newtypes the worker
        // pool uses.
        let accept_pipe = SendHandle(server_handle);
        let accept_event = SendEvent(event);
        std::thread::spawn(move || {
            let _ = accept_tx.send(accept_client(
                accept_pipe.into_handle(),
                accept_event.into_event(),
            ));
        });
        let accept_outcome = match accept_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(outcome) => outcome,
            Err(_) => {
                eprintln!(
                    "[self-test] FAIL: accept step did not complete within 5 s — a pre-connected \
                     client (ERROR_PIPE_CONNECTED) must never be waited on (C-01 deadlock)",
                );
                unsafe {
                    CloseHandle(server_handle).ok();
                    CloseHandle(event).ok();
                    CloseHandle(client_handle).ok();
                }
                return 1;
            }
        };
        if let AcceptOutcome::Failed(code) = accept_outcome {
            eprintln!("[self-test] FAIL: accept step failed (win32 error {code})");
            unsafe {
                CloseHandle(server_handle).ok();
                CloseHandle(event).ok();
                CloseHandle(client_handle).ok();
            }
            return 1;
        }

        // Round-trip: client writes "ping", server reads it.
        let ping = b"ping";
        let mut written = 0u32;
        let wr = unsafe { WriteFile(client_handle, Some(ping), Some(&mut written), None) };
        if wr.is_err() || written != ping.len() as u32 {
            eprintln!(
                "[self-test] FAIL: client write failed: {}",
                std::io::Error::last_os_error()
            );
            unsafe {
                CloseHandle(server_handle).ok();
                CloseHandle(event).ok();
                CloseHandle(client_handle).ok();
            }
            return 1;
        }

        let mut buf = [0u8; 16];
        let mut bytes_read = 0u32;
        let rd = unsafe { ReadFile(server_handle, Some(&mut buf), Some(&mut bytes_read), None) };
        if rd.is_ok() && bytes_read >= 4 && &buf[..4] == ping {
            println!("[self-test] PASS: unprivileged client connected + round-tripped (DACL OK)");
            let code = 0;
            unsafe {
                DisconnectNamedPipe(server_handle).ok();
                CloseHandle(server_handle).ok();
                CloseHandle(event).ok();
                CloseHandle(client_handle).ok();
            }
            code
        } else {
            eprintln!(
                "[self-test] FAIL: server read did not match (DACL may be OK but I/O broken)"
            );
            unsafe {
                CloseHandle(server_handle).ok();
                CloseHandle(event).ok();
                CloseHandle(client_handle).ok();
            }
            1
        }
    }

    // ── C-01 regression tests ───────────────────────────────────────────────
    // Only compiled for `cargo test --bin micontrol_bridge`; the whole file is
    // `#![cfg(windows)]`, so these are Windows-only by construction. They use
    // real named pipes (the race cannot be faked) but never touch
    // `handle_client` / `bridge_log`, so they do not write to the production
    // `ProgramData\MiControl\logs\bridge.log`.
    #[cfg(test)]
    mod c01_accept_tests {
        use super::*;
        use std::os::windows::ffi::OsStrExt;
        use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};
        use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };
        use windows::Win32::System::IO::GetOverlappedResult;

        /// Unique pipe name per call (pid + counter), so the tests can run in
        /// parallel and a leftover instance from a crashed run cannot collide.
        fn unique_pipe_name(tag: &str) -> String {
            static SEQ: AtomicU32 = AtomicU32::new(0);
            format!(
                r"\\.\pipe\micontrol_bridge_c01_{tag}_{}_{}",
                std::process::id(),
                SEQ.fetch_add(1, AtomicOrdering::SeqCst)
            )
        }

        fn wide(name: &str) -> Vec<u16> {
            std::ffi::OsStr::new(name)
                .encode_wide()
                .chain(Some(0))
                .collect()
        }

        /// A listening instance with the production flags (overlapped, byte
        /// mode, Everyone-RW DACL).
        fn listening_instance(name: &str) -> PipeInstance {
            let sec_attr = build_pipe_security_attributes();
            PipeInstance::create(&wide(name), 1, sec_attr.as_ref())
                .expect("CreateNamedPipeW/CreateEventW failed")
        }

        /// Client side. A SYNCHRONOUS handle (no FILE_FLAG_OVERLAPPED) so the
        /// test can use plain blocking WriteFile; the server side stays
        /// overlapped exactly like production.
        fn connect_client(name: &str) -> HANDLE {
            let path = wide(name);
            unsafe {
                CreateFileW(
                    PCWSTR(path.as_ptr()),
                    (GENERIC_READ | GENERIC_WRITE).0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    None,
                )
            }
            .expect("client CreateFileW failed")
        }

        fn client_write(handle: HANDLE, payload: &[u8]) {
            let mut written = 0u32;
            unsafe { WriteFile(handle, Some(payload), Some(&mut written), None) }
                .expect("client WriteFile failed");
            assert_eq!(written, payload.len() as u32, "short client write");
        }

        /// Server-side read on an overlapped instance handle. `None` means the
        /// client is gone (`ERROR_BROKEN_PIPE` / `ERROR_PIPE_NOT_CONNECTED`) —
        /// the condition `handle_client` maps to "read 0 bytes".
        fn server_read(handle: HANDLE, buf: &mut [u8]) -> Option<usize> {
            let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
                .expect("server_read: CreateEventW failed");
            let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
            overlapped.hEvent = event;
            let mut transferred = 0u32;

            let started = unsafe {
                ReadFile(
                    handle,
                    Some(buf),
                    Some(&mut transferred),
                    Some(&mut overlapped),
                )
            };
            let immediate = match started {
                Ok(()) => Some(transferred as usize),
                Err(_) => None,
            };

            let result = match immediate {
                Some(n) => Some(n),
                None => {
                    let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32;
                    if code != ERROR_IO_PENDING.0 {
                        None // client vanished: EOF
                    } else {
                        assert_eq!(
                            unsafe { WaitForSingleObject(event, 5_000) },
                            WAIT_OBJECT_0,
                            "server read never completed"
                        );
                        let mut done = 0u32;
                        unsafe { GetOverlappedResult(handle, &overlapped, &mut done, false) }
                            .expect("GetOverlappedResult failed");
                        Some(done as usize)
                    }
                }
            };

            unsafe { CloseHandle(event).ok() };
            result
        }

        /// One accept step on a helper thread with a 5 s guard: if the C-01
        /// bug ever comes back, the test fails instead of hanging the suite.
        fn accept_with_timeout(handle: HANDLE, event: HANDLE) -> AcceptOutcome {
            let (tx, rx) = std::sync::mpsc::channel();
            let pipe = SendHandle(handle);
            let accept_event = SendEvent(event);
            std::thread::spawn(move || {
                let _ = tx.send(accept_client(pipe.into_handle(), accept_event.into_event()));
            });
            rx.recv_timeout(std::time::Duration::from_secs(5)).expect(
                "accept step did not complete within 5 s — ERROR_PIPE_CONNECTED deadlock (C-01)",
            )
        }

        /// C-01, the exact production race: the client is connected BEFORE the
        /// accept step runs. `ConnectNamedPipe` then returns FALSE +
        /// ERROR_PIPE_CONNECTED *without queueing overlapped I/O*; the pre-fix
        /// code `let _ =`'d that result and waited INFINITE on an event that can
        /// never be signalled, parking the accept loop for good (service
        /// RUNNING, pipe permanently ERROR_BUSY).
        #[test]
        fn accept_completes_when_client_connects_before_connect_named_pipe() {
            let name = unique_pipe_name("race");
            let instance = listening_instance(&name);

            // Client first — this IS the race.
            let client = connect_client(&name);

            let outcome = accept_with_timeout(instance.handle, instance.event);
            assert_eq!(
                outcome,
                AcceptOutcome::AlreadyConnected,
                "a pre-connected client must be recognised as ERROR_PIPE_CONNECTED"
            );

            // And the connection must be USABLE, not merely "not blocked".
            client_write(client, b"ping");
            let mut buf = [0u8; 16];
            let n = server_read(instance.handle, &mut buf).expect("server read failed");
            assert_eq!(&buf[..n], b"ping".as_slice());

            unsafe { CloseHandle(client).ok() };
        }

        /// Normal path: no client yet, so `ConnectNamedPipe` pends
        /// (ERROR_IO_PENDING) and the accept step must return exactly when the
        /// client connects — never before.
        #[test]
        fn accept_returns_when_client_connects_after_connect_named_pipe() {
            let name = unique_pipe_name("normal");
            let instance = listening_instance(&name);

            let (tx, rx) = std::sync::mpsc::channel();
            let (handle, event) = (instance.handle, instance.event);
            let pipe = SendHandle(handle);
            let accept_event = SendEvent(event);
            std::thread::spawn(move || {
                let _ = tx.send(accept_client(pipe.into_handle(), accept_event.into_event()));
            });

            // Let the helper reach the blocking wait, so this really exercises
            // the ERROR_IO_PENDING path rather than the pre-connected one.
            std::thread::sleep(std::time::Duration::from_millis(300));
            assert!(
                matches!(rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)),
                "accept step returned before any client connected"
            );

            let client = connect_client(&name);
            let outcome = rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("accept step never woke up when the client connected");
            assert_eq!(outcome, AcceptOutcome::Connected);

            client_write(client, b"pong");
            let mut buf = [0u8; 16];
            let n = server_read(instance.handle, &mut buf).expect("server read failed");
            assert_eq!(&buf[..n], b"pong".as_slice());

            unsafe { CloseHandle(client).ok() };
        }

        /// A client that connects and immediately disappears must not wedge the
        /// accept step, and the loop must keep serving afterwards — the
        /// production signature was "client connected" / "read 0 bytes" right
        /// before bridge.log went silent for five days.
        #[test]
        fn accept_step_survives_client_that_connects_and_disconnects() {
            let name = unique_pipe_name("ghost");

            // Round 1: a probe connects and vanishes without sending anything.
            {
                let instance = listening_instance(&name);
                let ghost = connect_client(&name);
                assert!(matches!(
                    accept_with_timeout(instance.handle, instance.event),
                    AcceptOutcome::Connected | AcceptOutcome::AlreadyConnected
                ));
                unsafe { CloseHandle(ghost).ok() };

                let mut buf = [0u8; 16];
                assert_eq!(
                    server_read(instance.handle, &mut buf),
                    None,
                    "a vanished client must surface as EOF, never as a hang"
                );
            }

            // Round 2: same pipe name, fresh instance — the loop can still
            // accept and serve a real client (the name is released on drop).
            let instance = listening_instance(&name);
            let client = connect_client(&name);
            assert_eq!(
                accept_with_timeout(instance.handle, instance.event),
                AcceptOutcome::AlreadyConnected
            );

            client_write(client, b"pong");
            let mut buf = [0u8; 16];
            let n = server_read(instance.handle, &mut buf).expect("server read failed");
            assert_eq!(&buf[..n], b"pong".as_slice());

            unsafe { CloseHandle(client).ok() };
        }

        /// A hard `ConnectNamedPipe` failure must surface as `Failed(code)` so
        /// the accept loop destroys the instance and logs it, instead of being
        /// swallowed and mistaken for a connection.
        #[test]
        fn accept_reports_hard_failure_instead_of_assuming_connected() {
            // Null handle: ConnectNamedPipe fails with ERROR_INVALID_HANDLE.
            let outcome = accept_client(HANDLE::default(), HANDLE::default());
            assert!(
                matches!(outcome, AcceptOutcome::Failed(_)),
                "expected a hard failure, got {outcome:?}"
            );
        }
    }
}

// ── Watchdog ─────────────────────────────────────────────────────────────────
//
// S26-006: "MiControl must NEVER stop unless the user quits it manually."
//
// Root cause of spontaneous termination (proven via live crash dump):
//   WebView2's `ControlLib.dll` registers an ETW provider; when the DLL is
//   unloaded (WebView2 browser-process recycling / window teardown) the ETW
//   callback is left registered. ntdll's ETW notification thread later calls
//   into the unloaded DLL → executes non-executable memory → ACCESS_VIOLATION
//   (0xc0000005, Execute type) → the whole process dies with no Rust panic and
//   no chance to flush logs. This can also be observed as
//   `combase.dll`-based `0xc0000005` buckets in WER.
//
//   Nothing in MiControl application code can prevent this (it's an upstream
//   WebView2/ETW race), so the durable fix is external: the MiControlBridge
//   Windows service (runs 24/7 as LOCAL SYSTEM, started at boot) supervises
//   `micontrol.exe` and relaunches it into the interactive user session
//   whenever it disappears unexpectedly.
//
// Relaunch is gated on TWO machine-wide markers in `%ProgramData%\MiControl`:
//   - `watchdog_enabled`   — present AND `"1"` iff the user has autostart
//                            enabled (mirrored by the app at every startup and
//                            toggled with it). This is the production arming
//                            source: the file has existed since 2026-11-06
//                            22:19 but the service ignored it until now.
//   - `watchdog_user_quit` — sentinel written by the app on INTENTIONAL quit
//                            (tray Quit, dev window close). Its presence keeps
//                            the watchdog silent indefinitely; the app REMOVES
//                            it on its own startup, re-arming the watchdog for
//                            that session. This honors the requirement exactly:
//                            a manual quit stops MiControl for good (until the
//                            user starts it again), while any other death is
//                            auto-relaunched.
//
// The watchdog thread itself starts when either the `MCONTROL_WATCHDOG=1`
// environment variable (operator override) or the `watchdog_enabled` marker is
// present; `MCONTROL_WATCHDOG=0` force-disables it even when the marker exists.
mod watchdog {
    use super::*;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
    use windows::Win32::System::RemoteDesktop::{WTSGetActiveConsoleSessionId, WTSQueryUserToken};
    use windows::Win32::System::Threading::{CreateProcessAsUserW, STARTUPINFOW};

    /// Poll interval (ms) — 30 s. Crashes recurred in clusters every few
    /// minutes; 30 s keeps worst-case downtime well under a minute while
    /// being cheap for a SYSTEM service.
    const POLL_MS: u64 = 30_000;
    /// ProgramData base for markers.
    const MARKER_DIR: &str = r"MiControl";
    const ENABLED_MARKER: &str = "watchdog_enabled";
    const QUIT_MARKER: &str = "watchdog_user_quit";
    const HEARTBEAT_FILE: &str = "heartbeat";
    /// A heartbeat older than this (without the app refreshing it) means the
    /// process exists but its UI is dead (frozen zombie after a WebView2
    /// crash/relaunch). The watchdog force-restarts such instances.
    ///
    /// The app's ticker is a dedicated OS thread with a 10 s period, so 45 s
    /// is 4.5 missed ticks — no healthy process ever trips this.
    const HEARTBEAT_STALE_MS: u64 = 45_000;
    /// How long `ui_ok=0` must be observed before acting.
    ///
    /// The app reports `ui_ok=0` from the very first heartbeat after process
    /// start (the window does not exist yet), so acting on the first sample
    /// would kill every launch. 60 s is two poll periods — the confirmation
    /// window the stale path has always used, now explicit and time-based.
    const UI_DEAD_GRACE_MS: u64 = 60_000;
    /// After a watchdog-initiated relaunch, ignore missing/stale heartbeats
    /// for this long while the app boots WebView2.
    const STARTUP_GRACE_MS: u64 = 120_000;
    /// Consecutive polls of a stale/unreadable heartbeat required to act.
    const ZOMBIE_CONFIRMATIONS: u32 = 2;
    /// Max restart actions without a sustained healthy period ([`HEALTHY_RESET_MS`]).
    /// Bounds a kill loop if the app cannot recover.
    const MAX_RESTARTS: u32 = 3;
    /// Continuous healthy time that resets the restart budget.
    const HEALTHY_RESET_MS: u64 = 600_000;
    /// Consecutive "app not running" polls before relaunching (~2 min).
    const DEAD_MISSES_BEFORE_LAUNCH: u32 = 4;
    /// Read attempts for the heartbeat file. Writer uses write-then-rename,
    /// but a fallback direct write can be observed mid-truncation; retrying
    /// for ~50 ms avoids misreading a torn file as a dead UI.
    const HEARTBEAT_READ_ATTEMPTS: u32 = 3;
    /// Default install location fallback.
    const DEFAULT_EXE: &str = r"C:\Program Files\MiControl\micontrol.exe";
    /// Uninstall registry keys (DisplayIcon / InstallLocation) to find a
    /// relocated install.
    const UNINSTALL_KEYS: &[&str] = &[
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    ];

    fn marker_dir() -> std::path::PathBuf {
        std::env::var_os("ProgramData")
            .map(|pd| std::path::PathBuf::from(pd).join(MARKER_DIR))
            .unwrap_or_else(|| std::path::PathBuf::from(r"C:\ProgramData").join(MARKER_DIR))
    }

    fn marker_path(name: &str) -> std::path::PathBuf {
        marker_dir().join(name)
    }

    fn write_log(msg: &str) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        eprintln!("[{ts}] [watchdog] {msg}");
        // S64: persist with size-based rotation (the previous comment said
        // "rotating log" but the implementation was a plain unbounded append).
        let dir = marker_dir().join("logs");
        let path = dir.join("watchdog.log");
        let _ = micontrol_lib::util::log_rotation::append_line(
            &path,
            &format!("{ts} {msg}"),
            micontrol_lib::util::log_rotation::DEFAULT_MAX_BYTES,
            micontrol_lib::util::log_rotation::DEFAULT_MAX_FILES,
        );
    }

    /// True if the user has explicitly quit the app since it last started.
    ///
    /// The sentinel is written by the app's tray Quit handler and REMOVED by
    /// the app's own startup (setup). While it exists, the watchdog must NOT
    /// relaunch — "MiControl never stops unless the user quits it manually"
    /// means exactly that: a manual quit is armed forever, until the user
    /// starts the app again themselves.
    fn user_quit_pending() -> bool {
        // Logging is done by `log_gate_transition` — one line per transition,
        // not one line every 30 s poll.
        marker_path(QUIT_MARKER).exists()
    }

    /// True if the watchdog should be armed (autostart mirror says `"1"`).
    ///
    /// The app writes exactly `1` (`crash_recovery::set_watchdog_enabled`) and
    /// removes the file when autostart is off. Treat an empty file as armed
    /// (older builds wrote it before the content mattered) and `"0"` as
    /// explicitly disarmed.
    fn watchdog_armed() -> bool {
        let path = marker_path(ENABLED_MARKER);
        match std::fs::read_to_string(&path) {
            Ok(content) => content.trim() != "0",
            Err(_) => false,
        }
    }

    /// Should the watchdog thread start at all?
    ///
    /// `MCONTROL_WATCHDOG=1` / `=0` is an explicit operator override and wins
    /// over the marker; otherwise the app's `watchdog_enabled` file decides.
    /// S55 FIX 19 disabled the thread unconditionally after an access violation
    /// in the Win32 launch path, which left `ui_ok` with no consumer: the
    /// 06-Nov-2026 starvation ran for 2 h 46 min before the user quit the app.
    pub fn start_enabled() -> bool {
        match std::env::var("MCONTROL_WATCHDOG")
            .ok()
            .as_deref()
            .map(str::trim)
        {
            Some("1") => return true,
            Some("0") => return false,
            _ => {}
        }
        watchdog_armed()
    }

    /// Which source armed the watchdog (for the startup log line).
    fn arm_source() -> &'static str {
        match std::env::var("MCONTROL_WATCHDOG")
            .ok()
            .as_deref()
            .map(str::trim)
        {
            Some("1") => "env MCONTROL_WATCHDOG=1",
            Some("0") => "env MCONTROL_WATCHDOG=0 (forced off)",
            _ if watchdog_armed() => "marker watchdog_enabled",
            _ => "none",
        }
    }

    /// Returns the PID of a running `micontrol.exe`, if any.
    fn app_pid() -> Option<u32> {
        // leak-check: audited — every exit path closes the handle explicitly (S67).
        let target: Vec<u16> = "micontrol.exe".encode_utf16().collect();
        let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
            return None;
        };
        let snapshot_ok = snapshot != INVALID_HANDLE_VALUE;
        if !snapshot_ok {
            return None;
        }
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut result = None;
        let mut has = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
        while has {
            let mut name: Vec<u16> = entry.szExeFile[..].to_vec();
            while let Some(&0) = name.last() {
                name.pop();
            }
            if name.len() == target.len() && name == target {
                result = Some(entry.th32ProcessID);
                break;
            }
            has = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
        }
        unsafe {
            CloseHandle(snapshot).ok();
        }
        result
    }

    /// Wall-clock now in epoch milliseconds.
    fn epoch_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Parsed contents of `%ProgramData%\MiControl\heartbeat`.
    ///
    /// Writer: `hw::crash_recovery::write_heartbeat`; format
    /// `<epoch_ms>\n<pid>\n<ui_ok>\n`. All three fields are mandatory.
    ///
    /// Before 2026-11-07 `app_heartbeat_fresh()` read **only** line 1, so a
    /// fresh timestamp with `ui_ok=0` — a process whose tokio runtime is
    /// starved and whose UI cannot answer — looked perfectly healthy. The
    /// 06-Nov starvation ran 2 h 46 min that way.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Heartbeat {
        ts_ms: u64,
        pid: u32,
        ui_ok: u8,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum HeartbeatError {
        Missing,
        Empty,
        BadTimestamp,
        MissingPid,
        BadPid,
        MissingUiOk,
        BadUiOk,
    }

    impl HeartbeatError {
        fn as_str(self) -> &'static str {
            match self {
                Self::Missing => "missing",
                Self::Empty => "empty",
                Self::BadTimestamp => "invalid timestamp",
                Self::MissingPid => "missing pid",
                Self::BadPid => "invalid pid",
                Self::MissingUiOk => "missing ui_ok",
                Self::BadUiOk => "invalid ui_ok",
            }
        }
    }

    /// Strict three-line parse. Garbage in any field rejects the whole file:
    /// acting on a half-understood heartbeat is how a watchdog kills a healthy
    /// app. Extra trailing lines are ignored (forward-compatible).
    fn parse_heartbeat(content: &str) -> Result<Heartbeat, HeartbeatError> {
        let mut lines = content.lines();
        let ts = lines.next().ok_or(HeartbeatError::Empty)?.trim();
        if ts.is_empty() {
            return Err(HeartbeatError::Empty);
        }
        let ts_ms = ts
            .parse::<u64>()
            .map_err(|_| HeartbeatError::BadTimestamp)?;
        if ts_ms == 0 {
            return Err(HeartbeatError::BadTimestamp);
        }
        let pid = lines
            .next()
            .ok_or(HeartbeatError::MissingPid)?
            .trim()
            .parse::<u32>()
            .map_err(|_| HeartbeatError::BadPid)?;
        if pid == 0 {
            return Err(HeartbeatError::BadPid);
        }
        let ui_ok = lines
            .next()
            .ok_or(HeartbeatError::MissingUiOk)?
            .trim()
            .parse::<u8>()
            .map_err(|_| HeartbeatError::BadUiOk)?;
        if ui_ok > 1 {
            return Err(HeartbeatError::BadUiOk);
        }
        Ok(Heartbeat { ts_ms, pid, ui_ok })
    }

    /// Read + parse the heartbeat, retrying briefly.
    ///
    /// The app writes it with write-then-rename, but the fallback direct write
    /// can be observed mid-truncation; ~50 ms of retries keeps a torn read
    /// from being misdiagnosed as a dead UI.
    fn read_heartbeat() -> Result<Heartbeat, HeartbeatError> {
        let path = marker_path(HEARTBEAT_FILE);
        let mut last = HeartbeatError::Missing;
        for attempt in 0..HEARTBEAT_READ_ATTEMPTS {
            match std::fs::read_to_string(&path) {
                Ok(content) => match parse_heartbeat(&content) {
                    Ok(hb) => return Ok(hb),
                    Err(e) => last = e,
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(HeartbeatError::Missing)
                }
                Err(_) => last = HeartbeatError::Missing,
            }
            if attempt + 1 < HEARTBEAT_READ_ATTEMPTS {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        Err(last)
    }

    /// What the heartbeat says about the running process.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum HeartbeatHealth {
        /// Fresh, same pid, `ui_ok=1`.
        Healthy,
        /// Fresh, same pid, `ui_ok=0` — main window gone (WebView2 crash) or
        /// the async runtime is starved. Needs the grace period, then restart.
        UiNotReady,
        /// Fresh timestamp, but written by another process — typically the
        /// previous instance right after a relaunch, before the new one has
        /// written its first heartbeat.
        ForeignPid { pid: u32 },
        /// Timestamp older than [`HEARTBEAT_STALE_MS`].
        Stale { age_ms: u64 },
        /// Missing, empty or malformed.
        Unreadable { why: &'static str },
    }

    impl HeartbeatHealth {
        fn is_healthy(self) -> bool {
            matches!(self, Self::Healthy)
        }

        fn describe(self) -> String {
            match self {
                Self::Healthy => "healthy".to_string(),
                Self::UiNotReady => "ui_ok=0".to_string(),
                Self::ForeignPid { pid } => format!("heartbeat from other pid {pid}"),
                Self::Stale { age_ms } => format!("stale {age_ms}ms"),
                Self::Unreadable { why } => format!("unreadable ({why})"),
            }
        }
    }

    /// Classify a heartbeat against the running pid and the clock.
    fn heartbeat_health(
        heartbeat: Result<Heartbeat, HeartbeatError>,
        app_pid: Option<u32>,
        now_ms: u64,
    ) -> HeartbeatHealth {
        let hb = match heartbeat {
            Ok(hb) => hb,
            Err(e) => return HeartbeatHealth::Unreadable { why: e.as_str() },
        };
        match app_pid {
            Some(pid) if hb.pid == pid => {}
            Some(_) => {
                return HeartbeatHealth::ForeignPid { pid: hb.pid };
            }
            None => {
                return HeartbeatHealth::Unreadable {
                    why: "no running pid",
                }
            }
        }
        // A future timestamp (clock skew) is treated as age 0.
        let age_ms = now_ms.saturating_sub(hb.ts_ms);
        if age_ms > HEARTBEAT_STALE_MS {
            return HeartbeatHealth::Stale { age_ms };
        }
        if hb.ui_ok == 1 {
            HeartbeatHealth::Healthy
        } else {
            HeartbeatHealth::UiNotReady
        }
    }

    /// Escalation marker written by `hw::crash_recovery` after N consecutive
    /// failed async-runtime probes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct StarvationSignal {
        pid: u32,
        fails: u32,
        age_ms: u64,
    }

    fn read_starvation(now_ms: u64) -> Option<StarvationSignal> {
        use micontrol_lib::hw::crash_recovery as cr;
        let path = cr::starvation_marker_path()?;
        let content = std::fs::read_to_string(path).ok()?;
        let state = cr::parse_starvation_marker(&content).ok()?;
        if state.fails < cr::STARVATION_ESCALATE_FAILS {
            return None;
        }
        Some(StarvationSignal {
            pid: state.pid,
            fails: state.fails,
            age_ms: now_ms.saturating_sub(state.ts_ms),
        })
    }

    /// A signal is actionable only for the *current* app pid and while fresh;
    /// a leftover marker from a previous session must never trigger a restart.
    fn starvation_actionable(signal: StarvationSignal, app_pid: u32) -> bool {
        use micontrol_lib::hw::crash_recovery as cr;
        signal.pid == app_pid
            && signal.fails >= cr::STARVATION_ESCALATE_FAILS
            && signal.age_ms <= cr::STARVATION_MARKER_TTL_MS
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum RestartCause {
        /// No `micontrol.exe` at all.
        Missing,
        /// Process alive, heartbeat stale/foreign/unreadable.
        Stale,
        /// Fresh heartbeat with `ui_ok=0`.
        UiDead,
        /// `crash_recovery` escalated a starved async runtime.
        Starved,
    }

    impl RestartCause {
        fn as_str(self) -> &'static str {
            match self {
                Self::Missing => "app not running",
                Self::Stale => "heartbeat stale",
                Self::UiDead => "UI dead (ui_ok=0)",
                Self::Starved => "async runtime starved",
            }
        }
    }

    /// Why a poll decided to wait instead of acting — logged and asserted by
    /// the decision-matrix tests.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum WaitDetail {
        Missing { misses: u32 },
        StartupGrace,
        UiDeadGrace { since_ms: u64 },
        Confirm { count: u32 },
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum WatchdogAction {
        Idle,
        Wait {
            cause: RestartCause,
            detail: WaitDetail,
        },
        Relaunch {
            misses: u32,
        },
        Restart {
            cause: RestartCause,
        },
        GiveUp {
            cause: RestartCause,
        },
    }

    /// Which gate is currently blocking supervision (for one-line logging on
    /// transitions instead of a line every 30 s).
    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    enum GateState {
        #[default]
        Armed,
        QuitPending,
        Disarmed,
    }

    /// Counters and timers that survive between polls.
    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    struct WatchdogState {
        dead_misses: u32,
        zombie_confirmations: u32,
        ui_dead_since_ms: Option<u64>,
        last_launch_ms: Option<u64>,
        restarts: u32,
        healthy_since_ms: Option<u64>,
        gave_up: bool,
        gate: GateState,
    }

    /// Everything a poll observes. Kept as plain data so [`decide`] is pure
    /// and the whole matrix is unit-testable without Win32 or files.
    struct WatchdogInput {
        quit_pending: bool,
        armed: bool,
        app_pid: Option<u32>,
        heartbeat: HeartbeatHealth,
        starvation: Option<StarvationSignal>,
        now_ms: u64,
    }

    /// The watchdog decision matrix.
    ///
    /// Order matters:
    /// 1. quit sentinel / not armed → stand down (reset counters);
    /// 2. app absent → 4 misses, then relaunch;
    /// 3. healthy (fresh **and** `ui_ok=1` **and** matching pid) → reset;
    /// 4. escalated starvation → restart immediately (the app itself waited
    ///    for 10 consecutive probe failures ≈ 2.5 min);
    /// 5. fresh but `ui_ok=0` → 60 s grace, then restart;
    /// 6. stale/foreign/unreadable → startup grace after our own relaunch,
    ///    then 2 confirmations, then restart;
    /// 7. restart budget exhausted → give up until a sustained healthy period.
    fn decide(input: &WatchdogInput, state: &mut WatchdogState) -> WatchdogAction {
        if input.quit_pending {
            *state = WatchdogState {
                gate: GateState::QuitPending,
                ..WatchdogState::default()
            };
            return WatchdogAction::Idle;
        }
        if !input.armed {
            *state = WatchdogState {
                gate: GateState::Disarmed,
                ..WatchdogState::default()
            };
            return WatchdogAction::Idle;
        }
        state.gate = GateState::Armed;

        let Some(pid) = input.app_pid else {
            state.dead_misses += 1;
            state.zombie_confirmations = 0;
            state.ui_dead_since_ms = None;
            state.healthy_since_ms = None;
            if state.dead_misses < DEAD_MISSES_BEFORE_LAUNCH {
                return WatchdogAction::Wait {
                    cause: RestartCause::Missing,
                    detail: WaitDetail::Missing {
                        misses: state.dead_misses,
                    },
                };
            }
            let misses = state.dead_misses;
            state.dead_misses = 0;
            return start_restart(RestartCause::Missing, state, input.now_ms, misses);
        };

        if input.heartbeat.is_healthy() {
            state.dead_misses = 0;
            state.zombie_confirmations = 0;
            state.ui_dead_since_ms = None;
            let since = *state.healthy_since_ms.get_or_insert(input.now_ms);
            if state.restarts > 0 && input.now_ms.saturating_sub(since) >= HEALTHY_RESET_MS {
                state.restarts = 0;
                state.gave_up = false;
            }
            return WatchdogAction::Idle;
        }
        state.healthy_since_ms = None;

        // crash_recovery already counted 10 consecutive failed probes; that is
        // stronger evidence than any confirmation we could add.
        if let Some(signal) = input
            .starvation
            .filter(|signal| starvation_actionable(*signal, pid))
        {
            return start_restart(RestartCause::Starved, state, input.now_ms, signal.fails);
        }

        if input.heartbeat == HeartbeatHealth::UiNotReady {
            state.dead_misses = 0;
            state.zombie_confirmations = 0;
            // A freshly relaunched app reports ui_ok=0 until WebView2 creates
            // the main window; never count that against it.
            if let Some(launched) = state.last_launch_ms {
                if input.now_ms.saturating_sub(launched) < STARTUP_GRACE_MS {
                    return WatchdogAction::Wait {
                        cause: RestartCause::UiDead,
                        detail: WaitDetail::StartupGrace,
                    };
                }
            }
            let since = state.ui_dead_since_ms.unwrap_or(input.now_ms);
            state.ui_dead_since_ms = Some(since);
            if input.now_ms.saturating_sub(since) < UI_DEAD_GRACE_MS {
                return WatchdogAction::Wait {
                    cause: RestartCause::UiDead,
                    detail: WaitDetail::UiDeadGrace { since_ms: since },
                };
            }
            return start_restart(RestartCause::UiDead, state, input.now_ms, 0);
        }

        state.dead_misses = 0;
        state.ui_dead_since_ms = None;
        // After a relaunch the app needs a moment to create its window and
        // write the first heartbeat; never kill it inside that window.
        if let Some(launched) = state.last_launch_ms {
            if input.now_ms.saturating_sub(launched) < STARTUP_GRACE_MS {
                return WatchdogAction::Wait {
                    cause: RestartCause::Stale,
                    detail: WaitDetail::StartupGrace,
                };
            }
        }
        state.zombie_confirmations += 1;
        if state.zombie_confirmations < ZOMBIE_CONFIRMATIONS {
            return WatchdogAction::Wait {
                cause: RestartCause::Stale,
                detail: WaitDetail::Confirm {
                    count: state.zombie_confirmations,
                },
            };
        }
        start_restart(RestartCause::Stale, state, input.now_ms, 0)
    }

    /// Common bookkeeping for both "launch because absent" and "kill+relaunch".
    fn start_restart(
        cause: RestartCause,
        state: &mut WatchdogState,
        now_ms: u64,
        detail: u32,
    ) -> WatchdogAction {
        if state.restarts >= MAX_RESTARTS {
            if !state.gave_up {
                state.gave_up = true;
                return WatchdogAction::GiveUp { cause };
            }
            return WatchdogAction::Idle;
        }
        state.restarts += 1;
        state.dead_misses = 0;
        state.zombie_confirmations = 0;
        state.ui_dead_since_ms = None;
        state.last_launch_ms = Some(now_ms);
        state.healthy_since_ms = None;
        if cause == RestartCause::Missing {
            WatchdogAction::Relaunch { misses: detail }
        } else {
            WatchdogAction::Restart { cause }
        }
    }

    /// Execute a decision. All Win32/process work lives here; the decision
    /// itself is pure.
    fn apply_action(action: WatchdogAction, input: &WatchdogInput) {
        match action {
            WatchdogAction::Idle => {}
            WatchdogAction::Wait { cause, detail } => match detail {
                WaitDetail::Missing { misses } => {
                    write_log(&format!("micontrol.exe NOT running (miss #{misses})"));
                }
                WaitDetail::StartupGrace => {
                    write_log(&format!(
                        "{} — startup grace ({}ms) after relaunch, waiting",
                        cause.as_str(),
                        STARTUP_GRACE_MS
                    ));
                }
                WaitDetail::UiDeadGrace { since_ms } => {
                    write_log(&format!(
                        "{} for {}ms (grace {}ms) — waiting",
                        cause.as_str(),
                        input.now_ms.saturating_sub(since_ms),
                        UI_DEAD_GRACE_MS
                    ));
                }
                WaitDetail::Confirm { count } => {
                    write_log(&format!(
                        "{} — confirmation {count}/{ZOMBIE_CONFIRMATIONS}",
                        cause.as_str()
                    ));
                }
            },
            WatchdogAction::Relaunch { misses } => {
                write_log(&format!(
                    "micontrol.exe not running after {misses} misses — relaunching (minimized)"
                ));
                if let Some(exe) = find_app_exe() {
                    launch_app(&exe, true);
                }
            }
            WatchdogAction::Restart { cause } => {
                let escalation = match (cause, input.starvation) {
                    (RestartCause::Starved, Some(signal)) => {
                        format!(", crash_recovery fails={}", signal.fails)
                    }
                    _ => String::new(),
                };
                write_log(&format!(
                    "{} ({}{escalation}) — killing micontrol.exe (pid {}) and relaunching",
                    cause.as_str(),
                    input.heartbeat.describe(),
                    input
                        .app_pid
                        .map(|pid| pid.to_string())
                        .unwrap_or_else(|| "?".to_string())
                ));
                if kill_app() {
                    if let Some(exe) = find_app_exe() {
                        // Visible window: the user must see that the UI is back.
                        launch_app(&exe, false);
                    }
                } else {
                    write_log("could not kill the app — waiting for the next poll");
                }
            }
            WatchdogAction::GiveUp { cause } => {
                write_log(&format!(
                    "{} — {MAX_RESTARTS} restarts without a sustained healthy period; \
                     giving up until the next user launch",
                    cause.as_str()
                ));
            }
        }
    }

    /// Log a gate transition once, not on every 30 s poll.
    fn log_gate_transition(previous: GateState, current: GateState) {
        if previous == current {
            return;
        }
        match current {
            GateState::Armed => write_log("watchdog armed — supervising micontrol.exe"),
            GateState::QuitPending => {
                write_log("user quit sentinel present — watchdog stays silent");
            }
            GateState::Disarmed => {
                write_log("watchdog disabled (autostart off) — sleeping");
            }
        }
    }

    /// Force-terminate the running micontrol.exe instance(s). Returns true if
    /// at least one process was killed.
    fn kill_app() -> bool {
        // leak-check: audited — every exit path closes the handle explicitly (S67).
        use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
        let mut killed_any = false;
        while let Some(pid) = app_pid() {
            let handle = unsafe { OpenProcess(PROCESS_TERMINATE, false, pid) }.ok();
            if let Some(h) = handle {
                let ok = unsafe { TerminateProcess(h, 1) }.is_ok();
                unsafe {
                    CloseHandle(h).ok();
                }
                if ok {
                    write_log(&format!("killed frozen micontrol.exe (pid {pid})"));
                    killed_any = true;
                } else {
                    break; // avoid infinite loop if we cannot kill it
                }
            } else {
                break;
            }
            // Give the process a moment to die.
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        killed_any
    }

    /// Locate `micontrol.exe`: Uninstall-registry first, then the default
    /// install path.
    fn find_app_exe() -> Option<std::path::PathBuf> {
        // 1) Uninstall registry entries — DisplayIcon / InstallLocation.
        for root in [
            winreg::enums::HKEY_LOCAL_MACHINE,
            winreg::enums::HKEY_CURRENT_USER,
        ] {
            for sub in UNINSTALL_KEYS {
                if let Ok(key) = winreg::RegKey::predef(root).open_subkey(sub) {
                    // Enumerate subkeys (each app has its own GUID key).
                    for name in key.enum_keys().filter_map(|r| r.ok()) {
                        if let Ok(app) = key.open_subkey(&name) {
                            let di: Option<String> = app.get_value("DisplayIcon").ok();
                            let il: Option<String> = app.get_value("InstallLocation").ok();
                            let dn: Option<String> = app.get_value("DisplayName").ok();
                            let is_mic = dn
                                .as_deref()
                                .map(|s| s.to_lowercase().contains("micontrol"))
                                .unwrap_or(false)
                                || di
                                    .as_deref()
                                    .map(|s| s.to_lowercase().contains("micontrol"))
                                    .unwrap_or(false);
                            if is_mic {
                                if let Some(iloc) = il {
                                    let exe = std::path::PathBuf::from(&iloc).join("micontrol.exe");
                                    if exe.exists() {
                                        write_log(&format!(
                                            "found micontrol at {} (uninstall)",
                                            exe.display()
                                        ));
                                        return Some(exe);
                                    }
                                }
                                if let Some(icon) = di {
                                    // DisplayIcon may be "path,0" — strip index.
                                    let icon = icon.split(',').next().unwrap_or(&icon);
                                    let p = std::path::PathBuf::from(icon);
                                    // Icon path is the exe itself in our installer.
                                    if p.exists()
                                        && p.file_name()
                                            .map(|n| n == "micontrol.exe")
                                            .unwrap_or(false)
                                    {
                                        write_log(&format!(
                                            "found micontrol at {} (display icon)",
                                            p.display()
                                        ));
                                        return Some(p);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // 2) Default install path.
        let fallback = std::path::PathBuf::from(DEFAULT_EXE);
        if fallback.exists() {
            return Some(fallback);
        }
        write_log(&format!(
            "micontrol.exe not found (checked uninstall keys + {DEFAULT_EXE})"
        ));
        None
    }

    /// Launch `micontrol.exe` into the interactive user session.
    ///
    /// The service runs as LOCAL SYSTEM (session 0); to show GUI in the
    /// console session we must query that session's user token and create the
    /// process with it via `CreateProcessAsUserW`.
    ///
    /// `minimized`: when true (normal crash relaunch) start to tray with
    /// `--minimized`; when false (zombie force-restart) start with a VISIBLE
    /// window so the user can immediately see the UI is back.
    fn launch_app(exe: &std::path::Path, minimized: bool) -> bool {
        let session_id = unsafe { WTSGetActiveConsoleSessionId() };
        if session_id == 0xFFFFFFFF || session_id == 0 {
            write_log(&format!(
                "no active console session (id={session_id}) — skip relaunch"
            ));
            return false;
        }
        let mut token = windows::Win32::Foundation::HANDLE(std::ptr::null_mut());
        let q = unsafe { WTSQueryUserToken(session_id, &mut token) };
        if q.is_err() || token.is_invalid() {
            write_log("WTSQueryUserToken failed — skip relaunch");
            return false;
        }

        // Use the absolute exe path as lpApplicationName (unambiguous, no
        // PATH/search-order surprises) and quote the same path as the first
        // token of the command line ("argv[0]") for correct argument parsing.
        let Some(exe_str) = exe.to_str() else {
            write_log("micontrol path is not valid UTF-16 — skip relaunch");
            unsafe {
                CloseHandle(token).ok();
            }
            return false;
        };
        let app_w: Vec<u16> = OsString::from(exe_str)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let cmdline = if minimized {
            OsString::from(format!("\"{exe_str}\" --minimized"))
        } else {
            OsString::from(format!("\"{exe_str}\""))
        };
        let mut cmd_w: Vec<u16> = cmdline.encode_wide().chain(Some(0)).collect();
        let cur_dir = exe.parent().and_then(|p| p.to_str()).map(|s| {
            let v: Vec<u16> = OsString::from(s).encode_wide().chain(Some(0)).collect();
            v
        });
        let cur_dir_ptr = cur_dir
            .as_ref()
            .map(|v| PCWSTR(v.as_ptr()))
            .unwrap_or(PCWSTR::null());
        // IMPORTANT: do NOT set STARTF_USESHOWWINDOW + SW_HIDE and do NOT pass
        // CREATE_NO_WINDOW. A Tauri GUI app must be created with the default
        // startup info and flags (same as launching from Explorer / Run key,
        // which works — autostart uses Run key with `--minimized`). Forcing
        // SW_HIDE/CREATE_NO_WINDOW from a SYSTEM service was observed to leave
        // the relaunched micontrol.exe in a half-initialized state (process +
        // hidden window handle alive, but Tauri setup never completes — no log
        // output, no hotkeys registered, tray-only broken). The app hides its
        // own main window when `--minimized` is present (lib.rs setup).
        let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi = windows::Win32::System::Threading::PROCESS_INFORMATION::default();

        // CRITICAL: build the USER environment block for the session token and
        // pass it as lpEnvironment. With lpEnvironment=NULL,
        // CreateProcessAsUserW inherits the *calling process* environment block
        // (the SYSTEM service's), so USERPROFILE / LOCALAPPDATA / APPDATA point
        // at C:\Windows\System32\config\systemprofile instead of the real user
        // profile. The relaunched app then cannot create its log file, its
        // WebView2 user-data folder, or its SmartBrightness cache — it ends up
        // as a half-initialized zombie (process + single-instance window +
        // heartbeat ticker alive, but NO main window and NO tray popup — the
        // exact 0.1.23-style crash regression after watchdog relaunch).
        let mut env_block: *mut core::ffi::c_void = std::ptr::null_mut();
        let env_ok = unsafe { CreateEnvironmentBlock(&mut env_block, token, false).is_ok() };
        if env_ok && !env_block.is_null() {
            write_log("launch_app: using user environment block (CreateEnvironmentBlock OK)");
        } else {
            write_log("launch_app: CreateEnvironmentBlock failed — falling back to NULL env (may cause zombie relaunch)");
        }

        // CreateEnvironmentBlock yields a UNICODE (UTF-16) environment block;
        // per MSDN, when lpEnvironment points at a Unicode block the
        // CREATE_UNICODE_ENVIRONMENT flag MUST be set in dwCreationFlags,
        // otherwise CreateProcessAsUserW fails with ERROR_INVALID_PARAMETER
        // (0x80070057 — observed live after the env-block fix alone).
        let mut flags = windows::Win32::System::Threading::PROCESS_CREATION_FLAGS(0);
        if env_ok && !env_block.is_null() {
            flags |= windows::Win32::System::Threading::CREATE_UNICODE_ENVIRONMENT;
        }

        let ok = unsafe {
            CreateProcessAsUserW(
                token,
                PCWSTR(app_w.as_ptr()),
                windows::core::PWSTR(cmd_w.as_mut_ptr()),
                None,
                None,
                false,
                flags, // no CREATE_NO_WINDOW — normal GUI launch; +CREATE_UNICODE_ENVIRONMENT with user env block
                if env_ok && !env_block.is_null() {
                    Some(env_block as *const core::ffi::c_void)
                } else {
                    None
                },
                cur_dir_ptr,
                &si,
                &mut pi,
            )
        };

        unsafe {
            if env_ok && !env_block.is_null() {
                let _ = DestroyEnvironmentBlock(env_block as *const core::ffi::c_void);
            }
            CloseHandle(token).ok();
            if ok.is_ok() {
                CloseHandle(pi.hProcess).ok();
                CloseHandle(pi.hThread).ok();
            }
        }

        match ok {
            Ok(()) => {
                write_log(&format!(
                    "relaunched {} into session {}",
                    exe.display(),
                    session_id
                ));
                true
            }
            Err(e) => {
                write_log(&format!("CreateProcessAsUserW failed: {e}"));
                false
            }
        }
    }

    /// Watchdog main loop.
    pub fn run(shutdown: Arc<AtomicBool>, delay_start_ms: u64) {
        write_log(&format!(
            "watchdog thread started (armed by: {}; poll {}ms, stale {}ms, ui_ok grace {}ms)",
            arm_source(),
            POLL_MS,
            HEARTBEAT_STALE_MS,
            UI_DEAD_GRACE_MS
        ));
        // Give the app time to have (re)started before first probe.
        if delay_start_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay_start_ms));
        }
        let mut state = WatchdogState::default();
        while !shutdown.load(Ordering::SeqCst) {
            // S55 FIX 18: the whole poll body runs inside catch_unwind. A
            // panic anywhere here (WTS token paths, registry reads, process
            // enumeration — all unsafe Win32) previously unwound past the
            // thread boundary and TERMINATED THE WHOLE SYSTEM SERVICE,
            // producing an endless crash/restart loop every ~45 s with the
            // pipe down between restarts (the "abas em Loading eterno" and
            // UAC fallback storms the user saw). A panicked poll is now just
            // a skipped tick: the service survives and the next poll retries.
            let poll_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                watchdog_poll(&mut state);
            }));
            if poll_result.is_err() {
                write_log("watchdog poll PANICKED — tick skipped, service kept alive");
                state = WatchdogState::default();
            }
            // Sleep in small steps so shutdown is prompt.
            let step = std::time::Duration::from_millis(250);
            let mut left = POLL_MS;
            while left > 0 && !shutdown.load(Ordering::SeqCst) {
                std::thread::sleep(step);
                left = left.saturating_sub(step.as_millis() as u64);
            }
        }
        write_log("watchdog thread stopped");
    }

    /// One watchdog poll: observe everything, decide (pure), then act.
    ///
    /// S55 FIX 18: extracted so the watchdog loop can run it inside
    /// catch_unwind — a panic here must skip the tick, not kill the service.
    fn watchdog_poll(state: &mut WatchdogState) {
        let now_ms = epoch_ms();
        let quit_pending = user_quit_pending();
        let armed = watchdog_armed();
        let app_pid = app_pid();
        let heartbeat = heartbeat_health(read_heartbeat(), app_pid, now_ms);
        let starvation = read_starvation(now_ms);
        let input = WatchdogInput {
            quit_pending,
            armed,
            app_pid,
            heartbeat,
            starvation,
            now_ms,
        };

        let previous_gate = state.gate;
        let action = decide(&input, state);
        log_gate_transition(previous_gate, state.gate);
        apply_action(action, &input);
    }

    // ── Tests ───────────────────────────────────────────────────────────────

    #[cfg(test)]
    mod tests {
        use super::*;

        const T0: u64 = 1_794_044_373_192;
        const PID: u32 = 2944;

        fn hb(age_ms: u64, ui_ok: u8) -> HeartbeatHealth {
            heartbeat_health(
                Ok(Heartbeat {
                    ts_ms: T0 - age_ms,
                    pid: PID,
                    ui_ok,
                }),
                Some(PID),
                T0,
            )
        }

        fn input(now_ms: u64, heartbeat: HeartbeatHealth) -> WatchdogInput {
            WatchdogInput {
                quit_pending: false,
                armed: true,
                app_pid: Some(PID),
                heartbeat,
                starvation: None,
                now_ms,
            }
        }

        fn signal(age_ms: u64, fails: u32) -> StarvationSignal {
            StarvationSignal {
                pid: PID,
                fails,
                age_ms,
            }
        }

        // ── Heartbeat parsing ───────────────────────────────────────────────

        #[test]
        fn parses_the_production_heartbeat_format() {
            let parsed = parse_heartbeat("1794044373192\n2944\n1\n").expect("valid");
            assert_eq!(
                parsed,
                Heartbeat {
                    ts_ms: 1_794_044_373_192,
                    pid: 2944,
                    ui_ok: 1
                }
            );
            // CRLF and trailing junk lines are tolerated.
            assert!(parse_heartbeat("1794044373192\r\n2944\r\n0\r\n").is_ok());
            assert!(parse_heartbeat("1794044373192\n2944\n1\nextra\n").is_ok());
        }

        #[test]
        fn rejects_empty_garbage_and_partial_heartbeats() {
            for bad in [
                "",
                "\n\n\n",
                "garbage\n2944\n1\n",
                "0\n2944\n1\n",
                "1794044373192\nnope\n1\n",
                "1794044373192\n0\n1\n",
                "1794044373192\n2944\n",
                "1794044373192\n2944\n2\n",
                "1794044373192\n2944\nmaybe\n",
            ] {
                assert!(
                    parse_heartbeat(bad).is_err(),
                    "must reject heartbeat {bad:?}"
                );
            }
        }

        #[test]
        fn health_classifies_fresh_stale_ui_and_foreign_pid() {
            assert_eq!(hb(0, 1), HeartbeatHealth::Healthy);
            assert_eq!(
                hb(HEARTBEAT_STALE_MS + 1, 1),
                HeartbeatHealth::Stale {
                    age_ms: HEARTBEAT_STALE_MS + 1
                }
            );
            assert_eq!(hb(5_000, 0), HeartbeatHealth::UiNotReady);
            // Clock skew (heartbeat timestamp in the future) is treated as
            // age 0 rather than as stale.
            assert_eq!(
                heartbeat_health(
                    Ok(Heartbeat {
                        ts_ms: T0 + 5_000,
                        pid: PID,
                        ui_ok: 1
                    }),
                    Some(PID),
                    T0
                ),
                HeartbeatHealth::Healthy
            );
            // A heartbeat from another pid is never trusted.
            assert_eq!(
                heartbeat_health(
                    Ok(Heartbeat {
                        ts_ms: T0,
                        pid: 999,
                        ui_ok: 1
                    }),
                    Some(PID),
                    T0
                ),
                HeartbeatHealth::ForeignPid { pid: 999 }
            );
            assert!(matches!(
                heartbeat_health(Err(HeartbeatError::Missing), Some(PID), T0),
                HeartbeatHealth::Unreadable { .. }
            ));
        }

        /// The exact 06-Nov-2026 failure: fresh timestamp, `ui_ok=0`.
        #[test]
        fn fresh_heartbeat_with_ui_ok_zero_is_not_healthy() {
            let health = hb(1_000, 0);
            assert!(!health.is_healthy());
            assert_eq!(health, HeartbeatHealth::UiNotReady);
        }

        // ── Decision matrix ────────────────────────────────────────────────

        #[test]
        fn healthy_app_is_left_alone() {
            let mut state = WatchdogState::default();
            assert_eq!(
                decide(&input(T0, HeartbeatHealth::Healthy), &mut state),
                WatchdogAction::Idle
            );
            assert_eq!(state.restarts, 0);
        }

        #[test]
        fn quit_sentinel_and_disarmed_stand_down() {
            let mut state = WatchdogState {
                restarts: 2,
                dead_misses: 3,
                ..WatchdogState::default()
            };
            let mut quit = input(T0, HeartbeatHealth::Stale { age_ms: 90_000 });
            quit.quit_pending = true;
            assert_eq!(decide(&quit, &mut state), WatchdogAction::Idle);
            assert_eq!(state.gate, GateState::QuitPending);
            assert_eq!(state.restarts, 0, "counters reset while quit is pending");

            let mut disarmed = input(T0, HeartbeatHealth::Healthy);
            disarmed.armed = false;
            assert_eq!(decide(&disarmed, &mut state), WatchdogAction::Idle);
            assert_eq!(state.gate, GateState::Disarmed);
        }

        #[test]
        fn missing_app_relaunches_after_four_misses() {
            let mut state = WatchdogState::default();
            let mut gone = input(T0, HeartbeatHealth::Unreadable { why: "missing" });
            gone.app_pid = None;
            for miss in 1..DEAD_MISSES_BEFORE_LAUNCH {
                assert_eq!(
                    decide(&gone, &mut state),
                    WatchdogAction::Wait {
                        cause: RestartCause::Missing,
                        detail: WaitDetail::Missing { misses: miss }
                    }
                );
            }
            assert_eq!(
                decide(&gone, &mut state),
                WatchdogAction::Relaunch {
                    misses: DEAD_MISSES_BEFORE_LAUNCH
                }
            );
            assert_eq!(state.restarts, 1);
            assert_eq!(state.last_launch_ms, Some(T0));
        }

        /// ui_ok=0 must be observed for the grace period before killing:
        /// the app reports ui_ok=0 on the very first heartbeat of every start.
        #[test]
        fn ui_ok_zero_waits_for_the_grace_period_then_restarts() {
            let mut state = WatchdogState::default();
            assert_eq!(
                decide(&input(T0, hb(0, 0)), &mut state),
                WatchdogAction::Wait {
                    cause: RestartCause::UiDead,
                    detail: WaitDetail::UiDeadGrace { since_ms: T0 }
                }
            );
            assert_eq!(
                decide(&input(T0 + 30_000, hb(0, 0)), &mut state),
                WatchdogAction::Wait {
                    cause: RestartCause::UiDead,
                    detail: WaitDetail::UiDeadGrace { since_ms: T0 }
                }
            );
            assert_eq!(
                decide(&input(T0 + UI_DEAD_GRACE_MS, hb(0, 0)), &mut state),
                WatchdogAction::Restart {
                    cause: RestartCause::UiDead
                }
            );
            assert_eq!(state.restarts, 1);
        }

        /// A single ui_ok=0 sample that recovers must not restart anything.
        #[test]
        fn transient_ui_ok_zero_recovers_without_a_restart() {
            let mut state = WatchdogState::default();
            let _ = decide(&input(T0, hb(0, 0)), &mut state);
            assert_eq!(
                decide(&input(T0 + 30_000, hb(0, 1)), &mut state),
                WatchdogAction::Idle
            );
            assert_eq!(state.ui_dead_since_ms, None);
            assert_eq!(state.restarts, 0);
        }

        /// A freshly relaunched app reports ui_ok=0 until its window exists.
        #[test]
        fn ui_ok_zero_inside_startup_grace_never_kills() {
            let mut state = WatchdogState {
                last_launch_ms: Some(T0),
                ..WatchdogState::default()
            };
            assert_eq!(
                decide(&input(T0 + 30_000, hb(0, 0)), &mut state),
                WatchdogAction::Wait {
                    cause: RestartCause::UiDead,
                    detail: WaitDetail::StartupGrace
                }
            );
            assert_eq!(state.restarts, 0);
        }

        #[test]
        fn stale_heartbeat_needs_two_confirmations() {
            let mut state = WatchdogState::default();
            let stale = HeartbeatHealth::Stale { age_ms: 60_000 };
            assert_eq!(
                decide(&input(T0, stale), &mut state),
                WatchdogAction::Wait {
                    cause: RestartCause::Stale,
                    detail: WaitDetail::Confirm { count: 1 }
                }
            );
            assert_eq!(
                decide(&input(T0 + 30_000, stale), &mut state),
                WatchdogAction::Restart {
                    cause: RestartCause::Stale
                }
            );
        }

        #[test]
        fn startup_grace_after_our_own_relaunch_blocks_a_kill() {
            let mut state = WatchdogState {
                last_launch_ms: Some(T0),
                ..WatchdogState::default()
            };
            let stale = HeartbeatHealth::Stale { age_ms: 50_000 };
            assert_eq!(
                decide(&input(T0 + 30_000, stale), &mut state),
                WatchdogAction::Wait {
                    cause: RestartCause::Stale,
                    detail: WaitDetail::StartupGrace
                }
            );
            // Once the grace expires the normal confirmation applies.
            assert_eq!(
                decide(&input(T0 + STARTUP_GRACE_MS, stale), &mut state),
                WatchdogAction::Wait {
                    cause: RestartCause::Stale,
                    detail: WaitDetail::Confirm { count: 1 }
                }
            );
        }

        /// crash_recovery already waited 10 probe failures (~2.5 min): the
        /// watchdog acts on the first poll that sees a fresh marker.
        #[test]
        fn escalated_starvation_restarts_immediately() {
            let mut state = WatchdogState::default();
            let mut starved = input(T0, hb(0, 0));
            starved.starvation = Some(signal(30_000, 10));
            assert_eq!(
                decide(&starved, &mut state),
                WatchdogAction::Restart {
                    cause: RestartCause::Starved
                }
            );
            assert_eq!(state.restarts, 1);
        }

        #[test]
        fn starvation_marker_from_another_pid_or_too_old_is_ignored() {
            assert!(!starvation_actionable(signal(0, 10), PID + 1));
            assert!(starvation_actionable(signal(0, 10), PID));
            assert!(!starvation_actionable(
                signal(
                    micontrol_lib::hw::crash_recovery::STARVATION_MARKER_TTL_MS + 1,
                    10
                ),
                PID
            ));
            assert!(!starvation_actionable(signal(0, 9), PID));

            let mut state = WatchdogState::default();
            let mut stale_marker = input(T0, hb(0, 0));
            stale_marker.starvation = Some(signal(
                micontrol_lib::hw::crash_recovery::STARVATION_MARKER_TTL_MS + 1,
                10,
            ));
            // Falls back to the normal ui_ok grace — no immediate restart.
            assert_eq!(
                decide(&stale_marker, &mut state),
                WatchdogAction::Wait {
                    cause: RestartCause::UiDead,
                    detail: WaitDetail::UiDeadGrace { since_ms: T0 }
                }
            );
        }

        #[test]
        fn restart_budget_is_bounded_then_gives_up() {
            let mut state = WatchdogState::default();
            let mut starved = input(T0, hb(0, 0));
            starved.starvation = Some(signal(0, 10));
            for _ in 0..MAX_RESTARTS {
                assert!(matches!(
                    decide(&starved, &mut state),
                    WatchdogAction::Restart {
                        cause: RestartCause::Starved
                    }
                ));
            }
            assert_eq!(
                decide(&starved, &mut state),
                WatchdogAction::GiveUp {
                    cause: RestartCause::Starved
                }
            );
            // The give-up line is logged once, then the watchdog stays quiet.
            assert_eq!(decide(&starved, &mut state), WatchdogAction::Idle);
        }

        #[test]
        fn sustained_health_resets_the_restart_budget() {
            let mut state = WatchdogState::default();
            let mut starved = input(T0, hb(0, 0));
            starved.starvation = Some(signal(0, 10));
            for _ in 0..MAX_RESTARTS {
                let _ = decide(&starved, &mut state);
            }
            assert_eq!(state.restarts, MAX_RESTARTS);

            let healthy_at = T0 + 1_000;
            assert_eq!(
                decide(&input(healthy_at, HeartbeatHealth::Healthy), &mut state),
                WatchdogAction::Idle
            );
            assert_eq!(
                state.restarts, MAX_RESTARTS,
                "budget only decays after a while"
            );
            assert_eq!(
                decide(
                    &input(healthy_at + HEALTHY_RESET_MS, HeartbeatHealth::Healthy),
                    &mut state
                ),
                WatchdogAction::Idle
            );
            assert_eq!(state.restarts, 0);
        }
    }
}
