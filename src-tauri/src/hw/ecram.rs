/// ECRAM (Embedded Controller RAM) reader via IoTDriver.sys
///
/// IoTDriver.sys exposes physical ECRAM memory via two IOCTL codes:
///   0x22E000 — READ:  input {phys_addr:u64, byte_count:u64, zeros[0x100]}
///                     output {zeros[0x10], data[byte_count], zeros[...]}
///   0x22E004 — WRITE: same layout, driver writes data into EC RAM
///
/// The driver device is enumerated with GUID {AB7924A1-3162-4010-B33B-837E87E25FBC}.
/// Access requires both an elevated process (SeTokenIsAdmin) AND the calling
/// process to be named "IoTService.exe" located in the DriverStore path.
///
/// Known physical memory regions (discovered via DSDT + IoTService.exe RE):
///   0xFE0B0300 [0x100 bytes] — ACPI ERAM (SystemMemory OperationRegion)
///     Field map (subset of 219 total fields):
///       +0x80 bit0: ACIN  — AC adapter connected bit
///       +0x81:      ADPW  — AC adapter wattage (1 byte, in whole Watts, e.g. 65)
///       +0x8C:      BTCT  — Battery current (u16 LE, mA)
///       +0x8E:      BTPR  — Battery remaining capacity (u16 LE, mAh)
///       +0x90:      BTVT  — Battery voltage (u16 LE, mV)
///   0xFE0B0A00 [0x100 bytes] — ACPI SMA2 region (fields not decoded)
///   0xFE0B0F00 [8 bytes]     — IoTDriver status block (IoT device flags)
///   0xFE0B0F08 [0x78 bytes]  — IoTDevice state block (WiFi/bind status, NOT power)
///
/// AC adapter wattage (ADPW) is at physical address 0xFE0B0381 (ERAM + 0x81).
/// Reading requires satisfying the IoTDriver security check (process name = IoTService.exe).
use anyhow::{Context, Result};

/// Physical base address of the ACPI ERAM region (SystemMemory at 0xFE0B0300, size 0x100).
pub const ERAM_BASE: u64 = 0xFE0B0300;
/// Size of the ACPI ERAM region.
pub const ERAM_SIZE: usize = 0x100;
/// Byte offset within ERAM of the ADPW field (AC adapter wattage, 1 byte, in whole Watts).
pub const ERAM_ADPW_OFFSET: usize = 0x81;
/// Physical address of ADPW: ERAM_BASE + ERAM_ADPW_OFFSET = 0xFE0B0381.
#[allow(dead_code)]
pub const ADPW_ADDR: u64 = ERAM_BASE + ERAM_ADPW_OFFSET as u64;

/// Physical base of the IoTDevice state block (WiFi/bind status — not power data).
#[allow(dead_code)]
pub const ECRAM_BASE: u64 = 0xFE0B0F00;
/// Physical base of the ACPI SMA2 region (charger / EC sideband area, meaning not decoded yet).
pub const SMA2_BASE: u64 = 0xFE0B0A00;
/// Size of the ACPI SMA2 region.
pub const SMA2_SIZE: usize = 0x100;
/// Physical base of the 8-byte IoTDriver status block.
pub const IOT_STATUS_BASE: u64 = 0xFE0B0F00;
/// Size of the 8-byte IoTDriver status block.
pub const IOT_STATUS_SIZE: usize = 0x08;
/// Physical address of the 0x78-byte IoTDevice state block.
pub const ECRAM_SENSOR_BLOCK: u64 = 0xFE0B0F08;
/// Size of the IoTDevice state block.
pub const ECRAM_SENSOR_SIZE: usize = 0x78;
/// IOCTL code for ECRAM read.
const IOCTL_ECRAM_READ: u32 = 0x22E000;
/// IOCTL code for ECRAM write.
#[allow(dead_code)]
const IOCTL_ECRAM_WRITE: u32 = 0x22E004;
/// IoT driver device interface GUID: {AB7924A1-3162-4010-B33B-837E87E25FBC}
#[cfg(windows)]
const IOT_GUID: windows::core::GUID = windows::core::GUID {
    data1: 0xAB7924A1,
    data2: 0x3162,
    data3: 0x4010,
    data4: [0xB3, 0x3B, 0x83, 0x7E, 0x87, 0xE2, 0x5F, 0xBC],
};

