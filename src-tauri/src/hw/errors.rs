//! Structured error types for hardware operations.
//!
//! Replaces opaque `anyhow::Error` → `String` conversions with typed errors
//! that carry machine-readable codes for frontend error handling.

use serde::{Deserialize, Serialize};

/// Hardware operation errors.
///
/// Each variant maps to a stable `code` string that the frontend can switch on,
/// plus a human-readable message.
///
/// S25-008: `Debug` includes WMI query strings (developer-facing), but `Display`
/// (user-facing) does not expose them.
#[derive(thiserror::Error)]
pub enum HardwareError {
    /// WMI query failed (COM, namespace binding, or query syntax).
    ///
    /// S25-008: The `query` field is included in `Debug` but NOT in `Display`
    /// to avoid leaking WMI query strings to end users.
    #[error("WMI query failed: {source}")]
    WmiQuery {
        query: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// WMI connection-level failure (COM init, namespace binding, cache unavailable).
    /// Distinct from [`HardwareError::WmiQuery`] which covers transient query errors.
    #[error("WMI connection error: {0}")]
    WmiConnection(String),

    /// I/O error (file, pipe, device).
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// HID device error (preparsed data, caps, report).
    #[error("HID error: {0}")]
    Hid(String),

    /// Invalid configuration value.
    #[error("Invalid config: {0}")]
    InvalidConfig(String),

    /// Operation timed out.
    #[error("Timeout: {0}")]
    Timeout(String),

    /// Hardware feature not supported on this platform.
    #[error("Not supported: {0}")]
    NotSupported(String),

    /// EC RAM validation error (out-of-bounds offset, short read).
    #[error("EC RAM error: {0}")]
    Ecram(String),

    /// WiFi operation error (invalid SSID, connection failure).
    #[error("WiFi error: {0}")]
    Wifi(String),

    /// Display/graphics error (IGCL, brightness, HDR).
    #[error("Display error: {0}")]
    Display(String),

    /// Touchpad error (HID report, gesture).
    #[error("Touchpad error: {0}")]
    Touchpad(String),

    /// Battery/charging error.
    #[error("Battery error: {0}")]
    Battery(String),

    /// Hotkey error (hook, config, script).
    #[error("Hotkey error: {0}")]
    Hotkey(String),

    /// Registry error (read/write).
    #[error("Registry error: {0}")]
    Registry(String),

    /// Elevated bridge error (IPC, auth, dispatch).
    #[error("Elevated bridge error: {0}")]
    ElevatedBridge(String),

    /// AI request error (HTTP, keyring, response parsing).
    #[error("AI request error: {0}")]
    AiRequest(String),

    /// Hotkey configuration error (invalid config, serialization).
    #[error("Hotkey config error: {0}")]
    HotkeyConfig(String),

    /// Blocking task join error (spawn_blocking panic or cancellation).
    #[error("Task join error: {0}")]
    TaskJoin(String),

    /// Generic hardware error (catch-all for uncategorized failures).
    #[error("Hardware error: {0}")]
    Other(String),
}

// S25-008: Manual Debug impl that includes WMI query strings for developers.
// The Display impl (via #[error(...)]) omits query strings for end users.
impl std::fmt::Debug for HardwareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WmiQuery { query, source } => f
                .debug_struct("WmiQuery")
                .field("query", query)
                .field("source", source)
                .finish(),
            Self::WmiConnection(arg0) => f.debug_tuple("WmiConnection").field(arg0).finish(),
            Self::Io(arg0) => f.debug_tuple("Io").field(arg0).finish(),
            Self::Hid(arg0) => f.debug_tuple("Hid").field(arg0).finish(),
            Self::InvalidConfig(arg0) => f.debug_tuple("InvalidConfig").field(arg0).finish(),
            Self::Timeout(arg0) => f.debug_tuple("Timeout").field(arg0).finish(),
            Self::NotSupported(arg0) => f.debug_tuple("NotSupported").field(arg0).finish(),
            Self::Ecram(arg0) => f.debug_tuple("Ecram").field(arg0).finish(),
            Self::Wifi(arg0) => f.debug_tuple("Wifi").field(arg0).finish(),
            Self::Display(arg0) => f.debug_tuple("Display").field(arg0).finish(),
            Self::Touchpad(arg0) => f.debug_tuple("Touchpad").field(arg0).finish(),
            Self::Battery(arg0) => f.debug_tuple("Battery").field(arg0).finish(),
            Self::Hotkey(arg0) => f.debug_tuple("Hotkey").field(arg0).finish(),
            Self::Registry(arg0) => f.debug_tuple("Registry").field(arg0).finish(),
            Self::ElevatedBridge(arg0) => f.debug_tuple("ElevatedBridge").field(arg0).finish(),
            Self::AiRequest(arg0) => f.debug_tuple("AiRequest").field(arg0).finish(),
            Self::HotkeyConfig(arg0) => f.debug_tuple("HotkeyConfig").field(arg0).finish(),
            Self::TaskJoin(arg0) => f.debug_tuple("TaskJoin").field(arg0).finish(),
            Self::Other(arg0) => f.debug_tuple("Other").field(arg0).finish(),
        }
    }
}

