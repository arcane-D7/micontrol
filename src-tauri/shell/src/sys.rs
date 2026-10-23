//! Windows process/service/registry helpers.
//!
//! Strategy: use the PROVEN external-tool patterns from the old NSIS hooks
//! (`sc.exe`, `taskkill.exe`, `pnputil.exe`, `schtasks.exe`) via
//! `std::process::Command` — their exit-code semantics are documented in the
//! repo history (v0.1.21/v0.1.22 lessons: `sc query` on a missing service
//! exits 1060 with the text on stdout; `sc create` errors also land on
//! stdout). Native Win32 is used only where subprocesses cannot help
//! (elevation, process launch in the user session, shortcuts).

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Threading::{
    CreateProcessW, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, PROCESS_CREATION_FLAGS,
    PROCESS_INFORMATION, STARTUPINFOW,
};

/// Wide (NUL-terminated) UTF-16 representation of a string.
pub fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

/// Run a command with no window, capture output, return (exit_code, output).
pub fn run_capture(exe: &str, args: &[&str]) -> (i32, String) {
    let output = Command::new(exe)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW.0)
        .output();
    match output {
        Ok(out) => {
            let code = out.status.code().unwrap_or(-1);
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            let err = String::from_utf8_lossy(&out.stderr);
            if !err.trim().is_empty() {
                if !text.trim().is_empty() {
                    text.push(' ');
                }
                text.push_str(&err);
            }
            (code, text)
        }
        Err(e) => (-1, format!("failed to spawn {exe}: {e}")),
    }
}

/// True when `sc query <name>` reports the service is RUNNING.
pub fn service_is_running(name: &str) -> bool {
    let (code, out) = run_capture("sc.exe", &["query", name]);
    code == 0 && out.contains("RUNNING")
}

/// True when the service exists in the SCM (query exit 0).
pub fn service_exists(name: &str) -> bool {
    run_capture("sc.exe", &["query", name]).0 == 0
}

/// Stop a service and wait (bounded) until it is no longer RUNNING.
/// Best-effort: never fails the caller; returns true when it observed stopped.
pub fn service_stop_wait(name: &str, timeout: Duration) -> bool {
    let start = Instant::now();
    if !service_is_running(name) {
        return true;
    }
    let _ = run_capture("sc.exe", &["stop", name]);
    while start.elapsed() < timeout {
        if !service_is_running(name) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    !service_is_running(name)
}

/// Delete a service entry; BEST-EFFORT (SCM deletes are async and can outlive
/// any fixed wait — the v0.1.22 lesson: NEVER abort on a slow delete).
pub fn service_delete_best_effort(name: &str) {
    let _ = run_capture("sc.exe", &["delete", name]);
}

/// Kill every process with the given image name (taskkill /F /IM).
pub fn kill_process_name(image: &str) {
    let _ = run_capture("taskkill.exe", &["/F", "/IM", image]);
}

/// Kill a process by image name and wait until it is gone (bounded).
pub fn kill_and_wait(image: &str, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if !process_exists(image) {
            return true;
        }
        kill_process_name(image);
        if start.elapsed() > timeout {
            return !process_exists(image);
        }
        std::thread::sleep(Duration::from_millis(700));
    }
}

/// Cheap existence probe: `tasklist /FI "IMAGENAME eq <name>"`.
pub fn process_exists(image: &str) -> bool {
    let (code, out) = run_capture(
        "tasklist.exe",
        &["/FI", &format!("IMAGENAME eq {image}"), "/FO", "CSV", "/NH"],
    );
    code == 0 && out.contains(image)
}

/// Launch a process detached (fire-and-forget) with a hidden window.
pub fn spawn_detached(exe: &Path, args: &[String]) -> Result<(), String> {
    let mut cmdline = format!("\"{}\"", exe.display());
    for a in args {
        if !a.is_empty() {
            cmdline.push(' ');
            cmdline.push_str(&quote_arg(a));
        }
    }
    let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = u32::try_from(std::mem::size_of::<STARTUPINFOW>()).unwrap_or(0);
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let exe_w = wide(&exe.as_os_str().to_string_lossy());
    let mut cmd_w = wide(&cmdline);

    let ok = unsafe {
        CreateProcessW(
            PCWSTR(exe_w.as_ptr()),
            windows::core::PWSTR(cmd_w.as_mut_ptr()),
            None,
            None,
            false,
            PROCESS_CREATION_FLAGS(CREATE_NO_WINDOW.0 | CREATE_UNICODE_ENVIRONMENT.0),
            None,
            None,
            &si,
            &mut pi,
        )
    }
    .is_ok();

    if !ok {
        return Err(format!("CreateProcessW failed for {}", exe.display()));
    }
    unsafe {
        let _ = CloseHandle(pi.hThread);
        let _ = CloseHandle(pi.hProcess);
    }
    Ok(())
}