/// Total IOCTL buffer size (driver requires exactly 0x110 bytes for both in and out).
const IOCTL_BUF_SIZE: usize = 0x110;

/// IOCTL buffer layout: matches the driver's expected input/output format.
///   Bytes  0–7:   physical_address (u64 LE)
///   Bytes  8–15:  byte_count (u64 LE)
///   Bytes 16–271: on input: zeros (padding); on output: EC data starting at byte 16
#[repr(C)]
struct EcramBuf {
    physical_address: u64,
    byte_count: u64,
    data: [u8; 0x100],
}

const _: () = {
    assert!(std::mem::size_of::<EcramBuf>() == IOCTL_BUF_SIZE);
};

/// Read `byte_count` bytes from ECRAM at `phys_addr`.
///
/// Returns a `Vec<u8>` of length `byte_count` on success.
/// Requires the process to be running as administrator.
///
/// # Errors
/// Returns an error if the device cannot be opened (driver not loaded,
/// insufficient privileges) or if the IOCTL fails.
pub fn read_ecram(phys_addr: u64, byte_count: usize) -> Result<Vec<u8>> {
    assert!(
        byte_count <= 0x100,
        "byte_count must be ≤ 0x100 (driver limit)"
    );

    #[cfg(windows)]
    {
        let device_path = find_iot_device_path()
            .context("IoT driver device not found (is IoTDriver.sys loaded?)")?;
        read_ecram_inner(&device_path, phys_addr, byte_count)
    }

    #[cfg(not(windows))]
    {
        let _ = (phys_addr, byte_count);
        anyhow::bail!("ECRAM read is only supported on Windows")
    }
}

/// Convenience: read the full 256-byte ACPI ERAM block (contains ADPW, BTCT, BTVT, etc.).
#[allow(dead_code)]
pub fn read_eram() -> Result<Vec<u8>> {
    read_ecram(ERAM_BASE, ERAM_SIZE)
}

/// Convenience: read the full 256-byte ACPI SMA2 block.
#[allow(dead_code)]
pub fn read_sma2() -> Result<Vec<u8>> {
    read_ecram(SMA2_BASE, SMA2_SIZE)
}

/// Convenience: read the 8-byte IoT status block.
#[allow(dead_code)]
pub fn read_iot_status_block() -> Result<Vec<u8>> {
    read_ecram(IOT_STATUS_BASE, IOT_STATUS_SIZE)
}

/// Convenience: read the 0x78-byte IoTDevice state block at 0xFE0B0F08 (WiFi/bind status).
#[allow(dead_code)]
pub fn read_sensor_block() -> Result<Vec<u8>> {
    read_ecram(ECRAM_SENSOR_BLOCK, ECRAM_SENSOR_SIZE)
}

/// Read all ECRAM bytes available from IoTService's known ranges.
/// Returns ERAM (0xFE0B0300, 256 bytes) followed by IoTDevice block (0xFE0B0F08, 0x78 bytes).
#[allow(dead_code)]
pub fn read_all() -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(ERAM_SIZE + ECRAM_SENSOR_SIZE);
    buf.extend_from_slice(&read_ecram(ERAM_BASE, ERAM_SIZE)?);
    buf.extend_from_slice(&read_ecram(ECRAM_SENSOR_BLOCK, ECRAM_SENSOR_SIZE)?);
    Ok(buf)
}