impl HardwareError {
    /// Returns the stable machine-readable error code for this error variant.
    ///
    /// The frontend uses this to switch on error types instead of
    /// string-matching human-readable messages.
    pub fn code(&self) -> &'static str {
        match self {
            Self::WmiQuery { .. } => "wmi_query",
            Self::WmiConnection(_) => "wmi_connection",
            Self::Io(_) => "io",
            Self::Hid(_) => "hid",
            Self::InvalidConfig(_) => "invalid_config",
            Self::Timeout(_) => "timeout",
            Self::NotSupported(_) => "not_supported",
            Self::Ecram(_) => "ecram",
            Self::Wifi(_) => "wifi",
            Self::Display(_) => "display",
            Self::Touchpad(_) => "touchpad",
            Self::Battery(_) => "battery",
            Self::Hotkey(_) => "hotkey",
            Self::Registry(_) => "registry",
            Self::ElevatedBridge(_) => "elevated_bridge",
            Self::AiRequest(_) => "ai_request",
            Self::HotkeyConfig(_) => "hotkey_config",
            Self::TaskJoin(_) => "task_join",
            Self::Other(_) => "other",
        }
    }
}

impl From<anyhow::Error> for HardwareError {
    fn from(e: anyhow::Error) -> Self {
        // Check if the inner error is already a HardwareError (e.g. from
        // wmi_cache operations) to avoid double-wrapping.
        match e.downcast::<HardwareError>() {
            Ok(hw) => hw,
            Err(original) => Self::Other(original.to_string()),
        }
    }
}

impl From<serde_json::Error> for HardwareError {
    fn from(e: serde_json::Error) -> Self {
        Self::InvalidConfig(format!("JSON: {e}"))
    }
}

/// A serializable error response sent to the frontend.
///
/// This is the JSON representation of a `HardwareError` that crosses the
/// Tauri IPC boundary. The frontend can switch on `code` for typed handling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// Machine-readable error code (e.g. "wmi_query", "wifi", "timeout").
    pub code: String,
    /// Human-readable error message.
    pub message: String,
}

impl ErrorResponse {
    /// Create an `ErrorResponse` with an explicit code and message.
    #[allow(dead_code)]
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
        }
    }

    /// Create an `ErrorResponse` from a `HardwareError`.
    pub fn from_error(e: &HardwareError) -> Self {
        Self {
            code: e.code().to_string(),
            message: e.to_string(),
        }
    }

    /// Create an `ErrorResponse` from any error implementing `Display`.
    ///
    /// Public API: useful for converting non-`HardwareError` failures into
    /// a frontend-compatible error response.
    #[allow(dead_code)]
    pub fn from_display(e: &dyn std::fmt::Display) -> Self {
        Self {
            code: "other".to_string(),
            message: e.to_string(),
        }
    }
}

impl From<HardwareError> for ErrorResponse {
    fn from(e: HardwareError) -> Self {
        Self::from_error(&e)
    }
}

impl From<String> for ErrorResponse {
    fn from(s: String) -> Self {
        ErrorResponse {
            // S24-08: Standardized to "other" for all fallback codes.
            code: "other".to_string(),
            message: s,
        }
    }
}

impl From<anyhow::Error> for ErrorResponse {
    fn from(e: anyhow::Error) -> Self {
        ErrorResponse {
            // S24-08: Standardized to "other" for all fallback codes.
            code: "other".to_string(),
            message: e.to_string(),
        }
    }
}

