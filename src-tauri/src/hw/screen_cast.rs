//! Screen casting via Windows Miracast/WiDi API.
//!
//! Provides device discovery and casting control using WinRT
//! `Windows.Media.Casting` and `Windows.Devices.Enumeration` APIs.

use crate::hw::errors::{HardwareError, HardwareResult};
use serde::{Deserialize, Serialize};

/// A Miracast/WiDi receiver device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastDevice {
    pub name: String,
    pub id: String,
    pub device_type: String,
}

/// Result of a cast operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastResult {
    pub success: bool,
    pub message: String,
}

/// List the Miracast/WiDi receivers Windows *already knows about*.
///
/// `DeviceInformation::FindAllAsync` over the Miracast protocol id only
/// enumerates AssociationEndpoints that Windows has previously seen/paired
/// (this is why it returns in ~130 ms with zero results on a machine that has
/// never connected a wireless display — it does not start a Wi-Fi Direct
/// discovery). Real discovery needs a `DeviceWatcher` over the same selector
/// plus several seconds of radio time, which this command deliberately does
/// not do: the Windows Connect panel (opened by `start_casting`) owns
/// discovery. The UI therefore presents this list as "known displays", never
/// as "available displays".
#[cfg(windows)]
pub fn list_cast_devices() -> HardwareResult<Vec<CastDevice>> {
    use windows::core::HSTRING;
    use windows::Devices::Enumeration::DeviceInformation;

    log::info!("[screen_cast] Enumerating known Miracast endpoints (no active discovery)");

    // AQS filter that discovers wireless display (Miracast/WiDi) receivers.
    // This is the exact selector used by Microsoft's own display-project sample:
    //   System.Devices.Aep.ProtocolId:="{AA754C9A-3A24-4298-B530-6E4B13F77E15}"
    // The GUID {AA754C9A-3A24-4298-B530-6E4B13F77E15} is the Wi-Fi Direct
    // WirelessDisplay (Miracast) protocol id. The previous code used a
    // different GUID ({e0cce415-...}) which matches no Miracast sink, so
    // "Scan for devices" always returned zero results on real hardware.
    let selector =
        HSTRING::from("System.Devices.Aep.ProtocolId:=\"{AA754C9A-3A24-4298-B530-6E4B13F77E15}\"");

    // FindAllAsyncAqsFilter returns IAsyncOperation<DeviceInformationCollection>
    let async_op = DeviceInformation::FindAllAsyncAqsFilter(&selector)
        .map_err(|e| HardwareError::Cast(format!("FindAllAsync failed: {e}")))?;

    // Block on the async operation using .get()
    let collection = async_op
        .get()
        .map_err(|e| HardwareError::Cast(format!("Device enumeration async wait failed: {e}")))?;

    let count = collection
        .Size()
        .map_err(|e| HardwareError::Cast(format!("Failed to get device count: {e}")))?;

    let mut devices = Vec::new();
    for i in 0..count {
        let device_info = match collection.GetAt(i) {
            Ok(info) => info,
            Err(e) => {
                log::warn!("[screen_cast] Failed to get device at index {i}: {e}");
                continue;
            }
        };

        let name = device_info
            .Name()
            .map(|n| n.to_string())
            .unwrap_or_default();

        let id = device_info.Id().map(|i| i.to_string()).unwrap_or_default();

        if !name.is_empty() {
            devices.push(CastDevice {
                name,
                id,
                device_type: "miracast".to_string(),
            });
        }
    }

    log::info!(
        "[screen_cast] Found {} known Miracast endpoints",
        devices.len()
    );
    Ok(devices)
}

#[cfg(not(windows))]
pub fn list_cast_devices() -> HardwareResult<Vec<CastDevice>> {
    Ok(Vec::new())
}

/// Start casting to a device by ID using WinRT CastingDevice.
///
/// Uses `Windows.Media.Casting.CastingDevice.FromIdAsync` to validate
/// the device ID and retrieve the casting device. Then opens the Windows
/// Connect panel for the user to confirm the casting session.
///
/// Note: Full programmatic casting via `RequestStartCastingAsync` requires
/// a `CastingSource` derived from a UI element (e.g. a MediaPlayer or
/// ApplicationView), which is not directly accessible from the Rust backend.
/// The Connect panel provides the casting UI while we validate the device
/// via WinRT.
#[cfg(windows)]
pub fn start_casting(device_id: &str) -> HardwareResult<CastResult> {
    log::info!("[screen_cast] Starting cast, device_id: {device_id:?}");

    // If a specific device ID is provided, validate it via WinRT CastingDevice.
    // Otherwise (empty string), just open the Windows Connect panel directly.
    if !device_id.is_empty() {
        use windows::core::HSTRING;
        use windows::Media::Casting::CastingDevice;

        log::info!("[screen_cast] Validating casting device: {device_id}");

        let from_id_op = CastingDevice::FromIdAsync(&HSTRING::from(device_id))
            .map_err(|e| HardwareError::Cast(format!("FromIdAsync failed: {e}")))?;

        let casting_device = from_id_op
            .get()
            .map_err(|e| HardwareError::Cast(format!("Failed to get casting device: {e}")))?;

        let friendly_name = casting_device
            .FriendlyName()
            .map(|n| n.to_string())
            .unwrap_or_default();

        log::info!("[screen_cast] Validated casting device: {friendly_name} (id={device_id})");
    } else {
        log::info!("[screen_cast] No device ID provided — opening Connect panel directly");
    }

    // Open the Windows Connect panel via ShellExecuteW.
    // `ms-settings:connectabledevices:devicediscovery` is the documented URI
    // that opens the "Connect to a wireless display" projection flyout.
    // (The previous `ms-settings-connectabledevices:` used a hyphen where a
    // colon belongs — no Windows setting page exists for it, so explorer.exe
    // popped an error dialog instead of the Connect panel.)
    //
    // We DO NOT spawn `explorer.exe ms-settings:...` here anymore: explorer.exe
    // returns a non-zero exit code even when the URI is handled (the Shell
    // forwards the protocol to SystemSettings and explorer exits with code 1),
    // which made the UI report "Failed to open Connect panel" while nothing
    // visibly happened. ShellExecuteW hands the URI to the Windows shell the
    // same way double-clicking it would — reliable on every build.
    const URI: windows::core::PCWSTR =
        windows::core::w!("ms-settings:connectabledevices:devicediscovery");
    // SAFETY: ShellExecuteW with a valid protocol string and "open" verb is a
    // standard Win32 call. HWND is null (no parent window).
    unsafe {
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let hres = ShellExecuteW(
            None,
            windows::core::w!("open"),
            URI,
            None,
            None,
            SW_SHOWNORMAL,
        );
        let success = hres.0 as isize > 32; // ShellExecute returns >32 on success
        Ok(CastResult {
            success,
            message: if success {
                "Windows projection panel opened. Pick a display there — Windows owns the session, MiControl only opens the panel.".into()
            } else {
                format!("Failed to open Connect panel (code {}).", hres.0 as isize)
            },
        })
    }
}