/// Try to extract AC adapter input power (in milliwatts) from ECRAM.
///
/// Tries direct IoTDriver access first, then falls back to the DriverStore
/// shim (`ecram_shim.exe`) which satisfies the driver's path-prefix check.
///
/// Returns `None` if both paths fail or the ADPW value is out of range.
pub fn try_get_ac_power_mw() -> Option<i32> {
    // Attempt 1: direct read (works only when our process is in the DriverStore dir)
    if let Ok(eram) = read_ecram(ERAM_BASE, 0x100) {
        let adpw = eram[ERAM_ADPW_OFFSET] as i32;
        if adpw > 0 && adpw <= 300 {
            return Some(adpw * 1000);
        }
    }

    // Attempt 2: shim (deployed to the DriverStore dir, bypasses path check)
    #[cfg(windows)]
    match read_ecram_via_shim(ERAM_BASE, 0x100) {
        Ok(eram) => {
            let adpw = eram[ERAM_ADPW_OFFSET] as i32;
            if adpw > 0 && adpw <= 300 {
                return Some(adpw * 1000);
            }
        }
        Err(e) => log::warn!("[ecram] ADPW shim read failed: {e:#}"),
    }

    None
}

/// Return a hex dump string of all known ECRAM bytes for debugging.
/// Dumps both the ACPI ERAM (0xFE0B0300, 256 bytes, includes ADPW at +0x81)
/// and the IoTDevice state block (0xFE0B0F08, 0x78 bytes).
/// Format: "0xADDR: XX XX XX XX ..."
pub fn debug_ecram_hex() -> Result<String> {
    let mut out = String::new();

    out.push_str("=== ACPI ERAM (0xFE0B0300, 256 bytes) ===\n");
    match read_ecram(ERAM_BASE, 0x100) {
        Ok(eram) => {
            for (i, chunk) in eram.chunks(16).enumerate() {
                let addr = ERAM_BASE + (i * 16) as u64;
                let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02X}")).collect();
                out.push_str(&format!("0x{addr:08X}: {}\n", hex.join(" ")));
            }
            out.push_str(&format!(
                "ADPW (AC wattage) @ +0x81 = 0x{:02X} = {} W\n",
                eram[ERAM_ADPW_OFFSET], eram[ERAM_ADPW_OFFSET]
            ));
        }
        Err(e) => out.push_str(&format!("Error reading ERAM: {e}\n")),
    }

    out.push_str("\n=== IoTDevice state block (0xFE0B0F08, 0x78 bytes) ===\n");
    match read_ecram(ECRAM_SENSOR_BLOCK, ECRAM_SENSOR_SIZE) {
        Ok(blk) => {
            for (i, chunk) in blk.chunks(16).enumerate() {
                let addr = ECRAM_SENSOR_BLOCK + (i * 16) as u64;
                let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02X}")).collect();
                out.push_str(&format!("0x{addr:08X}: {}\n", hex.join(" ")));
            }
        }
        Err(e) => out.push_str(&format!("Error reading IoTDevice block: {e}\n")),
    }

    Ok(out)
}

// ── Windows implementation ────────────────────────────────────────────────────

#[cfg(windows)]
fn find_iot_device_path() -> Result<String> {
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
        SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
        SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
    };

    unsafe {
        let dev_info = SetupDiGetClassDevsW(
            Some(&IOT_GUID),
            None,
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
        .context("SetupDiGetClassDevsW for IoT GUID")?;

        let mut iface = SP_DEVICE_INTERFACE_DATA {
            cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..std::mem::zeroed()
        };

        let enum_result = SetupDiEnumDeviceInterfaces(dev_info, None, &IOT_GUID, 0, &mut iface);
        if enum_result.is_err() {
            let _ = SetupDiDestroyDeviceInfoList(dev_info);
            anyhow::bail!("No IoT device interface found (GUID {{AB7924A1-...}})");
        }

        // First call: get required buffer size
        let mut required = 0u32;
        let _ =
            SetupDiGetDeviceInterfaceDetailW(dev_info, &iface, None, 0, Some(&mut required), None);

        if required == 0 || required > 4096 {
            let _ = SetupDiDestroyDeviceInfoList(dev_info);
            anyhow::bail!("Invalid required size {required} for IoT device path");
        }

        // Second call: get the device path
        let mut buf = vec![0u8; required as usize];
        let detail_ptr = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
        (*detail_ptr).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;

        let detail_result = SetupDiGetDeviceInterfaceDetailW(
            dev_info,
            &iface,
            Some(detail_ptr),
            required,
            None,
            None,
        );
        let _ = SetupDiDestroyDeviceInfoList(dev_info);
        detail_result.context("SetupDiGetDeviceInterfaceDetailW")?;

        // Parse the UTF-16 device path (starts at offset 4, after the cbSize u32)
        let path_offset = 4usize;
        let wide_slice = std::slice::from_raw_parts(
            buf.as_ptr().add(path_offset) as *const u16,
            (required as usize - path_offset) / 2,
        );
        let null_pos = wide_slice
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(wide_slice.len());
        let path =
            String::from_utf16(&wide_slice[..null_pos]).context("Invalid UTF-16 device path")?;

        Ok(path)
    }
}

