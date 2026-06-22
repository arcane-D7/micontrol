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

/// Maximum valid byte index within the ACPI ERAM region.
/// The ERAM is 0x100 bytes (indices 0x00..=0xFF).
pub const ECRAM_MAX_INDEX: usize = 0xFF;

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
/// Reads ADPW (byte +0x81 of the ACPI ERAM at 0xFE0B0300) via direct IoTDriver
/// IOCTL access.  Returns `None` if the driver is unavailable (not loaded,
/// insufficient privileges) or the ADPW value is outside the plausible range
/// (1–300 W).
pub fn try_get_ac_power_mw() -> Option<i32> {
    let eram = read_ecram(ERAM_BASE, ERAM_SIZE).ok()?;
    let adpw = eram[ERAM_ADPW_OFFSET] as i32;
    if adpw > 0 && adpw <= 300 {
        Some(adpw * 1000)
    } else {
        None
    }
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

        // Validate that the driver actually returned enough data.
        check_bytes_returned(bytes_returned, byte_count)
            .context("IOCTL_ECRAM_READ returned fewer bytes than expected")?;

        // EC data starts at offset 0x10 in output (= out_buf.data[0..byte_count])
        // out_buf layout: [physical_address:8][byte_count:8][data:0x100]
        // The driver fills at out_buf+0x10 which corresponds to out_buf.data[0..byte_count]
        let ec_bytes = out_buf.data[..byte_count].to_vec();
        Ok(ec_bytes)
    }
}

/// Validate that `bytes_returned` from an IOCTL read is at least `expected_size`.
///
/// If the driver returned fewer bytes than requested, the output buffer may
/// contain stale or uninitialized data — reject rather than returning garbage.
fn check_bytes_returned(bytes_returned: u32, expected_size: usize) -> Result<()> {
    if (bytes_returned as usize) >= expected_size {
        return Ok(());
    }
    log::warn!(
        target: "hw::ecram",
        "Short read: expected {expected_size} bytes, got {bytes_returned} — rejecting",
    );
    anyhow::bail!("EC RAM short read: expected {expected_size}, got {bytes_returned}");
}

/// Validate that `index` is a valid byte offset within the ACPI ERAM region.
///
/// Returns an error if `index > ECRAM_MAX_INDEX`.
fn validate_eram_index(index: usize) -> Result<()> {
    if index <= ECRAM_MAX_INDEX {
        return Ok(());
    }
    log::warn!(
        target: "hw::ecram",
        "ERAM index 0x{index:X} exceeds maximum 0x{ECRAM_MAX_INDEX:X} — rejecting",
    );
    anyhow::bail!("ERAM index 0x{index:X} out of range (max 0x{ECRAM_MAX_INDEX:X})",)
}

#[cfg(windows)]
fn write_ecram_inner(device_path: &str, phys_addr: u64, data: &[u8]) -> Result<()> {
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

        let mut in_buf = EcramBuf {
            physical_address: phys_addr,
            byte_count: data.len() as u64,
            data: [0u8; 0x100],
        };
        in_buf.data[..data.len()].copy_from_slice(data);

        let result = DeviceIoControl(
            handle,
            IOCTL_ECRAM_WRITE,
            Some((&raw const in_buf).cast()),
            IOCTL_BUF_SIZE as u32,
            None,
            0,
            None,
            None,
        );

        CloseHandle(handle).ok();
        result.context("DeviceIoControl IOCTL_ECRAM_WRITE")?;
        Ok(())
    }
}

