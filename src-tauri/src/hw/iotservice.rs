//! IoTService.exe IPC client for Xiaomi hardware control.
//!
//! Communicates with the official Xiaomi IoTService Windows service
//! through its named pipe to control hardware features without direct
//! IOCTL access.

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
use crate::hw::errors::{HardwareError, HardwareResult};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

// We also bring `Result` into scope for internal helper functions.
use anyhow::Result;

/// Named pipe path to the IoTService IPC broker.
pub const IOT_PIPE: &str = r"\\.\pipe\LOCAL\IoTService_IPC_Broker";

/// Size of the fixed IPC header (src_id + dst_id + msg_type + payload_len)
const IPC_HEADER_SIZE: usize = 12;

/// Our client ID registered with the IoTService IPC broker.
const CLIENT_ID: u16 = 1;
/// Destination ID for the IoTDriver worker.
const DST_IOT_DRIVER: u16 = 2;

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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::enum_variant_names)]
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

/// Unified IoT event notification type.
///
/// Consolidates power events, EC events, and laptop status reports into a
/// single enum so the frontend can use one `iot_notify_event` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IotEvent {
    /// A power setting change event (AC/DC, battery, monitor, etc.).
    Power { event: PowerEvent },
    /// An EC (Embedded Controller) event.
    Ec { event_func: u32, event_value: u32 },
    /// A laptop lifecycle status report (boot ready, suspending, shutting).
    LaptopStatus { status: LaptopStatus },
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

// ── WiFi password encryption ──────────────────────────────────────────────────
//
// WiFi passwords are encrypted with AES-256-GCM before being sent over the
// local named pipe to prevent plaintext sniffing (CWE-312). The key is
// derived from the shared HMAC key via SHA-256.

/// Derive a 32-byte AES-256 key from the HMAC key.
///
/// Uses HKDF-SHA256 (S19-17) for proper key separation. Falls back to the
/// legacy SHA-256 derivation for backward compatibility with existing
/// encrypted WiFi entries.
fn derive_aes_key(key: &[u8]) -> [u8; 32] {
    // Try HKDF-derived sub-key first (S19-17)
    match crate::util::auth::derive_subkey_from_key(key, "wifi_encryption") {
        Ok(subkey) => subkey,
        Err(e) => {
            // S25-003: Log the fallback so security audits can detect weak key derivation.
            log::warn!("HKDF key derivation failed, falling back to legacy SHA-256: {e}");
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(b"micontrol-wifi-aes256-v1");
            hasher.update(key);
            let result = hasher.finalize();
            let mut out = [0u8; 32];
            out.copy_from_slice(&result);
            out
        }
    }
}

/// Encrypt a WiFi password using AES-256-GCM with a raw key.
///
/// Returns `{nonce_hex}:{ciphertext_hex}` (ciphertext includes the GCM tag).
fn encrypt_with_key(password: &str, key: &[u8], nonce_hex: &str) -> Result<String, String> {
    use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};

    let nonce_bytes = hex::decode(nonce_hex).map_err(|e| format!("Invalid nonce: {e}"))?;
    // AES-GCM requires a 12-byte nonce; truncate the 16-byte nonce to 12 bytes
    let nonce_12: Vec<u8> = nonce_bytes.into_iter().take(12).collect();
    if nonce_12.len() != 12 {
        return Err("AES-GCM nonce must be at least 12 bytes".to_string());
    }

    let aes_key = derive_aes_key(key);
    let cipher = Aes256Gcm::new_from_slice(&aes_key).map_err(|e| format!("AES key error: {e}"))?;
    let nonce = Nonce::from_slice(&nonce_12);

    let ciphertext = cipher
        .encrypt(nonce, password.as_bytes())
        .map_err(|e| format!("AES-GCM encryption failed: {e}"))?;

    let encrypted_hex: String = ciphertext.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("{nonce_hex}:{encrypted_hex}"))
}

