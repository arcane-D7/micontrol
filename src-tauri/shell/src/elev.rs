//! Elevation handling: relaunch self with the UAC "runas" verb when the
//! current token is not elevated. Mirrors `RequestExecutionLevel admin` from
//! the old NSIS installer without needing an embedded manifest.

use windows::core::PCWSTR;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::sys::wide;

/// Ensure this process runs elevated. When not, relaunches itself with the
/// UAC prompt, WAITS for the elevated child to finish, and exits with the
/// child's exit code (the parent process never returns from this function in
/// that case). Returns an error when the user cancels the UAC prompt.
pub fn ensure_elevated() -> Result<(), String> {
    if crate::sys::is_elevated() {
        return Ok(());
    }

    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut params = String::new();
    for a in &args {
        if !params.is_empty() {
            params.push(' ');
        }
        params.push_str(&crate::sys::quote_cmd_arg(a));
    }

    let exe_w = wide(&exe.as_os_str().to_string_lossy());
    let verb_w = wide("runas");
    let params_w = wide(&params);

    let mut sei: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    sei.cbSize = u32::try_from(std::mem::size_of::<SHELLEXECUTEINFOW>()).unwrap_or(0);
    sei.fMask = SEE_MASK_NOCLOSEPROCESS;
    sei.lpVerb = PCWSTR(verb_w.as_ptr());
    sei.lpFile = PCWSTR(exe_w.as_ptr());
    sei.lpParameters = PCWSTR(params_w.as_ptr());
    sei.nShow = SW_SHOWNORMAL.0;

    unsafe {
        ShellExecuteExW(&mut sei).map_err(|e| {
            format!("elevation was cancelled or failed ({e}). Run as Administrator to continue.")
        })?;
        if !sei.hProcess.is_invalid() {
            let _ = windows::Win32::System::Threading::WaitForSingleObject(sei.hProcess, INFINITE);
            let mut code: u32 = 0;
            let _ = GetExitCodeProcess(sei.hProcess, &mut code);
            let _ = CloseHandle(sei.hProcess);
            std::process::exit(code as i32);
        }
    }
    // No process handle (unlikely): treat as success of the elevation itself.
    std::process::exit(crate::EXIT_OK);
}