// ── Write allowlist (defense-in-depth) ────────────────────────────────────────
//
// The caller in `commands/hardware.rs` already gates writes behind an env var
// and an ERAM-range check.  This allowlist is a SECOND layer of defense at the
// hardware-access level: even if a future caller bypasses the command layer,
// only known-safe single-byte offsets within the ACPI ERAM region are accepted
// without the explicit raw-write override flag.
//
// These offsets correspond to harmless configuration bytes discovered via DSDT
// and SvrCModule.dll analysis:
//   0x1B — MISC flags (AILM, LBLM)
//   0x40, 0x42 — touchpad config
//   0x4A, 0x4B — Smart Mode Type/Data
//   0x68 — QFAN mode
//   0x96, 0xAE, 0xB2 — other config bytes
const SAFE_WRITE_ERAM_OFFSETS: [usize; 9] = [0x1B, 0x40, 0x42, 0x4A, 0x4B, 0x68, 0x96, 0xAE, 0xB2];

/// Environment variable that must be set to `1` to allow writes outside the
/// safe single-byte allowlist.  This mirrors the check in `commands/hardware.rs`
/// so that both layers agree on the override mechanism.
const RAW_ECRAM_WRITE_ENABLE_ENV: &str = "MICONTROL_ENABLE_RAW_ECRAM_WRITE";

/// Validate a write request against the defense-in-depth allowlist.
///
/// Returns `Ok(())` if the write is allowed, or an error describing why it was
/// rejected.  A write is allowed if EITHER:
///   1. It is a single byte to a known-safe ERAM offset, OR
///   2. The `MICONTROL_ENABLE_RAW_ECRAM_WRITE=1` env var is set AND the target
///      address falls within the ACPI ERAM region (0xFE0B0300..0xFE0B03FF).
fn validate_write(phys_addr: u64, data: &[u8]) -> Result<()> {
    // Check 1: known-safe single-byte write within ERAM
    let is_safe_single_byte =
        data.len() == 1 && phys_addr >= ERAM_BASE && phys_addr < ERAM_BASE + ERAM_SIZE as u64 && {
            let offset = (phys_addr - ERAM_BASE) as usize;
            SAFE_WRITE_ERAM_OFFSETS.contains(&offset)
        };
    if is_safe_single_byte {
        return Ok(());
    }

    // Check 2: raw-write override enabled and address is within ERAM
    let raw_enabled = std::env::var(RAW_ECRAM_WRITE_ENABLE_ENV)
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        })
        .unwrap_or(false);

    if !raw_enabled {
        anyhow::bail!(
            "ECRAM write to 0x{phys_addr:08X} ({} bytes) rejected: not in safe allowlist \
             and {RAW_ECRAM_WRITE_ENABLE_ENV}=1 is not set",
            data.len()
        );
    }

    // Even with the override, restrict to the ERAM region to prevent writes to
    // the IoT status/sensor blocks (0xFE0B0F00+) which control device state.
    let write_end = phys_addr.saturating_add(data.len() as u64);
    let in_eram = phys_addr >= ERAM_BASE && write_end <= ERAM_BASE + ERAM_SIZE as u64;
    if !in_eram {
        anyhow::bail!(
            "ECRAM write to 0x{phys_addr:08X} rejected: address outside ERAM region \
             (0x{ERAM_BASE:08X}..0x{:08X}) even with raw-write override",
            ERAM_BASE + ERAM_SIZE as u64
        );
    }

    Ok(())
}

// ── Direct ECRAM write ────────────────────────────────────────────────────────

/// Write `data` bytes into ECRAM at `phys_addr` via direct IoTDriver IOCTL.
///
/// Requires the process to be running as administrator and the IoTDriver
/// security check to pass (calling process must reside in the DriverStore
/// directory of `IoTDriver.sys`).
///
/// # Safety considerations
/// Writing to EC RAM can cause unpredictable hardware behaviour if the wrong
/// addresses or values are used.  Callers must validate inputs carefully.
pub fn write_ecram(phys_addr: u64, data: &[u8]) -> Result<()> {
    assert!(
        !data.is_empty() && data.len() <= 0x100,
        "data must be 1..=0x100 bytes (driver limit)"
    );

    // Defense-in-depth: validate the address against the allowlist before
    // touching hardware.  This is a second layer on top of the command-layer
    // checks in `commands/hardware.rs`.
    validate_write(phys_addr, data).context("ECRAM write rejected by allowlist")?;

    // Audit log: record every write for diagnostics and security review.
    let hex: String = data.iter().map(|b| format!("{b:02X}")).collect();
    log::warn!(
        target: "hw::ecram::write",
        "ECRAM WRITE: addr=0x{phys_addr:08X} len={} data=[{hex}]",
        data.len()
    );

    #[cfg(windows)]
    {
        let device_path = find_iot_device_path()
            .context("IoT driver device not found (is IoTDriver.sys loaded?)")?;
        write_ecram_inner(&device_path, phys_addr, data)
    }

    #[cfg(not(windows))]
    {
        let _ = (phys_addr, data);
        anyhow::bail!("ECRAM write is only supported on Windows")
    }
}