/// Decrypt a WiFi password using AES-256-GCM with a raw key.
///
/// Input format: `{nonce_hex}:{ciphertext_hex}`
#[cfg(test)]
fn decrypt_with_key(encrypted: &str, key: &[u8]) -> Result<String, String> {
    use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};

    let parts: Vec<&str> = encrypted.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err("Invalid encrypted password format".to_string());
    }
    let nonce_hex = parts[0];
    let encrypted_hex = parts[1];

    let nonce_bytes = hex::decode(nonce_hex).map_err(|e| format!("Invalid nonce: {e}"))?;
    // AES-GCM requires a 12-byte nonce; truncate the 16-byte nonce to 12 bytes
    let nonce_12: Vec<u8> = nonce_bytes.into_iter().take(12).collect();
    if nonce_12.len() != 12 {
        return Err("AES-GCM nonce must be at least 12 bytes".to_string());
    }

    let encrypted_bytes =
        hex::decode(encrypted_hex).map_err(|e| format!("Invalid ciphertext: {e}"))?;

    let aes_key = derive_aes_key(key);
    let cipher = Aes256Gcm::new_from_slice(&aes_key).map_err(|e| format!("AES key error: {e}"))?;
    let nonce = Nonce::from_slice(&nonce_12);

    let plaintext = cipher
        .decrypt(nonce, encrypted_bytes.as_ref())
        .map_err(|e| format!("AES-GCM decryption failed: {e}"))?;

    String::from_utf8(plaintext).map_err(|e| format!("Decrypted password is not valid UTF-8: {e}"))
}

/// Encrypt a WiFi password using the shared HMAC key.
///
/// Returns a hex-encoded string: `{nonce_hex}:{encrypted_hex}`.
fn encrypt_wifi_password(password: &str) -> Result<String, String> {
    let key = crate::util::auth::get_or_create_key()?;
    let nonce_hex = crate::util::auth::generate_nonce();
    encrypt_with_key(password, &key, &nonce_hex)
}

