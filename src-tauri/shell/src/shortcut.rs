//! Start-menu / desktop shortcut creation via IShellLinkW + IPersistFile.

use std::path::Path;

use windows::core::{Interface, PCWSTR};
use windows::Win32::System::Com::IPersistFile;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

use crate::sys::wide;

/// Create a `.lnk` shortcut pointing at `target`.
pub fn create_shortcut(target: &Path, dest: &Path, description: &str) -> Result<(), String> {
    unsafe {
        // HRESULT (not Result) in windows 0.58: tolerate RPC_E_CHANGED_MODE
        // (COM already initialized on another threading model).
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let changed_mode = hr.0 == 0x8001_0106u32 as i32;
        if hr.is_err() && !changed_mode {
            return Err(format!("CoInitializeEx failed: {hr}"));
        }
        let result = create_shortcut_inner(target, dest, description);
        if hr.is_ok() {
            CoUninitialize();
        }
        result
    }
}

unsafe fn create_shortcut_inner(
    target: &Path,
    dest: &Path,
    description: &str,
) -> Result<(), String> {
    let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
        .map_err(|e| format!("CoCreateInstance(ShellLink): {e}"))?;

    let target_w = wide(&target.as_os_str().to_string_lossy());
    let dir_w = wide(
        &target
            .parent()
            .map(|p| p.as_os_str().to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    let desc_w = wide(description);
    let icon_w = wide(&target.as_os_str().to_string_lossy());

    link.SetPath(PCWSTR(target_w.as_ptr()))
        .map_err(|e| format!("SetPath: {e}"))?;
    link.SetWorkingDirectory(PCWSTR(dir_w.as_ptr()))
        .map_err(|e| format!("SetWorkingDirectory: {e}"))?;
    link.SetDescription(PCWSTR(desc_w.as_ptr()))
        .map_err(|e| format!("SetDescription: {e}"))?;
    link.SetIconLocation(PCWSTR(icon_w.as_ptr()), 0)
        .map_err(|e| format!("SetIconLocation: {e}"))?;

    let persist: IPersistFile = link
        .cast()
        .map_err(|e| format!("cast(IPersistFile): {e}"))?;
    let dest_w = wide(&dest.as_os_str().to_string_lossy());
    persist
        .Save(PCWSTR(dest_w.as_ptr()), true)
        .map_err(|e| format!("Save({}): {e}", dest.display()))?;
    Ok(())
}

/// Delete a `.lnk` if present (best-effort).
pub fn delete_shortcut(dest: &Path) {
    let _ = std::fs::remove_file(dest);
}

/// Paths for the all-users desktop and start-menu shortcuts.
pub fn desktop_shortcut_path() -> std::path::PathBuf {
    let public = std::env::var("PUBLIC").unwrap_or_else(|_| r"C:\Users\Public".into());
    std::path::PathBuf::from(public)
        .join("Desktop")
        .join("MiControl.lnk")
}

pub fn start_menu_shortcut_path() -> std::path::PathBuf {
    let pd = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into());
    std::path::PathBuf::from(pd)
        .join(r"Microsoft\Windows\Start Menu\Programs")
        .join("MiControl.lnk")
}
