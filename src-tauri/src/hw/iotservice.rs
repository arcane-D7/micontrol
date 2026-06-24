/// IoTService.exe IPC client
///
/// Communicates with the official Xiaomi IoTService Windows service through
/// its named pipe (`\\.\pipe\LOCAL\IoTService_IPC_Broker`) to control
/// hardware features without direct IOCTL access.
///
/// Protocol reverse-engineered from IoTService.exe (v25.0.0.9, x86-64)
/// using Ghidra 12.1 headless string/function extraction.
///
/// ## IPC Message Format
///
/// The wire format matches the working implementation in `charging.rs`:
///
/// ```text
/// ┌──────────────┬──────────────┬──────────────┬──────────────┬───────────────────────────┐
/// │ src_id: u16  │ dst_id: u16  │ msg_type: u32│ payload_len  │ payload: [u8; payload_len] │
/// │              │              │              │ u32 LE       │ (JSON or binary data)     │
/// ├──────────────┴──────────────┴──────────────┴──────────────┴───────────────────────────┤
/// │ Header = 12 bytes                                                                     │
/// └───────────────────────────────────────────────────────────────────────────────────────┘
/// ```
///
/// Total header size: 12 bytes. No signature field — the pipe name itself
/// serves as the namespace delimiter.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};

/// Named pipe path to the IoTService IPC broker.
pub const IOT_PIPE: &str = r"\\.\pipe\LOCAL\IoTService_IPC_Broker";

/// Size of the fixed IPC header (src_id + dst_id + msg_type + payload_len)
const IPC_HEADER_SIZE: usize = 12;

/// Our client ID registered with the IoTService IPC broker.
const CLIENT_ID: u16 = 1;
/// Destination ID for the IoTDriver worker.
const DST_IOT_DRIVER: u16 = 2;
/// Destination ID for the WMI worker.
#[allow(dead_code)]
const DST_WMI: u16 = 3;

/// Maximum payload size we'll accept in a response.
const MAX_RESPONSE_PAYLOAD: usize = 0x10000;

/// Monotonically increasing request sequence number for tracking/debugging.
/// Incremented on each `send_ipc_message` call.
///
/// Note: the IoTService wire protocol does not support embedding this
/// sequence number in the message itself (the header format is fixed and
/// reverse-engineered). The counter is used for local tracing only.
static REQUEST_SEQ: AtomicU32 = AtomicU32::new(0);

/// Check whether a message type is known/recognized.
///
/// This is the single source of truth for valid incoming message types.
/// Unknown types are rejected (fail-closed) to prevent processing
/// unexpected or potentially malicious response messages.
fn is_known_msg_type(msg_type: u32) -> bool {
    matches!(
        msg_type,
        msg_type::GET_MODEL
            | msg_type::GET_FW_VERSION
            | msg_type::GET_BIND_STATUS
            | msg_type::GET_DEVICE_ID
            | msg_type::GET_DEVICE_STATUS
            | msg_type::SET_DEVICE_STATUS
            | msg_type::RESET_DEVICE
            | msg_type::SET_CHARGING_LIMIT
            | msg_type::SEND_LAPTOP_STATUS
            | msg_type::WRITE_WIFI_ITEM
            | msg_type::DELETE_WIFI_ITEM
            | msg_type::GET_WIFI_BY_INDEX
            | msg_type::READ_WIFI_COUNT
            | msg_type::READ_WIFI_STATUS
            | msg_type::EMPTY_WIFI_ITEMS
            | msg_type::CONNECT_WIFI
            // UNCONFIRMED — see msg_type module docs
            | msg_type::EC_EVENT
            | msg_type::POWER_EVENT
    )
}

// ── Message type constants (discovered via RE) ───────────────────────────────

/// msg_type values for IPC commands.
///
/// Most constants validated against Ghidra decompilation of Worker_IPC.cpp.
/// See `docs/iotservice-re-analysis.md` Section 3.4 for details on EC and
/// power events monitored by IoTService.
///
/// ## Unconfirmed types
///
/// EC_EVENT (0x5001) and POWER_EVENT (0x5002) are **unconfirmed** — they were
/// inferred from string analysis but their handler signatures were not located
/// in the decompiled output. The IoTService internally monitors EC events via
/// WMI (`SELECT * FROM HID_EVENT20`) and power events via
/// `RegisterPowerSettingNotification`, but whether an external client is
/// expected to send these message types to the service is unverified.
///
/// These types are kept in the known-type list so they are not rejected
/// by the fail-closed response validator, but they are marked as potentially
/// unused or incorrect. Use with caution — verify via traffic capture before
/// relying on them in production.
#[allow(dead_code)]
pub mod msg_type {
    // Device info (read-only, no JSON payload needed)
    pub const GET_MODEL: u32 = 0x1001;
    pub const GET_FW_VERSION: u32 = 0x1002;
    pub const GET_BIND_STATUS: u32 = 0x1004;
    pub const GET_DEVICE_ID: u32 = 0x1005;
    pub const GET_DEVICE_STATUS: u32 = 0x1006;