impl From<HardwareError> for String {
    /// Convert to a JSON string for Tauri command error responses.
    fn from(e: HardwareError) -> String {
        let resp = ErrorResponse::from_error(&e);
        serde_json::to_string(&resp)
            .unwrap_or_else(|_| format!(r#"{{"code":"other","message":"{}"}}"#, e))
    }
}

/// A type alias for results that return `HardwareError`.
pub type HardwareResult<T> = Result<T, HardwareError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wmi_query_code() {
        let e = HardwareError::WmiQuery {
            query: "SELECT * FROM Win32_Battery".to_string(),
            source: "COM error".into(),
        };
        assert_eq!(e.code(), "wmi_query");
    }

    #[test]
    fn test_wmi_query_display_omits_query_string() {
        // S25-008: Display must NOT contain the WMI query string (user-facing).
        let e = HardwareError::WmiQuery {
            query: "SELECT * FROM Win32_Battery".to_string(),
            source: "COM error".into(),
        };
        let display = format!("{e}");
        assert!(
            !display.contains("SELECT * FROM Win32_Battery"),
            "Display should not contain WMI query string, got: {display}"
        );
        assert!(
            display.contains("WMI query failed"),
            "Display should contain error description, got: {display}"
        );
    }

    #[test]
    fn test_wmi_query_debug_includes_query_string() {
        // S25-008: Debug MUST contain the WMI query string (developer-facing).
        let e = HardwareError::WmiQuery {
            query: "SELECT * FROM Win32_Battery".to_string(),
            source: "COM error".into(),
        };
        let debug = format!("{e:?}");
        assert!(
            debug.contains("SELECT * FROM Win32_Battery"),
            "Debug should contain WMI query string, got: {debug}"
        );
    }

    #[test]
    fn test_io_error_code() {
        let e = HardwareError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "file"));
        assert_eq!(e.code(), "io");
    }

    #[test]
    fn test_wifi_error_code() {
        let e = HardwareError::Wifi("Invalid SSID".to_string());
        assert_eq!(e.code(), "wifi");
    }

    #[test]
    fn test_timeout_error_code() {
        let e = HardwareError::Timeout("Elevated bridge".to_string());
        assert_eq!(e.code(), "timeout");
    }

    #[test]
    fn test_not_supported_code() {
        let e = HardwareError::NotSupported("No ambient light sensor".to_string());
        assert_eq!(e.code(), "not_supported");
    }

    #[test]
    fn test_ecram_error_code() {
        let e = HardwareError::Ecram("Offset out of bounds".to_string());
        assert_eq!(e.code(), "ecram");
    }

    #[test]
    fn test_error_response_serialization() {
        let e = HardwareError::Wifi("Invalid SSID".to_string());
        let resp = ErrorResponse::from_error(&e);
        assert_eq!(resp.code, "wifi");
        assert!(resp.message.contains("Invalid SSID"));

        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"code\":\"wifi\""));
        assert!(json.contains("\"message\""));
    }

    #[test]
    fn test_error_response_from_display() {
        let resp = ErrorResponse::from_display(&"some error".to_string());
        assert_eq!(resp.code, "other");
        assert_eq!(resp.message, "some error");
    }

    #[test]
    fn test_hardware_error_from_string() {
        // S19-03: blanket From<String> removed — use Other() directly.
        let e = HardwareError::Other("custom error".to_string());
        assert_eq!(e.code(), "other");
        assert!(e.to_string().contains("custom error"));
    }

    #[test]
    fn test_hardware_error_from_anyhow() {
        let anyhow_err = anyhow::anyhow!("something failed");
        let e: HardwareError = anyhow_err.into();
        assert_eq!(e.code(), "other");
    }

    #[test]
    fn test_to_string_json() {
        let e = HardwareError::Timeout("bridge".to_string());
        let json: String = e.into();
        assert!(json.contains("\"code\":\"timeout\""));
        assert!(json.contains("\"message\""));
    }

    #[test]
    fn test_all_variants_have_codes() {
        // Ensure every variant has a non-empty code
        let codes = [
            HardwareError::WmiQuery {
                query: String::new(),
                source: "".into(),
            }
            .code(),
            HardwareError::Io(std::io::Error::new(std::io::ErrorKind::Other, "")).code(),
            HardwareError::Hid("".into()).code(),
            HardwareError::InvalidConfig("".into()).code(),
            HardwareError::Timeout("".into()).code(),
            HardwareError::NotSupported("".into()).code(),
            HardwareError::Ecram("".into()).code(),
            HardwareError::Wifi("".into()).code(),
            HardwareError::Display("".into()).code(),
            HardwareError::Touchpad("".into()).code(),
            HardwareError::Battery("".into()).code(),
            HardwareError::Hotkey("".into()).code(),
            HardwareError::Registry("".into()).code(),
            HardwareError::ElevatedBridge("".into()).code(),
            HardwareError::AiRequest("".into()).code(),
            HardwareError::HotkeyConfig("".into()).code(),
            HardwareError::TaskJoin("".into()).code(),
            HardwareError::Other("".into()).code(),
        ];
        for code in &codes {
            assert!(!code.is_empty(), "Error code should not be empty");
        }
        // All codes should be unique
        let mut sorted = codes.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            codes.len(),
            "All error codes should be unique"
        );
    }
}