#[cfg(windows)]
fn read_ecram_inner(device_path: &str, phys_addr: u64, byte_count: usize) -> Result<Vec<u8>> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE},
            Storage::FileSystem::{
                CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
                OPEN_EXISTING,
            },
            System::IO::DeviceIoControl,
        },
    };

    let path_w: Vec<u16> = OsStr::new(device_path)
        .encode_wide()
        .chain(Some(0))
        .collect();

    unsafe {
        let handle = CreateFileW(
            PCWSTR(path_w.as_ptr()),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            HANDLE::default(),
        )
        .context("Open IoT driver device")?;

        if handle == INVALID_HANDLE_VALUE {
            anyhow::bail!("INVALID_HANDLE_VALUE opening IoT driver device");
        }

        // Build input buffer
        let in_buf = EcramBuf {
            physical_address: phys_addr,
            byte_count: byte_count as u64,
            data: [0u8; 0x100],
        };

        // Output buffer (driver writes EC data at byte offset 0x10)
        let mut out_buf = EcramBuf {
            physical_address: 0,
            byte_count: 0,
            data: [0u8; 0x100],
        };

        let mut bytes_returned = 0u32;
        let result = DeviceIoControl(
            handle,
            IOCTL_ECRAM_READ,
            Some((&raw const in_buf).cast()),
            IOCTL_BUF_SIZE as u32,
            Some((&raw mut out_buf).cast()),
            IOCTL_BUF_SIZE as u32,
            Some(&mut bytes_returned),
            None,
        );

        CloseHandle(handle).ok();
        result.context("DeviceIoControl IOCTL_ECRAM_READ")?;

        // EC data starts at offset 0x10 in output (= out_buf.data[0..byte_count])
        // out_buf layout: [physical_address:8][byte_count:8][data:0x100]
        // The driver fills at out_buf+0x10 which corresponds to out_buf.data[0..byte_count]
        let ec_bytes = out_buf.data[..byte_count].to_vec();
        Ok(ec_bytes)
    }
}

// ── Shim-based ECRAM access (bypasses IoTDriver path check) ──────────────────
//
// IoTDriver.sys calls `SeLocateProcessImageName` + `RtlPrefixUnicodeString` to
// verify that the calling process's directory starts with the DriverStore prefix
// of IoTDriver.sys itself.  We bypass this by deploying a small helper binary
// (`ecram_shim.exe`) INTO the DriverStore directory so it passes the check.
//
// Deployment: uses `SeRestorePrivilege` + `FILE_FLAG_BACKUP_SEMANTICS` to copy
// the shim to a directory owned by TrustedInstaller.
// Invocation: spawns the shim as a child process, reads JSON from its stdout.

/// Find the DriverStore directory that contains `IoTDriver.sys` by reading
/// `HKLM\SYSTEM\CurrentControlSet\Services\IoTDriver\ImagePath`.
///
/// Returns e.g. `C:\Windows\System32\DriverStore\FileRepository\miiotdrv.inf_amd64_XXX\`
#[cfg(windows)]
pub fn find_iotdriver_store_dir() -> Result<std::path::PathBuf> {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let svc = hklm
        .open_subkey("SYSTEM\\CurrentControlSet\\Services\\IoTDriver")
        .context("IoTDriver service registry key not found — is the driver installed?")?;
    let image_path: String = svc
        .get_value("ImagePath")
        .context("IoTDriver ImagePath value missing")?;

    // Normalise `\SystemRoot\` → actual Windows directory
    let windows_dir = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    let normalised = if image_path
        .to_ascii_lowercase()
        .starts_with("\\systemroot\\")
    {
        format!("{}{}", windows_dir, &image_path["\\SystemRoot".len()..])
    } else if image_path.starts_with("\\??\\") {
        image_path[4..].to_string()
    } else {
        image_path.clone()
    };

    let path = std::path::PathBuf::from(normalised);
    path.parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("Cannot derive DriverStore dir from: {image_path}"))
}