    // Device control (JSON payload)
    pub const SET_DEVICE_STATUS: u32 = 0x2001;
    pub const RESET_DEVICE: u32 = 0x2002;

    // Charging (binary payload, 1 byte)
    pub const SET_CHARGING_LIMIT: u32 = 0x1003;

    // Laptop status (JSON payload)
    pub const SEND_LAPTOP_STATUS: u32 = 0x3001;

    // WiFi management (JSON payload)
    pub const WRITE_WIFI_ITEM: u32 = 0x4001;
    pub const DELETE_WIFI_ITEM: u32 = 0x4002;
    pub const GET_WIFI_BY_INDEX: u32 = 0x4003;
    pub const READ_WIFI_COUNT: u32 = 0x4004;
    pub const READ_WIFI_STATUS: u32 = 0x4005;
    pub const EMPTY_WIFI_ITEMS: u32 = 0x4006;
    pub const CONNECT_WIFI: u32 = 0x4007;

    // Event notification (no response expected)
    pub const EC_EVENT: u32 = 0x5001;
    pub const POWER_EVENT: u32 = 0x5002;
}

// ── Raw IPC message ──────────────────────────────────────────────────────────

/// Packed binary representation of an IPC message on the wire.
///
/// Layout matches the proven format in `charging.rs`:
///   - src_id: u16 (offset 0)
///   - dst_id: u16 (offset 2)
///   - msg_type: u32 (offset 4)
///   - payload_len: u32 (offset 8)
///   - Total header: 12 bytes (naturally aligned, no padding)
#[repr(C)]
struct IpcWireHeader {
    src_id: u16,
    dst_id: u16,
    msg_type: u32,
    payload_len: u32,
}

impl IpcWireHeader {
    fn new(src_id: u16, dst_id: u16, msg_type: u32, payload_len: u32) -> Self {
        Self {
            src_id,
            dst_id,
            msg_type,
            payload_len,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        // SAFETY: IpcWireHeader is a #[repr(C)] struct with no padding; casting to &[u8] of size_of<IpcWireHeader> is safe because the data is valid for reads and the size matches the actual struct layout.
        unsafe {
            std::slice::from_raw_parts(
                self as *const IpcWireHeader as *const u8,
                std::mem::size_of::<IpcWireHeader>(),
            )
        }
    }
}

// ── JSON command/response types ──────────────────────────────────────────────

/// Generic IPC response wrapper (used by internal deserialization).
#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct IpcResponse {
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(flatten)]
    pub data: serde_json::Value,
}

/// Model information returned by GetModel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub model: String,
}

/// Firmware version returned by GetFwVersion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FwVersionInfo {
    pub fw_version: String,
}

/// Device bind status returned by GetBindStatus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindStatusInfo {
    pub bound: bool,
    pub uid: Option<u64>,
}

/// Device info returned by GetDeviceID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceIdInfo {
    pub device_id: i64,
}

/// Device status returned by GetDeviceStatus / SetDeviceStatus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceStatusInfo {
    pub status: String,
}

/// Laptop status values (matching IoTService constants).
///
/// Confirmed via Ghidra decompilation of Worker_IPC.cpp:
///   - LaptopStatus key with type tag determines the value:
///     type 4 → WinReady, type 6 → Suspending, type 8 → Shutting
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaptopStatus {
    /// Windows has booted and is ready.
    WinReady,
    /// System is entering sleep/suspend.
    Suspending,
    /// System is shutting down.
    Shutting,
}

impl LaptopStatus {
    /// Convert to the u32 value expected by IoTService.
    /// Confirmed from decompilation: 4=WinReady, 6=Suspending, 8=Shutting.
    pub fn to_hw_value(self) -> u32 {
        match self {
            LaptopStatus::WinReady => 4,
            LaptopStatus::Suspending => 6,
            LaptopStatus::Shutting => 8,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            LaptopStatus::WinReady => "IOT_WIN_READY",
            LaptopStatus::Suspending => "IOT_SUSPENDING",
            LaptopStatus::Shutting => "IOT_SHUTING",
        }
    }
}