/// Windows-style argument quoting (public: also used by the elevation path).
pub fn quote_cmd_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains(' ') && !arg.contains('"') {
        return arg.to_string();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                backslashes = 0;
                out.push('"');
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                out.push(ch);
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// Windows-style argument quoting (internal alias).
fn quote_arg(arg: &str) -> String {
    quote_cmd_arg(arg)
}

/// Is the current process token an elevated (admin) token?
pub fn is_elevated() -> bool {
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut handle = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut ret_len = 0u32;
        let ok = GetTokenInformation(
            handle,
            TokenElevation,
            Some(&mut elevation as *mut TOKEN_ELEVATION as *mut core::ffi::c_void),
            u32::try_from(std::mem::size_of::<TOKEN_ELEVATION>()).unwrap_or(0),
            &mut ret_len,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        ok && elevation.TokenIsElevated != 0
    }
}

/// Launch `micontrol.exe` for the logged-on user after install/update.
///
/// Lesson from the v0.1.23 watchdog fix: do NOT pass CREATE_NO_WINDOW or
/// STARTF_USESHOWWINDOW|SW_HIDE — relaunched GUI processes come back as
/// frozen zombies. Use default startup info and let the app manage itself.
pub fn launch_app_for_user(exe: &Path) -> Result<(), String> {
    if !exe.is_file() {
        return Err(format!("app binary not found: {}", exe.display()));
    }
    let cmdline = format!("\"{}\"", exe.display());

    // When running as SYSTEM (invoked via the bridge), create the process in
    // the active console session with the user's token.
    if is_local_system() && launch_in_console_session(&cmdline) {
        return Ok(());
    }

    let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = u32::try_from(std::mem::size_of::<STARTUPINFOW>()).unwrap_or(0);
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let exe_w = wide(&exe.as_os_str().to_string_lossy());
    let mut cmd_w = wide(&cmdline);
    unsafe {
        CreateProcessW(
            PCWSTR(exe_w.as_ptr()),
            windows::core::PWSTR(cmd_w.as_mut_ptr()),
            None,
            None,
            false,
            PROCESS_CREATION_FLAGS(0),
            None,
            None,
            &si,
            &mut pi,
        )
        .map_err(|e| format!("CreateProcessW({}): {e}", exe.display()))?;
        let _ = CloseHandle(pi.hThread);
        let _ = CloseHandle(pi.hProcess);
    }
    Ok(())
}

/// Does the current process run as NT AUTHORITY\SYSTEM?
///
/// Compares the token user SID against the well-known LocalSystem SID.
pub fn is_local_system() -> bool {
    use windows::Win32::Security::{
        GetTokenInformation, IsWellKnownSid, TokenUser, WinLocalSystemSid, TOKEN_QUERY, TOKEN_USER,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut handle = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle).is_err() {
            return false;
        }
        // Size query first (expected to fail with ERROR_INSUFFICIENT_BUFFER).
        let mut len = 0u32;
        let _ = GetTokenInformation(handle, TokenUser, None, 0, &mut len);
        if len == 0 {
            let _ = CloseHandle(handle);
            return false;
        }
        let mut buf = vec![0u8; len as usize];
        let ok = GetTokenInformation(
            handle,
            TokenUser,
            Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
            len,
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return false;
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        IsWellKnownSid(user.User.Sid, WinLocalSystemSid).as_bool()
    }
}

/// Create a process in the active console session using the user's token
/// (WTSQueryUserToken + CreateProcessAsUserW — same approach as the bridge
/// watchdog). Returns false when no interactive session is available.
fn launch_in_console_session(cmdline: &str) -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::RemoteDesktop::{WTSGetActiveConsoleSessionId, WTSQueryUserToken};
    use windows::Win32::System::Threading::{
        CreateProcessAsUserW, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTUPINFOW,
    };

    unsafe {
        let session = WTSGetActiveConsoleSessionId();
        let mut user_token = HANDLE::default();
        if WTSQueryUserToken(session, &mut user_token).is_err() {
            return false;
        }
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = u32::try_from(std::mem::size_of::<STARTUPINFOW>()).unwrap_or(0);
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let mut cmd_w = wide(cmdline);
        let ok = CreateProcessAsUserW(
            user_token,
            None,
            windows::core::PWSTR(cmd_w.as_mut_ptr()),
            None,
            None,
            false,
            PROCESS_CREATION_FLAGS(0),
            None,
            None,
            &si,
            &mut pi,
        )
        .is_ok();
        let _ = CloseHandle(user_token);
        if ok {
            let _ = CloseHandle(pi.hThread);
            let _ = CloseHandle(pi.hProcess);
        }
        ok
    }
}