/// Deploy the helper executable to the IoTDriver DriverStore directory.
///
/// Uses `SeRestorePrivilege` + `FILE_FLAG_BACKUP_SEMANTICS` to write into the
/// TrustedInstaller-owned directory.  No-ops if the deployed file is already
/// current (same size as the source).
///
/// Returns the absolute path to the deployed shim.
#[cfg(windows)]
pub fn deploy_ecram_shim(dest_file_name: &str) -> Result<std::path::PathBuf> {
    // Locate source shim (same directory as the running executable)
    let exe = std::env::current_exe().context("current_exe")?;
    let shim_src = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Cannot find exe directory"))?
        .join("ecram_shim.exe");

    anyhow::ensure!(
        shim_src.exists(),
        "ecram_shim.exe not found at {shim_src:?} — build with `cargo build --bin ecram_shim`"
    );

    let driverstore_dir = find_iotdriver_store_dir()?;
    let dest = driverstore_dir.join(dest_file_name);

    // Skip if already up to date
    if dest.exists() {
        let src_len = std::fs::metadata(&shim_src)?.len();
        if let Ok(dst_meta) = std::fs::metadata(&dest) {
            if dst_meta.len() == src_len {
                log::debug!("[ecram_shim] Already deployed at {dest:?}");
                return Ok(dest);
            }
        }
    }

    log::info!("[ecram_shim] Deploying {dest_file_name} to {dest:?}");
    enable_restore_privilege().context("enable SeRestorePrivilege")?;
    copy_with_backup_semantics(&shim_src, &dest)?;
    log::info!("[ecram_shim] Deployed successfully");
    Ok(dest)
}

/// Read `byte_count` bytes of ECRAM at `phys_addr` via the deployed shim.
///
/// Deploys the shim on first call (or when it changes), then spawns it as a
/// child process and parses its JSON stdout.
pub fn read_ecram_via_shim(phys_addr: u64, byte_count: usize) -> Result<Vec<u8>> {
    #[cfg(windows)]
    {
        let v = run_ecram_shim([
            "read".to_string(),
            format!("{phys_addr:#010x}"),
            format!("{byte_count}"),
        ])?;
        decode_shim_hex_payload(&v)
    }
    #[cfg(not(windows))]
    {
        let _ = (phys_addr, byte_count);
        anyhow::bail!("read_ecram_via_shim is only supported on Windows")
    }
}

/// Read a named IoT region through the deployed shim.
///
/// Supported regions: `ERAM`, `SMA2`, `IOT_STATUS`, `IOT_SENSORS`.
#[allow(dead_code)]
pub fn read_named_region_via_shim(region: &str) -> Result<Vec<u8>> {
    #[cfg(windows)]
    {
        let v = run_ecram_shim(["read-region".to_string(), region.to_string()])?;
        decode_shim_hex_payload(&v)
    }
    #[cfg(not(windows))]
    {
        let _ = region;
        anyhow::bail!("read_named_region_via_shim is only supported on Windows")
    }
}

/// Write bytes into EC RAM through the deployed shim.
#[allow(dead_code)]
pub fn write_ecram_via_shim(phys_addr: u64, data: &[u8]) -> Result<()> {
    #[cfg(windows)]
    {
        anyhow::ensure!(
            !data.is_empty() && data.len() <= 0x100,
            "write_ecram_via_shim expects 1..256 bytes"
        );

        let hex_data: String = data.iter().map(|b| format!("{b:02x}")).collect();
        let _ = run_ecram_shim(["write".to_string(), format!("{phys_addr:#010x}"), hex_data])?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (phys_addr, data);
        anyhow::bail!("write_ecram_via_shim is only supported on Windows")
    }
}