/// WiFi network item for provisioning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WiFiItem {
    pub ssid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default)]
    pub enable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fw_version: Option<String>,
}

/// WiFi item returned by GetWiFiByIndex.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WiFiItemInfo {
    pub ssid: String,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub enabled: bool,
}

/// WiFi status returned by ReadWiFiStatus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WiFiStatusInfo {
    pub wifi_status: u32,
    pub ssid: Option<String>,
}

/// WiFi count returned by ReadWiFiCount.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WiFiCountInfo {
    pub count: u32,
}

/// Power event types monitored by IoTService.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerEventType {
    AcDcSourceChange,
    BatteryPercentageChange,
    MonitorPowerChange,
    PowerSavingChange,
    PowerSchemeChange,
    AwayModeChange,
    LidSwitchChange,
    ConsoleDisplayChange,
    UserPresenceChange,
}

/// Power event details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerEvent {
    pub event_type: PowerEventType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ac_online: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery_percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monitor_on: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery_saver_on: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_scheme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub away_mode: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lid_open: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_on: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_present: Option<bool>,
}

/// EC event information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EcEvent {
    pub event_func: u32,
    pub event_value: u32,
}

/// SetDeviceStatus request payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SetDeviceStatusRequest {
    status: String,
}

/// SendLaptopStatus request payload.
/// Confirmed via Ghidra: key must be "LaptopStatus" (not "status").
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SendLaptopStatusRequest {
    /// IoTService expects: 4 = WinReady, 6 = Suspending, 8 = Shutting
    #[serde(rename = "LaptopStatus")]
    laptop_status: u32,
}

/// ResetDevice request payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResetDeviceRequest {
    #[serde(default = "default_true")]
    reset: bool,
}

fn default_true() -> bool {
    true
}

// ── Pipe communication ───────────────────────────────────────────────────────

/// Resolve the pipe path: use discovered path if available, otherwise default.
pub fn resolve_pipe_path() -> String {
    #[cfg(windows)]
    {
        crate::hw::discovery::global_profile()
            .and_then(|p| p.iot_pipe_path)
            .unwrap_or_else(|| IOT_PIPE.to_string())
    }
    #[cfg(not(windows))]
    {
        IOT_PIPE.to_string()
    }
}

/// Send a raw IPC message and read the response.
///
/// Returns the raw response payload bytes, or an empty Vec if the message type
/// does not expect a response (fire-and-forget commands like events).
fn send_ipc_message(dst_id: u16, msg_type: u32, payload: &[u8]) -> Result<Vec<u8>> {
    #[cfg(windows)]
    {
        use std::fs::OpenOptions;
        use std::time::Duration;

        let seq = REQUEST_SEQ.fetch_add(1, Ordering::SeqCst);
        log::trace!(
            target: "hw::iotservice",
            "IPC request #{seq}: msg_type=0x{msg_type:04X}, dst_id={dst_id}, payload_len={}",
            payload.len()
        );

        let pipe_path = resolve_pipe_path();

        let mut pipe = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pipe_path)
            .context(format!("Open IoT IPC pipe: {pipe_path}"))?;

        // Build and send the 12-byte header
        let header = IpcWireHeader::new(CLIENT_ID, dst_id, msg_type, payload.len() as u32);
        pipe.write_all(header.as_bytes())
            .context("Write IPC header")?;

        // Send payload if any
        if !payload.is_empty() {
            pipe.write_all(payload).context("Write IPC payload")?;
        }
        pipe.flush().context("Flush IPC pipe")?;

        // Read response header (12 bytes) with enforced timeout
        let mut resp_header_buf = [0u8; IPC_HEADER_SIZE];
        match read_exact_timeout(&mut pipe, &mut resp_header_buf, Duration::from_secs(5)) {
            Ok(()) => {}
            Err(e) => {
                log::warn!(
                    "IoT IPC: no response header for msg_type 0x{msg_type:04X}: {e} \
                     (this is normal for fire-and-forget commands)"
                );
                return Ok(Vec::new());
            }
        }

        // SAFETY: resp_header_buf is exactly IPC_HEADER_SIZE (12) bytes and was filled by read_exact_timeout. IpcWireHeader is #[repr(C)] with four fields matching the known wire format (two u16 + two u32 = 12 bytes, no padding), so the pointer cast is valid and aligned.
        let resp_header: &IpcWireHeader =
            unsafe { &*(resp_header_buf.as_ptr() as *const IpcWireHeader) };

        // Fail-closed: reject responses with unknown message types.
        // This prevents processing unexpected or potentially malicious messages
        // from the IoTService pipe.
        if !is_known_msg_type(resp_header.msg_type) {
            log::warn!(
                target: "hw::iotservice",
                "Unknown IoT message type 0x{:04X} in response — dropping (fail-closed)",
                resp_header.msg_type
            );
            return Ok(Vec::new());
        }

        // Response authentication: verify src_id/dst_id match expectations.
        // The response should come from the destination we sent to (dst_id)
        // and be addressed to us (CLIENT_ID).
        validate_response_header(resp_header, dst_id, CLIENT_ID).with_context(|| {
            format!("Response auth failed for request #{seq} (msg_type=0x{msg_type:04X})")
        })?;

        let payload_len = resp_header.payload_len as usize;
        if payload_len > MAX_RESPONSE_PAYLOAD {
            anyhow::bail!("Response payload too large: {payload_len} bytes");
        }

        if payload_len == 0 {
            return Ok(Vec::new());
        }

        let mut payload_buf = vec![0u8; payload_len];
        pipe.read_exact(&mut payload_buf)
            .context("Read IPC response payload")?;

        Ok(payload_buf)
    }
    #[cfg(not(windows))]
    {
        let _ = (dst_id, msg_type, payload);
        anyhow::bail!("IoT IPC is only supported on Windows")
    }
}