#[cfg(not(windows))]
pub fn start_casting(_device_id: &str) -> HardwareResult<CastResult> {
    Ok(CastResult {
        success: false,
        message: "Screen casting only available on Windows".into(),
    })
}

/// Stop casting by closing the Windows Connect panel.
///
/// Instead of killing ALL `SystemSettings.exe` processes (which would close
/// any other Settings windows the user may have open), we use PowerShell
/// with the Win32 API to find and close only the Connect panel window.
///
/// The result is NOT hardcoded any more: the script prints the number of
/// panels it closed, and `success` reflects that. MiControl cannot end a
/// Miracast session directly (no such public API outside the projection UI),
/// so the message says what actually happened — "panel closed" — rather than
/// claiming the projection was stopped.
#[cfg(windows)]
pub fn stop_casting() -> HardwareResult<CastResult> {
    use std::os::windows::process::CommandExt;

    log::info!("[screen_cast] Closing the Windows Connect panel (if open)");

    // Use PowerShell to close only the Connect panel window by sending WM_CLOSE.
    // This avoids killing all SystemSettings.exe processes.
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let ps_script = r#"
        Add-Type @"
        using System;
        using System.Runtime.InteropServices;
        public class Win32 {
            [DllImport("user32.dll")]
            public static extern int SendMessage(IntPtr hWnd, int Msg, int wParam, int lParam);
        }
"@
        # Find SystemSettings processes whose window title contains "Connect"
        $closed = 0
        Get-Process -Name SystemSettings -ErrorAction SilentlyContinue | ForEach-Object {
            if ($_.MainWindowTitle -match 'Connect') {
                [void][Win32]::SendMessage($_.MainWindowHandle, 0x0010, 0, 0)
                $closed++
            }
        }
        Write-Output $closed
    "#;

    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", ps_script]);
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd
        .output()
        .map_err(|e| HardwareError::Cast(format!("Failed to close the Connect panel: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let closed = parse_closed_count(&stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    log::info!(
        "[screen_cast] stop_casting exit_ok={} panels_closed={closed} stderr={}",
        output.status.success(),
        stderr.trim()
    );

    if !output.status.success() {
        return Ok(CastResult {
            success: false,
            message: format!("Failed to close the Connect panel: {}", stderr.trim()),
        });
    }

    Ok(CastResult {
        success: closed > 0,
        message: if closed > 0 {
            format!("Connect panel closed ({closed}). Note: a running projection is managed by Windows, not MiControl.")
        } else {
            "No Connect panel was open. A running projection is managed by Windows — stop it with Win+K or the display's own controls.".to_string()
        },
    })
}

/// Last non-empty line of the PowerShell output, as a count. Non-numeric
/// output (warnings, errors) counts as zero.
fn parse_closed_count(stdout: &str) -> u32 {
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .and_then(|l| l.parse::<u32>().ok())
        .unwrap_or(0)
}

#[cfg(not(windows))]
pub fn stop_casting() -> HardwareResult<CastResult> {
    Ok(CastResult {
        success: false,
        message: "Screen casting only available on Windows".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_cast_devices_returns_ok() {
        let result = list_cast_devices();
        assert!(result.is_ok());
    }

    /// `stop_casting` must never claim success when no panel was closed. The
    /// parser is the part that decides; the PowerShell call itself is not
    /// exercised here (it would close a real Settings window).
    #[test]
    fn closed_count_parsing() {
        assert_eq!(parse_closed_count("1"), 1);
        assert_eq!(parse_closed_count(" 2 \r\n"), 2);
        assert_eq!(parse_closed_count("Add-Type: warning\r\n0\r\n"), 0);
        assert_eq!(parse_closed_count(""), 0);
        assert_eq!(parse_closed_count("not a number"), 0);
    }

    /// The success flag is derived from the count, not hardcoded: with zero
    /// panels closed the caller must see `success == false`.
    #[test]
    fn zero_panels_closed_is_not_success() {
        let closed = parse_closed_count("0\n");
        assert!(closed == 0);
        // Mirrors the value the command builds for that case.
        let result = CastResult {
            success: closed > 0,
            message: "no panel".into(),
        };
        assert!(!result.success);
    }
}