/// Returns `true` when the current process token is elevated (administrator).
#[cfg(windows)]
pub fn is_process_elevated() -> bool {
    use windows::Win32::UI::Shell::IsUserAnAdmin;
    unsafe { IsUserAnAdmin().as_bool() }
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn is_process_elevated() -> bool {
    false
}

/// Spawn `ecram_shim.exe` with no console window and capture its stdout.
///
/// Requires the calling process to be elevated (administrator).  IoTDriver.sys
/// enforces both the DriverStore path check (satisfied by deploying the shim)
/// AND requires `SeTokenIsAdmin` on the calling token.  If the process is not
/// elevated this returns an actionable error rather than spawning a subprocess
/// that would silently fail with `ERROR_ACCESS_DENIED`.
#[cfg(windows)]
fn run_ecram_shim(args: impl IntoIterator<Item = String>) -> Result<serde_json::Value> {
    use std::os::windows::process::CommandExt;
    /// Suppress the console window that would otherwise flash when spawning
    /// a Windows console subsystem binary from a GUI (windowless) process.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let args_vec: Vec<String> = args.into_iter().collect();

    if !is_process_elevated() {
        anyhow::bail!(
            "IoT module requires administrator privileges.\n\
             Please run MiControl as administrator to enable ECRAM access."
        );
    }

    // Some firmware builds verify the process image name in addition to path.
    // Try IoTService.exe alias first, then fall back to ecram_shim.exe.
    let mut errors: Vec<String> = Vec::new();
    for helper_name in ["IoTService.exe", "ecram_shim.exe"] {
        let shim_path = match deploy_ecram_shim(helper_name)
            .with_context(|| format!("deploy {helper_name} to IoTDriver DriverStore"))
        {
            Ok(p) => p,
            Err(e) => {
                errors.push(format!("{helper_name}: {e:#}"));
                continue;
            }
        };

        let output = match std::process::Command::new(&shim_path)
            .args(args_vec.iter())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .with_context(|| format!("Failed to spawn helper at {shim_path:?}"))
        {
            Ok(o) => o,
            Err(e) => {
                errors.push(format!("{helper_name}: {e:#}"));
                continue;
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let v: serde_json::Value = match serde_json::from_str(stdout.trim())
            .with_context(|| format!("Invalid shim output: stdout={stdout} stderr={stderr}"))
        {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("{helper_name}: {e:#}"));
                continue;
            }
        };

        if !v["ok"].as_bool().unwrap_or(false) {
            let err = v["error"].as_str().unwrap_or("unknown shim error");
            errors.push(format!("{helper_name}: {err}"));
            continue;
        }

        return Ok(v);
    }

    if errors.is_empty() {
        anyhow::bail!("No shim helper variant succeeded");
    }
    anyhow::bail!("All helper variants failed: {}", errors.join(" | "))
}

#[cfg(windows)]
fn decode_shim_hex_payload(v: &serde_json::Value) -> Result<Vec<u8>> {
    let hex_str = v["data"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing data field in shim output"))?;

    (0..hex_str.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex_str[i..i + 2], 16).map_err(Into::into))
        .collect::<Result<Vec<u8>>>()
        .context("Hex decode of shim output")
}

/// Enable `SeRestorePrivilege` on the current process token so that
/// `FILE_FLAG_BACKUP_SEMANTICS` bypasses ACL checks on DriverStore writes.
#[cfg(windows)]
fn enable_restore_privilege() -> Result<()> {
    use windows::Win32::Security::{
        AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED,
        TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = windows::Win32::Foundation::HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES, &mut token)
            .context("OpenProcessToken")?;

        let priv_name: Vec<u16> = "SeRestorePrivilege\0".encode_utf16().collect();
        let mut luid = windows::Win32::Foundation::LUID::default();
        LookupPrivilegeValueW(None, windows::core::PCWSTR(priv_name.as_ptr()), &mut luid)
            .context("LookupPrivilegeValueW(SeRestorePrivilege)")?;

        let tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };

        AdjustTokenPrivileges(token, false, Some(&tp), 0, None, None)
            .context("AdjustTokenPrivileges")?;

        let _ = windows::Win32::Foundation::CloseHandle(token);
    }
    Ok(())
}