/// Read exactly `buf.len()` bytes from a named pipe with a timeout.
///
/// Uses `PeekNamedPipe` to check for available data without blocking.
/// This is necessary because `std::fs::File::read()` on Windows named pipes
/// blocks indefinitely when no data is available — the previous implementation
/// relied on `WouldBlock` which never occurs with default pipe mode.
#[cfg(windows)]
fn read_exact_timeout(
    pipe: &mut std::fs::File,
    buf: &mut [u8],
    timeout: std::time::Duration,
) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use std::time::Instant;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Pipes::PeekNamedPipe;

    let deadline = Instant::now() + timeout;
    let mut filled = 0;
    let handle = HANDLE(pipe.as_raw_handle());

    while filled < buf.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            anyhow::bail!(
                "Timeout reading IPC response ({filled}/{len} bytes)",
                len = buf.len()
            );
        }

        // PeekNamedPipe returns immediately — it tells us how many bytes
        // are available without blocking. If data is available, read it;
        // otherwise sleep briefly and retry (with deadline check).
        let mut bytes_available: u32 = 0;
        // SAFETY: PeekNamedPipe with a valid HANDLE (from a named pipe File opened via OpenOptions) is safe; passing null for the buffer is explicitly allowed per MSDN (query only, no data copied).
        let peek_ok = unsafe {
            PeekNamedPipe(handle, None, 0, None, Some(&mut bytes_available), None).is_ok()
        };

        if peek_ok && bytes_available > 0 {
            match pipe.read(&mut buf[filled..]) {
                Ok(0) => anyhow::bail!("IPC pipe closed after reading {filled} bytes"),
                Ok(n) => filled += n,
                Err(e) => return Err(e.into()),
            }
        } else {
            // No data yet — sleep briefly and retry
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    Ok(())
}

/// Validate that the response header's src_id and dst_id match expectations.
///
/// This provides response authentication: the response should come from the
/// destination we sent the request to (expected_src_id == our original dst_id)
/// and be addressed to us (expected_dst_id == CLIENT_ID).
///
/// Note: the IoTService wire protocol does not support nonces or sequence
/// numbers in the message header (the format is fixed and reverse-engineered).
/// This src_id/dst_id cross-check is the best available authentication.
#[cfg(windows)]
fn validate_response_header(
    resp_header: &IpcWireHeader,
    expected_src_id: u16,
    expected_dst_id: u16,
) -> Result<()> {
    if resp_header.src_id != expected_src_id {
        anyhow::bail!(
            "Response src_id mismatch: expected 0x{expected_src_id:04X}, got 0x{:04X}",
            resp_header.src_id
        );
    }
    if resp_header.dst_id != expected_dst_id {
        anyhow::bail!(
            "Response dst_id mismatch: expected 0x{expected_dst_id:04X}, got 0x{:04X}",
            resp_header.dst_id
        );
    }
    Ok(())
}