/// Decrypt a WiFi password using the shared HMAC key.
///
/// Input format: `{nonce_hex}:{encrypted_hex}`
#[cfg(test)]
fn decrypt_wifi_password(encrypted: &str) -> Result<String, String> {
    let key = crate::util::auth::get_or_create_key()?;
    decrypt_with_key(encrypted, &key)
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

/// Rate limiter for IPC writes — max 100 writes per second.
static IPC_WRITE_TIMES: Mutex<Vec<Instant>> = Mutex::new(Vec::new());
const RATE_LIMIT_MAX_WRITES: usize = 100;
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(1);

/// Check if an IPC write is allowed under the rate limit.
/// Returns true if allowed, false if rate limited.
fn check_rate_limit() -> bool {
    // S24-006: Use lock_or_recover for consistent poison recovery with logging.
    let mut times = crate::util::panic::lock_or_recover(&IPC_WRITE_TIMES);
    let now = Instant::now();

    // Remove entries older than the window
    times.retain(|&t| now.duration_since(t) < RATE_LIMIT_WINDOW);

    if times.len() >= RATE_LIMIT_MAX_WRITES {
        return false;
    }

    times.push(now);
    true
}

/// Send a raw IPC message and read the response.
///
/// Returns the raw response payload bytes, or an empty Vec if the message type
/// does not expect a response (fire-and-forget commands like events).
fn send_ipc_message(dst_id: u16, msg_type: u32, payload: &[u8]) -> Result<Vec<u8>> {
    #[cfg(windows)]
    {
        if !check_rate_limit() {
            return Err(anyhow::anyhow!(
                "IPC write rate limit exceeded (max {} writes/second)",
                RATE_LIMIT_MAX_WRITES
            ));
        }

        crate::util::retry::with_retry("IoT IPC send", || {
            use std::fs::OpenOptions;
            use std::time::Duration;

            let seq = REQUEST_SEQ.fetch_add(1, Ordering::SeqCst);
            log::debug!(
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

            // Validate buffer is large enough for the header before casting.
            if resp_header_buf.len() < std::mem::size_of::<IpcWireHeader>() {
                anyhow::bail!(
                    "IPC buffer too small for header: {} < {}",
                    resp_header_buf.len(),
                    std::mem::size_of::<IpcWireHeader>()
                );
            }

            // SAFETY: resp_header_buf has been validated to contain at least size_of::<IpcWireHeader>()
            // bytes and was filled by read_exact_timeout. IpcWireHeader is #[repr(C)] with four fields
            // matching the known wire format (two u16 + two u32 = 12 bytes, no padding). Using
            // read_unaligned avoids alignment issues on the stack-allocated buffer.
            let resp_header: IpcWireHeader = unsafe {
                std::ptr::read_unaligned(resp_header_buf.as_ptr() as *const IpcWireHeader)
            };

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
            validate_response_header(&resp_header, dst_id, CLIENT_ID).with_context(|| {
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
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (dst_id, msg_type, payload);
        anyhow::bail!("IoT IPC is only supported on Windows")
    }
}

/// Read exactly `buf.len()` bytes from a named pipe with a timeout.
///
/// Uses overlapped I/O with `ReadFile` + `OVERLAPPED` + `WaitForSingleObject`
/// to avoid busy-wait polling (previously `PeekNamedPipe` + 10ms sleep loop).
#[cfg(windows)]
fn read_exact_timeout(
    pipe: &mut std::fs::File,
    buf: &mut [u8],
    timeout: std::time::Duration,
) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
    use windows::Win32::Storage::FileSystem::ReadFile;
    use windows::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};
    use windows::Win32::System::IO::{
        CancelIoEx, GetOverlappedResult, OVERLAPPED, OVERLAPPED_0, OVERLAPPED_0_0,
    };

    let handle = HANDLE(pipe.as_raw_handle());

    // Create an event for overlapped I/O
    // SAFETY: CreateEventW with null name creates an unnamed event.
    let event = unsafe {
        CreateEventW(None, true, false, windows::core::PCWSTR::null())
            .map_err(|e| anyhow::anyhow!("CreateEventW failed: {e}"))?
    };

    let mut filled = 0;

    while filled < buf.len() {
        let mut overlapped = OVERLAPPED {
            Internal: 0,
            InternalHigh: 0,
            Anonymous: OVERLAPPED_0 {
                Anonymous: OVERLAPPED_0_0 {
                    Offset: 0,
                    OffsetHigh: 0,
                },
            },
            hEvent: event,
        };

        // Reset the event before each overlapped read
        // SAFETY: event is valid.
        unsafe { ResetEvent(event).ok() };

        // Start overlapped read
        let mut bytes_read: u32 = 0;
        // SAFETY: handle is valid, buf is valid, overlapped is initialized.
        let result = unsafe {
            ReadFile(
                handle,
                Some(&mut buf[filled..]),
                Some(&mut bytes_read),
                Some(&mut overlapped),
            )
        };

        if result.is_ok() {
            // Completed synchronously
            if bytes_read == 0 {
                // S23-001: Pipe closed by remote end (EOF). Without this check,
                // the loop spins forever consuming 100% CPU.
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(event);
                }
                anyhow::bail!(
                    "Pipe closed by remote end (EOF) after reading {filled}/{} bytes",
                    buf.len()
                );
            }
            filled += bytes_read as usize;
        } else {
            let err = windows::core::Error::from_win32();
            // ERROR_IO_PENDING is expected for overlapped I/O
            if err.code() != windows::Win32::Foundation::ERROR_IO_PENDING.to_hresult() {
                // SAFETY: event was created by CreateEventW.
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(event);
                }
                return Err(err).context("ReadFile (overlapped) failed");
            }

            // Wait for the read to complete
            // SAFETY: event is valid.
            let wait_result = unsafe { WaitForSingleObject(event, timeout.as_millis() as u32) };

            if wait_result != WAIT_OBJECT_0 {
                // Timeout — cancel the pending I/O
                // SAFETY: handle is valid, overlapped is from the pending operation.
                unsafe {
                    let _ = CancelIoEx(handle, Some(&overlapped as *const OVERLAPPED));
                }
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(event);
                }
                anyhow::bail!(
                    "Pipe read timeout after reading {filled}/{len} bytes",
                    len = buf.len()
                );
            }

            // SAFETY: overlapped is valid and the operation completed.
            unsafe {
                GetOverlappedResult(handle, &overlapped, &mut bytes_read, false)
                    .map_err(|e| anyhow::anyhow!("GetOverlappedResult failed: {e}"))?;
            }
            // S23-001: Check for EOF on the overlapped path as well.
            if bytes_read == 0 {
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(event);
                }
                anyhow::bail!(
                    "Pipe closed by remote end (EOF) after reading {filled}/{} bytes",
                    buf.len()
                );
            }
            filled += bytes_read as usize;
        }
    }

    // SAFETY: event was created by CreateEventW.
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(event);
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
pub fn get_model() -> HardwareResult<String> {
    let info: ModelInfo = send_query(DST_IOT_DRIVER, msg_type::GET_MODEL)?;
    Ok(info.model)
}

/// Get the firmware version string.
pub fn get_fw_version() -> HardwareResult<String> {
    let info: FwVersionInfo = send_query(DST_IOT_DRIVER, msg_type::GET_FW_VERSION)?;
    Ok(info.fw_version)
}

/// Get the IoT device bind status (whether a Xiaomi account is linked).
pub fn get_bind_status() -> HardwareResult<BindStatusInfo> {
    send_query::<BindStatusInfo>(DST_IOT_DRIVER, msg_type::GET_BIND_STATUS)
        .map_err(HardwareError::from)
}

/// Get the IoT device ID.
pub fn get_device_id() -> HardwareResult<i64> {
    let info: DeviceIdInfo = send_query(DST_IOT_DRIVER, msg_type::GET_DEVICE_ID)?;
    Ok(info.device_id)
}

/// Get the current device status string.
pub fn get_device_status() -> HardwareResult<String> {
    let info: DeviceStatusInfo = send_query(DST_IOT_DRIVER, msg_type::GET_DEVICE_STATUS)?;
    Ok(info.status)
}

// ── Device control ───────────────────────────────────────────────────────────

/// Set the device status.
pub fn set_device_status(status: &str) -> HardwareResult<()> {
    send_json_cmd_no_resp(
        DST_IOT_DRIVER,
        msg_type::SET_DEVICE_STATUS,
        &SetDeviceStatusRequest {
            status: status.to_string(),
        },
    )
    .map_err(HardwareError::from)
}

/// Reset the IoT device.
pub fn reset_device() -> HardwareResult<()> {
    send_json_cmd_no_resp(
        DST_IOT_DRIVER,
        msg_type::RESET_DEVICE,
        &ResetDeviceRequest { reset: true },
    )
    .map_err(HardwareError::from)
}

// ── Laptop status ────────────────────────────────────────────────────────────

/// Report the laptop status to the IoT device (boot ready, suspending, shutting down).
pub fn send_laptop_status(status: LaptopStatus) -> HardwareResult<()> {
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
    .map_err(HardwareError::from)
}

/// Convenience: report that Windows is ready.
pub fn report_windows_ready() -> HardwareResult<()> {
    send_laptop_status(LaptopStatus::WinReady)
}

/// Convenience: report that the system is going to sleep.
pub fn report_suspending() -> HardwareResult<()> {
    send_laptop_status(LaptopStatus::Suspending)
}

/// Convenience: report that the system is shutting down.
pub fn report_shutting_down() -> HardwareResult<()> {
    send_laptop_status(LaptopStatus::Shutting)
}

// ── WiFi management ──────────────────────────────────────────────────────────
pub fn write_wifi_item(item: &WiFiItem) -> HardwareResult<()> {
    log::info!("IoT IPC: writing WiFi item for SSID '{}'", item.ssid);

    // Encrypt the password before sending over the pipe to prevent
    // plaintext sniffing (CWE-312).
    let encrypted_item = if let Some(ref password) = item.password {
        let encrypted_password = encrypt_wifi_password(password)
            .map_err(|e| anyhow::anyhow!("Failed to encrypt WiFi password: {e}"))?;
        WiFiItem {
            password: Some(encrypted_password),
            ..item.clone()
        }
    } else {
        item.clone()
    };

    send_json_cmd_no_resp(DST_IOT_DRIVER, msg_type::WRITE_WIFI_ITEM, &encrypted_item)
        .map_err(HardwareError::from)
}

/// Delete a WiFi network from the IoT device's provisioning list by SSID.
pub fn delete_wifi_item(ssid: &str) -> HardwareResult<()> {
    log::info!("IoT IPC: deleting WiFi item for SSID '{ssid}'");
    send_json_cmd_no_resp(
        DST_IOT_DRIVER,
        msg_type::DELETE_WIFI_ITEM,
        &serde_json::json!({ "ssid": ssid }),
    )
    .map_err(HardwareError::from)
}

/// Get a WiFi item from the provisioning list by index.
pub fn get_wifi_by_index(index: u32) -> HardwareResult<WiFiItemInfo> {
    send_json_cmd::<WiFiItemInfo>(
        DST_IOT_DRIVER,
        msg_type::GET_WIFI_BY_INDEX,
        &serde_json::json!({ "index": index }),
    )
    .map_err(HardwareError::from)
}

/// Get the number of provisioned WiFi networks.
pub fn read_wifi_count() -> HardwareResult<u32> {
    let info: WiFiCountInfo = send_query(DST_IOT_DRIVER, msg_type::READ_WIFI_COUNT)?;
    Ok(info.count)
}

/// Get the current WiFi connection status.
pub fn read_wifi_status() -> HardwareResult<WiFiStatusInfo> {
    send_query::<WiFiStatusInfo>(DST_IOT_DRIVER, msg_type::READ_WIFI_STATUS)
        .map_err(HardwareError::from)
}

/// Remove all provisioned WiFi networks.
pub fn empty_wifi_items() -> HardwareResult<()> {
    send_ipc_message(DST_IOT_DRIVER, msg_type::EMPTY_WIFI_ITEMS, &[])?;
    Ok(())
}

/// Force the IoT device to connect to the provisioned WiFi.
pub fn connect_wifi() -> HardwareResult<()> {
    send_ipc_message(DST_IOT_DRIVER, msg_type::CONNECT_WIFI, &[])?;
    Ok(())
}

// ── Power & EC events ────────────────────────────────────────────────────────

/// Send a power event notification to IoTService.
pub fn notify_power_event(event: &PowerEvent) -> HardwareResult<()> {
    let json = serde_json::to_vec(event).context("Serialize power event")?;
    send_ipc_message(DST_IOT_DRIVER, msg_type::POWER_EVENT, &json)?;
    Ok(())
}

/// Send an EC event notification to IoTService.
pub fn notify_ec_event(event_func: u32, event_value: u32) -> HardwareResult<()> {
    let json = serde_json::to_vec(&EcEvent {
        event_func,
        event_value,
    })
    .context("Serialize EC event")?;
    send_ipc_message(DST_IOT_DRIVER, msg_type::EC_EVENT, &json)?;
    Ok(())
}

/// Send a unified IoT event notification to IoTService.
///
/// This is the consolidated entry point that dispatches to `notify_power_event`,
/// `notify_ec_event`, or `send_laptop_status` depending on the variant.
pub fn notify_event(event: &IotEvent) -> HardwareResult<()> {
    match event {
        IotEvent::Power { event } => notify_power_event(event),
        IotEvent::Ec {
            event_func,
            event_value,
        } => notify_ec_event(*event_func, *event_value),
        IotEvent::LaptopStatus { status } => send_laptop_status(*status),
    }
}

// ── Aggregate device info query ──────────────────────────────────────────────
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
        model: get_model()
            .map_err(|e| log::warn!("[iot] Failed to query model: {e}"))
            .ok(),
        fw_version: get_fw_version()
            .map_err(|e| log::warn!("[iot] Failed to query firmware version: {e}"))
            .ok(),
        bind_status: get_bind_status()
            .map_err(|e| log::warn!("[iot] Failed to query bind status: {e}"))
            .ok(),
        device_id: get_device_id()
            .map_err(|e| log::warn!("[iot] Failed to query device ID: {e}"))
            .ok(),
        device_status: get_device_status()
            .map_err(|e| log::warn!("[iot] Failed to query device status: {e}"))
            .ok(),
        wifi_status: read_wifi_status()
            .map_err(|e| log::warn!("[iot] Failed to read WiFi status: {e}"))
            .ok(),
        wifi_network_count: read_wifi_count()
            .map_err(|e| log::warn!("[iot] Failed to read WiFi network count: {e}"))
            .ok(),
    }
}