/// Copy `src` to `dst` using `FILE_FLAG_BACKUP_SEMANTICS` so that the write
/// bypasses the directory DACL when `SeRestorePrivilege` is active.
#[cfg(windows)]
fn copy_with_backup_semantics(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{CloseHandle, GENERIC_WRITE, HANDLE},
            Storage::FileSystem::{
                CreateFileW, WriteFile, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL,
                FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ,
            },
        },
    };

    let src_data = std::fs::read(src).with_context(|| format!("Read shim source {src:?}"))?;

    let dst_wide: Vec<u16> = OsStr::new(dst).encode_wide().chain(Some(0)).collect();

    unsafe {
        let handle = CreateFileW(
            PCWSTR(dst_wide.as_ptr()),
            GENERIC_WRITE.0,
            FILE_SHARE_READ,
            None,
            CREATE_ALWAYS,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_ATTRIBUTE_NORMAL,
            HANDLE::default(),
        )
        .with_context(|| format!("CreateFileW (backup semantics) on {dst:?}"))?;

        let mut written = 0u32;
        let write_result = WriteFile(handle, Some(&src_data), Some(&mut written), None);
        let _ = CloseHandle(handle);
        write_result.context("WriteFile shim to DriverStore")?;

        anyhow::ensure!(
            written as usize == src_data.len(),
            "Wrote {} of {} bytes to {dst:?}",
            written,
            src_data.len()
        );
    }
    Ok(())
}

// ── ECRAM register map ────────────────────────────────────────────────────────

/// Structured decode of all known ACPI ERAM fields.
///
/// Decoded from DSDT `ERAM` SystemMemory OperationRegion at `0xFE0B0300`
/// and `SvrCModule.dll` string analysis.  Field names match ACPI DSDT names.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct EramMap {
    // ── Byte +0x00 — Miscellaneous flags 0 ───────────────────────────────────
    /// Raw byte at offset 0x00 (MISC flags byte 0)
    pub misc0: u8,
    // ── Byte +0x01 — Miscellaneous flags 1 ───────────────────────────────────
    pub misc1: u8,
    /// Raw control byte at offset 0x1B.
    pub control_flags_1b: u8,
    /// AILM flag (`0x1B bit 2`) from AML decode.
    pub ai_limit_enabled: bool,
    /// LBLM flag (`0x1B bit 3`) from AML decode.
    pub long_battery_limit_enabled: bool,

    // ── Thermal / Fan ────────────────────────────────────────────────────────
    /// CPU temperature sensor (°C, offset +0x03)
    pub cpu_temp_c: u8,
    /// Fan speed RPM (u16 LE, offset +0x04..+0x05)
    pub fan_rpm: u16,
    /// Fan 2 speed RPM (u16 LE, offset +0x06..+0x07), 0 if single-fan model
    pub fan2_rpm: u16,
    /// CPU power (Watts, 1 byte, offset +0x0A)
    pub cpu_power_w: u8,
    /// SMMT — Smart Mode Type byte (offset +0x4A)
    pub smart_mode_type: u8,
    /// SMMD — Smart Mode Data byte (offset +0x4B)
    pub smart_mode_data: u8,
    /// Derived human-readable meaning for the SMMT/SMMD pair when recognized.
    pub smart_mode_profile: Option<String>,
    /// QFAN — fan/smart profile byte (offset +0x68)
    pub qfan_mode: u8,

    // ── Performance mode ─────────────────────────────────────────────────────
    /// Performance profile byte (offset +0x40): 0x00=Balanced, 0x01=Performance, 0x02=Silent
    pub perf_profile: u8,
    /// TDP override byte (offset +0x42, Watts)
    pub tdp_w: u8,

    // ── AC / Battery ─────────────────────────────────────────────────────────
    /// Byte +0x80: bit 0 = ACIN (AC adapter present)
    pub ac_flags: u8,
    /// ACIN bit derived from ac_flags
    pub ac_connected: bool,
    /// ADPW — AC adapter rated wattage (Watts, 1 byte, offset +0x81)
    pub ac_adapter_w: u8,
    /// BTCT — Battery charge/discharge current (mA, i16 LE, offset +0x8C)
    /// Positive = charging, negative = discharging
    pub battery_current_ma: i16,
    /// BTPR — Battery remaining capacity (mAh, u16 LE, offset +0x8E)
    pub battery_capacity_mah: u16,
    /// BTVT — Battery voltage (mV, u16 LE, offset +0x90)
    pub battery_voltage_mv: u16,
    /// Charging threshold setting (%, offset +0x96)
    pub charge_threshold_pct: u8,
    /// Battery temperature (°C, 1 byte, offset +0x97)
    pub battery_temp_c: u8,
    /// DBLL — display brightness level (7-bit, offset +0xAE)
    pub display_brightness_level: u8,
    /// KBLL — keyboard backlight level (7-bit, offset +0xB2)
    pub keyboard_backlight_level: u8,

    /// Raw full 256-byte ERAM dump (hex string, for debugging unmapped fields)
    pub raw_hex: String,
}

