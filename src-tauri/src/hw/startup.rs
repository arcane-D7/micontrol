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
        // SAFETY: FFI calls to Windows Registry API. HKEY (hkey) is MaybeUninit
        // zero-initialized, then populated by RegOpenKeyExW — assume_init is only
        // reached after a successful return. PCWSTR pointers are derived from
        // null-terminated wide strings whose backing Vec lives for the call
        // duration. RegCloseKey is called with the initialized handle.
        unsafe {
            let key_w: Vec<u16> = OsStr::new(STARTUP_REG_KEY)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let mut hkey = std::mem::MaybeUninit::uninit();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key_w.as_ptr()),
                0,
                windows::Win32::System::Registry::KEY_READ,
                hkey.as_mut_ptr(),
            )
            .is_err()
            {
                return Ok(false);
            }
            let hkey = hkey.assume_init();
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

        // SAFETY: FFI calls to Windows Registry API. HKEY is MaybeUninit
        // zero-initialized and populated by RegOpenKeyExW — assume_init is only
        // reached after success. from_raw_parts on exe_w is valid because the
        // Vec owns contiguous aligned u16 storage; casting the pointer to *const u8
        // and doubling the length gives a correct byte view of the wide string.
        // RegSetValueExW/RegDeleteValueW/RegCloseKey are called with valid handles
        // and null-terminated wide string pointers.
        unsafe {
            let key_w: Vec<u16> = OsStr::new(STARTUP_REG_KEY)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let mut hkey = std::mem::MaybeUninit::uninit();
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key_w.as_ptr()),
                0,
                KEY_WRITE,
                hkey.as_mut_ptr(),
            )
            .ok()
            .context("Open startup registry key")?;
            let hkey = hkey.assume_init();

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
