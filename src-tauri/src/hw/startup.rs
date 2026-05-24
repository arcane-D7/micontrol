use anyhow::{Context, Result};

const STARTUP_REG_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
const STARTUP_REG_VALUE: &str = "MiControl";

pub fn get_autostart() -> Result<bool> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, REG_VALUE_TYPE,
        };
        unsafe {
            let key_w: Vec<u16> = OsStr::new(STARTUP_REG_KEY)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let mut hkey = std::mem::zeroed();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key_w.as_ptr()),
                0,
                windows::Win32::System::Registry::KEY_READ,
                &mut hkey,
            )
            .is_err()
            {
                return Ok(false);
            }
            let val_w: Vec<u16> = OsStr::new(STARTUP_REG_VALUE)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let mut size = 512u32;
            let mut ty = REG_VALUE_TYPE::default();
            let exists = RegQueryValueExW(
                hkey,
                PCWSTR(val_w.as_ptr()),
                None,
                Some(&mut ty),
                None,
                Some(&mut size),
            )
            .is_ok();
            let _ = RegCloseKey(hkey).ok();
            Ok(exists)
        }
    }
    #[cfg(not(windows))]
    {
        Ok(false)
    }
}

pub fn set_autostart(enabled: bool) -> Result<()> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY_CURRENT_USER,
            KEY_WRITE, REG_SZ,
        };

        unsafe {
            let key_w: Vec<u16> = OsStr::new(STARTUP_REG_KEY)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let mut hkey = std::mem::zeroed();
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key_w.as_ptr()),
                0,
                KEY_WRITE,
                &mut hkey,
            )
            .ok()
            .context("Open startup registry key")?;

            let val_w: Vec<u16> = OsStr::new(STARTUP_REG_VALUE)
                .encode_wide()
                .chain(Some(0))
                .collect();

            if enabled {
                // Get current exe path
                let exe_path = std::env::current_exe().context("Get exe path")?;
                let exe_str = format!("\"{}\" --minimized", exe_path.display());
                let exe_w: Vec<u16> = OsStr::new(&exe_str).encode_wide().chain(Some(0)).collect();
                let bytes =
                    std::slice::from_raw_parts(exe_w.as_ptr() as *const u8, exe_w.len() * 2);
                RegSetValueExW(hkey, PCWSTR(val_w.as_ptr()), 0, REG_SZ, Some(bytes))
                    .ok()
                    .context("Write startup entry")?;
            } else {
                let _ = RegDeleteValueW(hkey, PCWSTR(val_w.as_ptr())).ok();
            }

            let _ = RegCloseKey(hkey).ok();
        }
    }
    Ok(())
}