/// Read the ACPI ERAM block and decode all known fields.
///
/// Uses the shim path automatically if the direct IoTDriver access fails.
pub fn read_eram_map() -> Result<EramMap> {
    // Try direct read, fall back to shim
    let eram = read_ecram(ERAM_BASE, 0x100)
        .or_else(|_| read_ecram_via_shim(ERAM_BASE, 0x100))
        .context("ECRAM read (both direct and shim paths failed)")?;

    anyhow::ensure!(eram.len() >= 0x100, "Short ERAM read: {} bytes", eram.len());

    let raw_hex: String = eram.iter().map(|b| format!("{b:02x}")).collect();
    let control_flags_1b = eram[0x1B];
    let smart_mode_type = eram[0x4A];
    let smart_mode_data = eram[0x4B];
    let smart_mode_profile = match (smart_mode_type, smart_mode_data) {
        (0, 5) => Some("FUN3=5 -> NTDP profile 5".to_string()),
        (0, 6) => Some("FUN3=6 -> NTDP profile 6".to_string()),
        (7, 0) => Some("FUN3=7 -> Smart Mode Type 7 (inverted)".to_string()),
        (8, 0) => Some("FUN3=8 -> Smart Mode Type 8 (inverted)".to_string()),
        (0, 0) => Some("Cleared SMMT/SMMD (FUN3=9 or FUN3=10 path)".to_string()),
        _ => None,
    };

    Ok(EramMap {
        misc0: eram[0x00],
        misc1: eram[0x01],
        control_flags_1b,
        ai_limit_enabled: (control_flags_1b & 0b0000_0100) != 0,
        long_battery_limit_enabled: (control_flags_1b & 0b0000_1000) != 0,
        cpu_temp_c: eram[0x03],
        fan_rpm: u16::from_le_bytes([eram[0x04], eram[0x05]]),
        fan2_rpm: u16::from_le_bytes([eram[0x06], eram[0x07]]),
        cpu_power_w: eram[0x0A],
        smart_mode_type,
        smart_mode_data,
        smart_mode_profile,
        qfan_mode: eram[0x68],
        perf_profile: eram[0x40],
        tdp_w: eram[0x42],
        ac_flags: eram[0x80],
        ac_connected: (eram[0x80] & 0x01) != 0,
        ac_adapter_w: eram[0x81],
        battery_current_ma: i16::from_le_bytes([eram[0x8C], eram[0x8D]]),
        battery_capacity_mah: u16::from_le_bytes([eram[0x8E], eram[0x8F]]),
        battery_voltage_mv: u16::from_le_bytes([eram[0x90], eram[0x91]]),
        charge_threshold_pct: eram[0x96],
        battery_temp_c: eram[0x97],
        display_brightness_level: eram[0xAE] & 0x7F,
        keyboard_backlight_level: eram[0xB2] & 0x7F,
        raw_hex,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecram_buf_size() {
        assert_eq!(std::mem::size_of::<EcramBuf>(), IOCTL_BUF_SIZE);
    }
}