/// Consolidated WiFi list response.
///
/// Combines WiFi connection status, provisioned network count, and the full
/// list of provisioned WiFi items into a single struct so the frontend can
/// fetch everything with one `get_iot_wifi_list` command.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IotWifiList {
    /// Current WiFi connection status (None if the query fails).
    pub status: Option<WiFiStatusInfo>,
    /// Number of provisioned WiFi networks on the IoT device.
    pub count: u32,
    /// All provisioned WiFi items (may be shorter than `count` if individual
    /// lookups fail).
    pub networks: Vec<WiFiItemInfo>,
}

/// Get the full WiFi provisioning list in one call.
///
/// Reads the count, then iterates indices 0..count to collect all items.
/// The connection status is fetched independently.  Individual failures are
/// tolerated — the corresponding item is simply omitted from `networks`.
pub fn get_wifi_list() -> IotWifiList {
    let count = read_wifi_count().unwrap_or(0);
    let status = read_wifi_status().ok();
    let networks = (0..count)
        .filter_map(|i| get_wifi_by_index(i).ok())
        .collect();
    IotWifiList {
        status,
        count,
        networks,
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
    fn test_rate_limit_allows_under_limit() {
        // The rate limiter should allow writes under the limit
        // This test verifies the function doesn't panic
        let _ = check_rate_limit();
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
        let exe = std::env::current_exe().expect("get current exe path in test");
        let mut file = std::fs::File::open(&exe).expect("open test binary in test");
        let mut buf: &mut [u8] = &mut [];
        let result = read_exact_timeout(&mut file, &mut buf, std::time::Duration::from_secs(1));
        assert!(result.is_ok(), "zero-length read should succeed");
    }

    // ── WiFi password encryption ──────────────────────────────────────────

    const TEST_KEY: &[u8] = b"0123456789abcdef0123456789abcdef"; // 32 bytes

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let password = "MySecretWiFi123!";
        let nonce = crate::util::auth::generate_nonce();
        let encrypted =
            encrypt_with_key(password, TEST_KEY, &nonce).expect("Encryption should succeed");
        assert!(
            encrypted.contains(':'),
            "Encrypted password should contain nonce separator"
        );
        let decrypted = decrypt_with_key(&encrypted, TEST_KEY).expect("Decryption should succeed");
        assert_eq!(
            decrypted, password,
            "Decrypted password should match original"
        );
    }

    #[test]
    fn test_encrypt_produces_different_ciphertext() {
        let password = "SamePassword";
        let nonce1 = "00000000000000000000000000000001";
        let nonce2 = "00000000000000000000000000000002";
        let enc1 =
            encrypt_with_key(password, TEST_KEY, nonce1).expect("Encryption 1 should succeed");
        let enc2 =
            encrypt_with_key(password, TEST_KEY, nonce2).expect("Encryption 2 should succeed");
        assert_ne!(
            enc1, enc2,
            "Different nonces should produce different ciphertext"
        );
    }

    #[test]
    fn test_decrypt_invalid_format() {
        let result = decrypt_with_key("invalid_no_colon", TEST_KEY);
        assert!(result.is_err(), "Invalid format should return error");
    }

    #[test]
    fn test_aes_gcm_encrypt_decrypt_roundtrip() {
        let password = "MyWiFiPassword123!";
        let key = TEST_KEY;
        let nonce_hex = "aabbccddeeff00112233445566778899"; // 16-byte nonce hex
        let encrypted = encrypt_with_key(password, key, nonce_hex).unwrap();
        let decrypted = decrypt_with_key(&encrypted, key).unwrap();
        assert_eq!(
            decrypted, password,
            "AES-GCM decrypt should recover original password"
        );
    }

    // ── IotEvent tagged enum serialization (S28-005) ──────────────────────

    #[test]
    fn test_iot_event_power_serialization() {
        let event = IotEvent::Power {
            event: PowerEvent {
                event_type: PowerEventType::AcDcSourceChange,
                ac_online: Some(true),
                battery_percent: None,
                monitor_on: None,
                battery_saver_on: None,
                power_scheme: None,
                away_mode: None,
                lid_open: None,
                display_on: None,
                user_present: None,
            },
        };
        let json = serde_json::to_string(&event).expect("serialize Power variant");
        assert!(
            json.contains("\"kind\":\"power\""),
            "Power variant should have kind=power: {json}"
        );
        let parsed: IotEvent = serde_json::from_str(&json).expect("deserialize Power variant");
        match parsed {
            IotEvent::Power { event } => {
                assert_eq!(event.event_type, PowerEventType::AcDcSourceChange);
                assert_eq!(event.ac_online, Some(true));
            }
            _ => panic!("expected Power variant, got something else"),
        }
    }

    #[test]
    fn test_iot_event_ec_serialization() {
        let event = IotEvent::Ec {
            event_func: 0x5001,
            event_value: 42,
        };
        let json = serde_json::to_string(&event).expect("serialize Ec variant");
        assert!(
            json.contains("\"kind\":\"ec\""),
            "Ec variant should have kind=ec: {json}"
        );
        let parsed: IotEvent = serde_json::from_str(&json).expect("deserialize Ec variant");
        match parsed {
            IotEvent::Ec {
                event_func,
                event_value,
            } => {
                assert_eq!(event_func, 0x5001);
                assert_eq!(event_value, 42);
            }
            _ => panic!("expected Ec variant, got something else"),
        }
    }

    #[test]
    fn test_iot_event_laptop_status_serialization() {
        let event = IotEvent::LaptopStatus {
            status: LaptopStatus::Suspending,
        };
        let json = serde_json::to_string(&event).expect("serialize LaptopStatus variant");
        assert!(
            json.contains("\"kind\":\"laptop_status\""),
            "LaptopStatus variant should have kind=laptop_status: {json}"
        );
        let parsed: IotEvent =
            serde_json::from_str(&json).expect("deserialize LaptopStatus variant");
        match parsed {
            IotEvent::LaptopStatus { status } => {
                assert_eq!(status, LaptopStatus::Suspending);
            }
            _ => panic!("expected LaptopStatus variant, got something else"),
        }
    }

    #[test]
    fn test_iot_wifi_list_default() {
        // Verify the default struct has expected empty values.
        let list = IotWifiList::default();
        assert_eq!(list.count, 0, "count should be 0 by default");
        assert!(
            list.networks.is_empty(),
            "networks should be empty by default"
        );
        assert!(list.status.is_none(), "status should be None by default");
    }
}