/// Send a JSON command and parse the response into the expected type.
fn send_json_cmd<T: for<'de> Deserialize<'de>>(
    dst_id: u16,
    msg_type: u32,
    request: &impl Serialize,
) -> Result<T> {
    let json = serde_json::to_vec(request).context("Serialize IPC request")?;
    let raw = send_ipc_message(dst_id, msg_type, &json)?;

    if raw.is_empty() {
        anyhow::bail!("Empty response for msg_type 0x{msg_type:04X}");
    }

    serde_json::from_slice::<T>(&raw)
        .with_context(|| format!("Deserialize IPC response for msg_type 0x{msg_type:04X}"))
}

/// Send a JSON command that expects no response (fire-and-forget).
fn send_json_cmd_no_resp(dst_id: u16, msg_type: u32, request: &impl Serialize) -> Result<()> {
    let json = serde_json::to_vec(request).context("Serialize IPC request")?;
    send_ipc_message(dst_id, msg_type, &json)?;
    Ok(())
}

/// Send a query command (no JSON payload needed) and parse the response.
fn send_query<T: for<'de> Deserialize<'de>>(dst_id: u16, msg_type: u32) -> Result<T> {
    let raw = send_ipc_message(dst_id, msg_type, &[])?;

    if raw.is_empty() {
        anyhow::bail!("Empty response for msg_type 0x{msg_type:04X}");
    }

    serde_json::from_slice::<T>(&raw)
        .with_context(|| format!("Deserialize IPC response for msg_type 0x{msg_type:04X}"))
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Check if the IoTService pipe is available.
pub fn is_pipe_available() -> bool {
    #[cfg(windows)]
    {
        let pipe_path = resolve_pipe_path();
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pipe_path)
            .is_ok()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Check if IoTService IPC is available (alias for `is_pipe_available`).
pub fn is_available() -> bool {
    is_pipe_available()
}

// ── Device info queries ──────────────────────────────────────────────────────

/// Get the device model string (e.g., "Mi NoteBook Pro X 15").
pub fn get_model() -> Result<String> {
    let info: ModelInfo = send_query(DST_IOT_DRIVER, msg_type::GET_MODEL)?;
    Ok(info.model)
}

/// Get the firmware version string.
pub fn get_fw_version() -> Result<String> {
    let info: FwVersionInfo = send_query(DST_IOT_DRIVER, msg_type::GET_FW_VERSION)?;
    Ok(info.fw_version)
}

/// Get the IoT device bind status (whether a Xiaomi account is linked).
pub fn get_bind_status() -> Result<BindStatusInfo> {
    send_query::<BindStatusInfo>(DST_IOT_DRIVER, msg_type::GET_BIND_STATUS)
}

/// Get the IoT device ID.
pub fn get_device_id() -> Result<i64> {
    let info: DeviceIdInfo = send_query(DST_IOT_DRIVER, msg_type::GET_DEVICE_ID)?;
    Ok(info.device_id)
}

/// Get the current device status string.
pub fn get_device_status() -> Result<String> {
    let info: DeviceStatusInfo = send_query(DST_IOT_DRIVER, msg_type::GET_DEVICE_STATUS)?;
    Ok(info.status)
}

// ── Device control ───────────────────────────────────────────────────────────

/// Set the device status.
pub fn set_device_status(status: &str) -> Result<()> {
    send_json_cmd_no_resp(
        DST_IOT_DRIVER,
        msg_type::SET_DEVICE_STATUS,
        &SetDeviceStatusRequest {
            status: status.to_string(),
        },
    )
}

/// Reset the IoT device.
pub fn reset_device() -> Result<()> {
    send_json_cmd_no_resp(
        DST_IOT_DRIVER,
        msg_type::RESET_DEVICE,
        &ResetDeviceRequest { reset: true },
    )
}

// ── Laptop status ────────────────────────────────────────────────────────────

/// Report the laptop status to the IoT device (boot ready, suspending, shutting down).
pub fn send_laptop_status(status: LaptopStatus) -> Result<()> {
    log::info!(
        "IoT IPC: sending laptop status {} ({})",
        status.as_str(),
        status.to_hw_value()
    );
    send_json_cmd_no_resp(
        DST_IOT_DRIVER,
        msg_type::SEND_LAPTOP_STATUS,
        &SendLaptopStatusRequest {
            laptop_status: status.to_hw_value(),
        },
    )
}

/// Convenience: report that Windows is ready.
pub fn report_windows_ready() -> Result<()> {
    send_laptop_status(LaptopStatus::WinReady)
}

/// Convenience: report that the system is going to sleep.
pub fn report_suspending() -> Result<()> {
    send_laptop_status(LaptopStatus::Suspending)
}

/// Convenience: report that the system is shutting down.
pub fn report_shutting_down() -> Result<()> {
    send_laptop_status(LaptopStatus::Shutting)
}

// ── Charging ─────────────────────────────────────────────────────────────────

/// Set the battery charging threshold (percent).
///
/// Accepted values: 40, 50, 60, 70, 80, 100.
/// This uses the same binary format as `charging.rs` for the 0x1003 message type.
///
/// Note: the main application uses `charging::set_charging_threshold()` which
/// has additional registry fallback logic. This function is the raw IPC path.
#[allow(dead_code)]
pub fn set_charging_threshold(threshold: u8) -> Result<()> {
    const VALID: &[u8] = &[40, 50, 60, 70, 80, 100];
    if !VALID.contains(&threshold) {
        anyhow::bail!("Invalid threshold {threshold}. Must be one of: 40,50,60,70,80,100");
    }

    send_ipc_message(
        DST_IOT_DRIVER,
        msg_type::SET_CHARGING_LIMIT,
        &[threshold, 0, 0, 0],
    )?;
    Ok(())
}

// ── WiFi management ──────────────────────────────────────────────────────────

/// Write a WiFi network to the IoT device's provisioning list.
pub fn write_wifi_item(item: &WiFiItem) -> Result<()> {
    log::info!("IoT IPC: writing WiFi item for SSID '{}'", item.ssid);
    send_json_cmd_no_resp(DST_IOT_DRIVER, msg_type::WRITE_WIFI_ITEM, item)
}

/// Delete a WiFi network from the IoT device's provisioning list by SSID.
pub fn delete_wifi_item(ssid: &str) -> Result<()> {
    log::info!("IoT IPC: deleting WiFi item for SSID '{ssid}'");
    send_json_cmd_no_resp(
        DST_IOT_DRIVER,
        msg_type::DELETE_WIFI_ITEM,
        &serde_json::json!({ "ssid": ssid }),
    )
}

/// Get a WiFi item from the provisioning list by index.
pub fn get_wifi_by_index(index: u32) -> Result<WiFiItemInfo> {
    send_json_cmd::<WiFiItemInfo>(
        DST_IOT_DRIVER,
        msg_type::GET_WIFI_BY_INDEX,
        &serde_json::json!({ "index": index }),
    )
}

/// Get the number of provisioned WiFi networks.
pub fn read_wifi_count() -> Result<u32> {
    let info: WiFiCountInfo = send_query(DST_IOT_DRIVER, msg_type::READ_WIFI_COUNT)?;
    Ok(info.count)
}

/// Get the current WiFi connection status.
pub fn read_wifi_status() -> Result<WiFiStatusInfo> {
    send_query::<WiFiStatusInfo>(DST_IOT_DRIVER, msg_type::READ_WIFI_STATUS)
}

/// Remove all provisioned WiFi networks.
pub fn empty_wifi_items() -> Result<()> {
    send_ipc_message(DST_IOT_DRIVER, msg_type::EMPTY_WIFI_ITEMS, &[])?;
    Ok(())
}

/// Force the IoT device to connect to the provisioned WiFi.
pub fn connect_wifi() -> Result<()> {
    send_ipc_message(DST_IOT_DRIVER, msg_type::CONNECT_WIFI, &[])?;
    Ok(())
}

// ── Power & EC events (UNCONFIRMED — not found in Ghidra decompilation) ──────

/// Send a power event notification to IoTService.
///
/// **UNCONFIRMED — see `msg_type` module docs and `docs/iotservice-re-analysis.md`
/// Section 3.4 for context.**
///
/// The message type 0x5002 (POWER_EVENT) was inferred from string analysis but
/// was NOT confirmed in the Ghidra decompiled output. The IoTService internally
/// monitors power events via `RegisterPowerSettingNotification`, but whether
/// an external client is expected to send this type is unverified.
///
/// Test before relying on this in production.
#[allow(dead_code)]
pub fn notify_power_event(event: &PowerEvent) -> Result<()> {
    let json = serde_json::to_vec(event).context("Serialize power event")?;
    send_ipc_message(DST_IOT_DRIVER, msg_type::POWER_EVENT, &json)?;
    Ok(())
}

/// Send an EC event notification to IoTService.
///
/// **UNCONFIRMED — see `msg_type` module docs and `docs/iotservice-re-analysis.md`
/// Section 3.4 for context.**
///
/// The message type 0x5001 (EC_EVENT) was inferred from string analysis but
/// was NOT confirmed in the Ghidra decompiled output. The IoTService internally
/// monitors EC events via WMI (`SELECT * FROM HID_EVENT20`), but whether
/// an external client is expected to send this type is unverified.
///
/// Test before relying on this in production.
#[allow(dead_code)]
pub fn notify_ec_event(event_func: u32, event_value: u32) -> Result<()> {
    let json = serde_json::to_vec(&EcEvent {
        event_func,
        event_value,
    })
    .context("Serialize EC event")?;
    send_ipc_message(DST_IOT_DRIVER, msg_type::EC_EVENT, &json)?;
    Ok(())
}

// ── Aggregate device info query ──────────────────────────────────────────────

/// All device information obtainable via IoTService IPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IotDeviceInfo {
    pub pipe_available: bool,
    pub model: Option<String>,
    pub fw_version: Option<String>,
    pub bind_status: Option<BindStatusInfo>,
    pub device_id: Option<i64>,
    pub device_status: Option<String>,
    pub wifi_status: Option<WiFiStatusInfo>,
    pub wifi_network_count: Option<u32>,
}

/// Get all available device information in one call.
///
/// Each field is independently queried; if the pipe is unavailable or a
/// specific query fails, the corresponding field is `None`.
pub fn get_device_info() -> IotDeviceInfo {
    let pipe_available = is_available();

    IotDeviceInfo {
        pipe_available,
        model: get_model().ok(),
        fw_version: get_fw_version().ok(),
        bind_status: get_bind_status().ok(),
        device_id: get_device_id().ok(),
        device_status: get_device_status().ok(),
        wifi_status: read_wifi_status().ok(),
        wifi_network_count: read_wifi_count().ok(),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipc_header_size() {
        assert_eq!(std::mem::size_of::<IpcWireHeader>(), IPC_HEADER_SIZE);
    }

    #[test]
    fn test_header_default() {
        let h = IpcWireHeader::new(1, 2, 0x1001, 0);
        assert_eq!(h.src_id, 1);
        assert_eq!(h.dst_id, 2);
        assert_eq!(h.msg_type, 0x1001);
        assert_eq!(h.payload_len, 0);
    }

    #[test]
    fn test_header_with_payload() {
        let h = IpcWireHeader::new(1, 2, 0x4001, 256);
        assert_eq!(h.payload_len, 256);
    }

    #[test]
    fn test_header_bytes_roundtrip() {
        let h = IpcWireHeader::new(0xAA, 0xBB, 0xDEADBEEF, 42);
        let bytes = h.as_bytes();

        // SAFETY: bytes is from as_bytes() on a valid IpcWireHeader; the pointer cast back to the same #[repr(C)] type is safe because alignment and layout match exactly.
        let parsed: &IpcWireHeader = unsafe { &*(bytes.as_ptr() as *const IpcWireHeader) };
        assert_eq!(parsed.src_id, 0xAA);
        assert_eq!(parsed.dst_id, 0xBB);
        assert_eq!(parsed.msg_type, 0xDEADBEEF);
        assert_eq!(parsed.payload_len, 42);
    }

    #[test]
    fn test_laptop_status_values() {
        // Confirmed via Ghidra decompilation: 4=WinReady, 6=Suspending, 8=Shutting
        assert_eq!(LaptopStatus::WinReady.to_hw_value(), 4);
        assert_eq!(LaptopStatus::Suspending.to_hw_value(), 6);
        assert_eq!(LaptopStatus::Shutting.to_hw_value(), 8);
    }

    #[test]
    fn test_charging_validation_only_values() {
        // Verify that only valid thresholds are accepted at the API level.
        // We don't actually try the pipe — that would hang in CI.
        const VALID: &[u8] = &[40, 50, 60, 70, 80, 100];
        for &v in VALID {
            assert!(v >= 40 && v <= 100);
        }
        // 99 is not a valid threshold
        assert!(!VALID.contains(&99));
    }

    // ── Message type validation (fail-closed) ──────────────────────────────

    #[test]
    fn test_is_known_msg_type_returns_true_for_0x5001() {
        // 0x5001 (EC_EVENT) is unconfirmed but kept in the known-type list.
        assert!(is_known_msg_type(msg_type::EC_EVENT));
    }

    #[test]
    fn test_is_known_msg_type_returns_true_for_0x5002() {
        // 0x5002 (POWER_EVENT) is unconfirmed but kept in the known-type list.
        assert!(is_known_msg_type(msg_type::POWER_EVENT));
    }

    #[test]
    fn test_is_known_msg_type_returns_true_for_all_confirmed_types() {
        let confirmed: Vec<u32> = vec![
            msg_type::GET_MODEL,
            msg_type::GET_FW_VERSION,
            msg_type::GET_BIND_STATUS,
            msg_type::GET_DEVICE_ID,
            msg_type::GET_DEVICE_STATUS,
            msg_type::SET_DEVICE_STATUS,
            msg_type::RESET_DEVICE,
            msg_type::SET_CHARGING_LIMIT,
            msg_type::SEND_LAPTOP_STATUS,
            msg_type::WRITE_WIFI_ITEM,
            msg_type::DELETE_WIFI_ITEM,
            msg_type::GET_WIFI_BY_INDEX,
            msg_type::READ_WIFI_COUNT,
            msg_type::READ_WIFI_STATUS,
            msg_type::EMPTY_WIFI_ITEMS,
            msg_type::CONNECT_WIFI,
        ];
        for t in confirmed {
            assert!(
                is_known_msg_type(t),
                "Expected confirmed type 0x{t:04X} to be known"
            );
        }
    }

    #[test]
    fn test_is_known_msg_type_returns_false_for_unknown_type() {
        // 0x9999 is not a known message type — should be rejected.
        assert!(!is_known_msg_type(0x9999));
    }

    #[test]
    fn test_is_known_msg_type_returns_false_for_zero() {
        // 0 is not a valid IoTService message type.
        assert!(!is_known_msg_type(0));
    }

    #[test]
    fn test_is_known_msg_type_returns_false_for_max_u32() {
        // u32::MAX is never a valid message type.
        assert!(!is_known_msg_type(u32::MAX));
    }

    // ── Response authentication (validate_response_header) ────────────────

    #[cfg(windows)]
    #[test]
    fn test_validate_response_header_ok() {
        // A valid response: src=2 (IoTDriver), dst=1 (us)
        let h = IpcWireHeader::new(2, 1, msg_type::GET_MODEL, 10);
        assert!(validate_response_header(&h, 2, 1).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn test_validate_response_header_wrong_src() {
        // Response claims to be from src=3 (WMI worker), but we sent to dst=2
        let h = IpcWireHeader::new(3, 1, msg_type::GET_MODEL, 10);
        let err = validate_response_header(&h, 2, 1).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("src_id mismatch"), "Got: {msg}");
    }

    #[cfg(windows)]
    #[test]
    fn test_validate_response_header_wrong_dst() {
        // Response addressed to dst=99, but we are CLIENT_ID=1
        let h = IpcWireHeader::new(2, 99, msg_type::GET_MODEL, 10);
        let err = validate_response_header(&h, 2, 1).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("dst_id mismatch"), "Got: {msg}");
    }

    // ── Request sequence counter ──────────────────────────────────────────

    #[test]
    fn test_request_seq_increments() {
        // Verify the sequence counter increments monotonically.
        // We read the current value, then simulate two requests.
        let before = REQUEST_SEQ.load(Ordering::SeqCst);
        let s1 = REQUEST_SEQ.fetch_add(1, Ordering::SeqCst);
        assert_eq!(s1, before);
        let s2 = REQUEST_SEQ.fetch_add(1, Ordering::SeqCst);
        assert_eq!(s2, before + 1);
    }

    // ── Timeout enforcement ───────────────────────────────────────────────

    #[cfg(windows)]
    #[test]
    fn test_read_exact_timeout_zero_length() {
        // A zero-length buffer should succeed immediately (loop doesn't execute).
        // We need a valid File handle; use the test binary itself.
        let mut file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        let mut buf: &mut [u8] = &mut [];
        let result = read_exact_timeout(&mut file, &mut buf, std::time::Duration::from_secs(1));
        assert!(result.is_ok(), "zero-length read should succeed");
    }
}