/// Read a named ECRAM region via direct IoTDriver IOCTL access.
///
/// Supported regions: `ERAM`, `SMA2`, `IOT_STATUS`, `IOT_SENSORS`.
pub fn read_named_region(region: &str) -> Result<Vec<u8>> {
    match region.to_ascii_uppercase().as_str() {
        "ERAM" => read_ecram(ERAM_BASE, ERAM_SIZE),
        "SMA2" => read_ecram(SMA2_BASE, SMA2_SIZE),
        "IOT_STATUS" => read_ecram(IOT_STATUS_BASE, IOT_STATUS_SIZE),
        "IOT_SENSORS" => read_ecram(ECRAM_SENSOR_BLOCK, ECRAM_SENSOR_SIZE),
        _ => anyhow::bail!(
            "Unknown ECRAM region: {region}. Supported: ERAM, SMA2, IOT_STATUS, IOT_SENSORS"
        ),
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
/// Direct read of the ACPI ERAM register map.
pub fn read_eram_map() -> Result<EramMap> {
    // Direct read only
    let eram =
        read_ecram(ERAM_BASE, 0x100).context("ECRAM read (direct IoTDriver access failed)")?;

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

    #[test]
    fn ecram_max_index_value() {
        assert_eq!(ECRAM_MAX_INDEX, 0xFF);
        assert_eq!(ECRAM_MAX_INDEX as usize, ERAM_SIZE - 1);
    }

    #[test]
    fn validate_eram_index_ok() {
        assert!(validate_eram_index(0x00).is_ok());
        assert!(validate_eram_index(0x81).is_ok());
        assert!(validate_eram_index(0xFF).is_ok());
    }

    #[test]
    fn validate_eram_index_rejects_out_of_range() {
        let err = validate_eram_index(0x100).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("0x100"),
            "error should mention the actual index: {msg}"
        );
        assert!(
            msg.contains("0xFF"),
            "error should mention the max index: {msg}"
        );
        assert!(
            err.to_string().contains("out of range"),
            "error should say 'out of range': {msg}"
        );
    }

    #[test]
    fn validate_eram_index_rejects_large() {
        assert!(validate_eram_index(0x1000).is_err());
        assert!(validate_eram_index(usize::MAX).is_err());
    }

    #[test]
    fn check_bytes_returned_ok() {
        // Exact match
        assert!(check_bytes_returned(256, 256).is_ok());
        // More returned than expected (still valid — buffer is large enough)
        assert!(check_bytes_returned(272, 256).is_ok());
    }

    #[test]
    fn check_bytes_returned_short_read_fails() {
        let err = check_bytes_returned(128, 256).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("128"),
            "error should mention actual bytes: {msg}"
        );
        assert!(
            msg.contains("256"),
            "error should mention expected bytes: {msg}"
        );
        assert!(
            msg.contains("short read") || msg.contains("Short read"),
            "error should mention short read: {msg}"
        );
    }

    #[test]
    fn check_bytes_returned_zero_bytes() {
        // Zero bytes returned when some were expected
        assert!(check_bytes_returned(0, 1).is_err());
        // Zero bytes expected, zero returned (edge case — should pass)
        assert!(check_bytes_returned(0, 0).is_ok());
    }
}
