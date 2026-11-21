//! Open URLs and shell protocol links through `ShellExecuteW`.
//!
//! The shell resolves the default browser (or the `ms-settings:` handler) and
//! launches it directly. Unlike the rundll32 / cmd-start spawn patterns this
//! creates no console window (owner rule B7) and leaves no child process
//! behind that would need a bounded wait.

use crate::hw::errors::{HardwareError, HardwareResult};

/// Open `url` (an `https://` address or a `ms-settings:` protocol link) with
/// the default handler.
///
/// `ShellExecuteW` returns an HINSTANCE value greater than 32 on success;
/// values <= 32 are SE_ERR_* failure codes. A returned 2 (file not found) or
/// similar is reported instead of being swallowed, so a dead protocol handler
/// surfaces as a real error.
pub fn open_url(url: &str) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        if !(url.starts_with("https://")
            || url.starts_with("http://")
            || url.starts_with("ms-settings:"))
        {
            return Err(HardwareError::Other(format!(
                "refusing to open a non-http protocol: {url}"
            )));
        }

        let file: Vec<u16> = std::ffi::OsStr::new(url)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: ShellExecuteW with the "open" verb on a validated URL string;
        // no parent window and no parameters, normal show state. The returned
        // HINSTANCE is inspected (>32 means the handler was launched).
        let launched = unsafe {
            ShellExecuteW(
                None,
                windows::core::w!("open"),
                PCWSTR(file.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
        // HINSTANCE values <= 32 are SE_ERR_* failure codes (2 = file not
        // found, 31 = no association); anything above means the handler
        // launched. The handle is a raw pointer wrapper, so read the value.
        let code = launched.0 as usize;
        if code <= 32 {
            return Err(HardwareError::Other(format!(
                "ShellExecuteW could not open {url} (code {code})"
            )));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        Err(HardwareError::NotSupported(
            "opening a URL is only available on Windows".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn refuses_non_http_protocols() {
        assert!(open_url(r"file://C:\Windows\System32\calc.exe").is_err());
        assert!(open_url("calc.exe").is_err());
    }
}
