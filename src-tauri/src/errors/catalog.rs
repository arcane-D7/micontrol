//! Central error catalog and the `AppError` payload.
//!
//! Contract: `docs/architecture/UNIVERSAL-CONTRACTS.md` section 2. Every error
//! the app can surface has exactly one catalog entry with a stable id, an
//! i18n key, a severity, a retry policy and a suggested user action.
//!
//! Rules enforced here and by `scripts/check-error-catalog.mjs`:
//! - Ids are stable, unique and shaped `domain.snake_case` (they go to logs).
//! - Every code has an `errors.<id>` message in all four i18n locales.
//! - `AppError` is only constructed through a catalog code; a code that is
//!   missing from the catalog is a programming error and fails loudly.
//! - The legacy flat wire codes (`ErrorResponse.code`, matched by the
//!   frontend, e.g. `ecram_driver_blocked`) stay byte-identical; they are
//!   carried by [`ErrorCode::as_str`].

use crate::hw::errors::HardwareError;

/// Severity drives presentation: `Info` renders calm (expected platform
/// behaviour), `Fatal` renders as a genuine "stop and get help" alert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Expected or purely informational (e.g. driver-blocked sensor range).
    Info,
    /// Degraded but recoverable; worth surfacing, not alarming.
    Warning,
    /// A real failure the user should know about.
    Error,
    /// The feature or the app cannot continue without intervention.
    Fatal,
}

/// Automated retry policy for this error class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryPolicy {
    /// Retrying cannot help; require a different action instead.
    Never,
    /// Retry with exponential backoff from `base_ms` up to `max_ms`.
    Backoff { base_ms: u32, max_ms: u32 },
}

/// Suggested user action shown next to the localized message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserAction {
    /// Nothing for the user to do.
    None,
    /// The user may simply retry.
    Retry,
    /// The action needs elevation (the `MiControlElevated` path).
    Elevate,
    /// The installation looks broken; reinstalling is the honest advice.
    Reinstall,
    /// No self-service path; escalate.
    ContactSupport,
}

/// Machine-readable error code.
///
/// Every variant maps every [`HardwareError`] variant (the `EcramBlocked`
/// variant dispatches on its [`crate::hw::errors::RegionBlockKind`] into the
/// three `ecram.*` codes, keeping the platform's expected driver block calm)
/// plus the four catalog-only codes for errors born outside `hw`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    // HardwareError::WmiQuery
    WmiQuery,
    // HardwareError::WmiConnection
    WmiConnection,
    // HardwareError::Io
    Io,
    // HardwareError::Hid
    Hid,
    // HardwareError::InvalidConfig
    InvalidConfig,
    // HardwareError::Timeout
    Timeout,
    // HardwareError::NotSupported
    NotSupported,
    // HardwareError::Ecram
    Ecram,
    // HardwareError::EcramBlocked { kind: RegionBlockKind::DriverBlocked }
    // Expected on this platform; the UI renders it as a calm dash.
    EcramDriverBlocked,
    // HardwareError::EcramBlocked { kind: RegionBlockKind::ServiceUnavailable }
    EcramServiceUnavailable,
    // HardwareError::EcramBlocked { kind: RegionBlockKind::Unknown }
    EcramUnknown,
    // HardwareError::Wifi
    Wifi,
    // HardwareError::Cast
    Cast,
    // HardwareError::Display
    Display,
    // HardwareError::Touchpad
    Touchpad,
    // HardwareError::Battery
    Battery,
    // HardwareError::Hotkey
    Hotkey,
    // HardwareError::Registry
    Registry,
    // HardwareError::ElevatedBridge
    ElevatedBridge,
    // HardwareError::AiRequest
    AiRequest,
    // HardwareError::HotkeyConfig
    HotkeyConfig,
    // HardwareError::TaskJoin
    TaskJoin,
    // HardwareError::Wmi (generic)
    Wmi,
    // HardwareError::Other (catch-all)
    Other,
    // Catalog-only codes: errors born outside the hardware layer.
    /// A spawned process outlived its mandatory timeout and was killed.
    SpawnTimeout,
    /// Persisted state failed validation or cannot be parsed.
    StoreCorrupt,
    /// A stored policy value failed validation and was ignored.
    PolicyInvalid,
    /// The watchdog refused an action (health gate or restart policy).
    WatchdogRejected,
}

impl ErrorCode {
    /// Legacy flat wire code carried in `ErrorResponse.code`.
    ///
    /// The frontend switches on these exact strings (e.g.
    /// `ecram_driver_blocked`); this is a stable public contract, do not
    /// rename or reformat the values.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WmiQuery => "wmi_query",
            Self::WmiConnection => "wmi_connection",
            Self::Io => "io",
            Self::Hid => "hid",
            Self::InvalidConfig => "invalid_config",
            Self::Timeout => "timeout",
            Self::NotSupported => "not_supported",
            Self::Ecram => "ecram",
            Self::EcramDriverBlocked => "ecram_driver_blocked",
            Self::EcramServiceUnavailable => "ecram_service_unavailable",
            Self::EcramUnknown => "ecram_unknown",
            Self::Wifi => "wifi",
            Self::Cast => "cast",
            Self::Display => "display",
            Self::Touchpad => "touchpad",
            Self::Battery => "battery",
            Self::Hotkey => "hotkey",
            Self::Registry => "registry",
            Self::ElevatedBridge => "elevated_bridge",
            Self::AiRequest => "ai_request",
            Self::HotkeyConfig => "hotkey_config",
            Self::TaskJoin => "task_join",
            Self::Wmi => "wmi",
            Self::Other => "other",
            Self::SpawnTimeout => "spawn_timeout",
            Self::StoreCorrupt => "store_corrupt",
            Self::PolicyInvalid => "policy_invalid",
            Self::WatchdogRejected => "watchdog_rejected",
        }
    }

    /// Catalog entry for this code.
    ///
    /// Panics when the code is missing from the catalog: that is a
    /// programming error (the catalog gate and the unit tests pin the
    /// enum/catalog parity), never a runtime condition.
    pub fn spec(self) -> &'static ErrorSpec {
        CATALOG
            .iter()
            .find(|spec| spec.code == self)
            .expect("error code missing from CATALOG; add it to errors/catalog.rs")
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Display carries the legacy wire code so existing `to_string()` call
        // sites (ErrorResponse and Tauri command boundaries) keep emitting the
        // exact strings the frontend matches.
        f.write_str(self.as_str())
    }
}

/// One catalog row: everything the app knows about an error class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorSpec {
    /// The machine-readable code.
    pub code: ErrorCode,
    /// Stable `domain.snake_case` identifier; goes to logs, never changes.
    pub id: &'static str,
    /// Localization key, always `errors.<id>`.
    pub i18n_key: &'static str,
    /// Presentation severity (calm info vs red alert).
    pub severity: Severity,
    /// Automated retry policy.
    pub retry: RetryPolicy,
    /// Suggested user action.
    pub user_action: UserAction,
}

/// The error catalog. One entry per `ErrorCode` variant, no duplicates.
///
/// Adding a code: add the variant to `ErrorCode`, add the row here, and add
/// the `errors.<id>` message to all four locale files. The gate
/// `scripts/check-error-catalog.mjs` and the unit tests fail otherwise.
pub const CATALOG: &[ErrorSpec] = &[
    ErrorSpec {
        code: ErrorCode::WmiQuery,
        id: "wmi.query_failed",
        i18n_key: "errors.wmi.query_failed",
        severity: Severity::Warning,
        retry: RetryPolicy::Backoff {
            base_ms: 200,
            max_ms: 5000,
        },
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::WmiConnection,
        id: "wmi.connection_failed",
        i18n_key: "errors.wmi.connection_failed",
        severity: Severity::Error,
        retry: RetryPolicy::Backoff {
            base_ms: 500,
            max_ms: 10000,
        },
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::Wmi,
        id: "wmi.operation_failed",
        i18n_key: "errors.wmi.operation_failed",
        severity: Severity::Warning,
        retry: RetryPolicy::Backoff {
            base_ms: 200,
            max_ms: 5000,
        },
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::Io,
        id: "io.failure",
        i18n_key: "errors.io.failure",
        severity: Severity::Warning,
        retry: RetryPolicy::Backoff {
            base_ms: 500,
            max_ms: 10000,
        },
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::Hid,
        id: "hid.failure",
        i18n_key: "errors.hid.failure",
        severity: Severity::Warning,
        retry: RetryPolicy::Backoff {
            base_ms: 100,
            max_ms: 2000,
        },
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::InvalidConfig,
        id: "config.invalid",
        i18n_key: "errors.config.invalid",
        severity: Severity::Error,
        retry: RetryPolicy::Never,
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::Timeout,
        id: "hardware.timeout",
        i18n_key: "errors.hardware.timeout",
        severity: Severity::Warning,
        retry: RetryPolicy::Backoff {
            base_ms: 1000,
            max_ms: 15000,
        },
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::NotSupported,
        id: "hardware.unsupported",
        i18n_key: "errors.hardware.unsupported",
        severity: Severity::Info,
        retry: RetryPolicy::Never,
        user_action: UserAction::None,
    },
    ErrorSpec {
        code: ErrorCode::Other,
        id: "hardware.other",
        i18n_key: "errors.hardware.other",
        severity: Severity::Error,
        retry: RetryPolicy::Backoff {
            base_ms: 500,
            max_ms: 10000,
        },
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::Ecram,
        id: "ecram.access_failed",
        i18n_key: "errors.ecram.access_failed",
        severity: Severity::Warning,
        retry: RetryPolicy::Never,
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::EcramDriverBlocked,
        id: "ecram.driver_blocked",
        i18n_key: "errors.ecram.driver_blocked",
        severity: Severity::Info,
        retry: RetryPolicy::Never,
        user_action: UserAction::None,
    },
    ErrorSpec {
        code: ErrorCode::EcramServiceUnavailable,
        id: "ecram.service_unavailable",
        i18n_key: "errors.ecram.service_unavailable",
        severity: Severity::Error,
        retry: RetryPolicy::Backoff {
            base_ms: 1000,
            max_ms: 30000,
        },
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::EcramUnknown,
        id: "ecram.unknown",
        i18n_key: "errors.ecram.unknown",
        severity: Severity::Warning,
        retry: RetryPolicy::Backoff {
            base_ms: 500,
            max_ms: 10000,
        },
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::Wifi,
        id: "wifi.failure",
        i18n_key: "errors.wifi.failure",
        severity: Severity::Warning,
        retry: RetryPolicy::Backoff {
            base_ms: 500,
            max_ms: 10000,
        },
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::Cast,
        id: "cast.failure",
        i18n_key: "errors.cast.failure",
        severity: Severity::Warning,
        retry: RetryPolicy::Backoff {
            base_ms: 500,
            max_ms: 10000,
        },
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::Display,
        id: "display.failure",
        i18n_key: "errors.display.failure",
        severity: Severity::Warning,
        retry: RetryPolicy::Backoff {
            base_ms: 500,
            max_ms: 10000,
        },
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::Touchpad,
        id: "touchpad.failure",
        i18n_key: "errors.touchpad.failure",
        severity: Severity::Warning,
        retry: RetryPolicy::Backoff {
            base_ms: 100,
            max_ms: 2000,
        },
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::Battery,
        id: "battery.failure",
        i18n_key: "errors.battery.failure",
        severity: Severity::Warning,
        retry: RetryPolicy::Backoff {
            base_ms: 500,
            max_ms: 10000,
        },
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::Hotkey,
        id: "hotkey.failure",
        i18n_key: "errors.hotkey.failure",
        severity: Severity::Warning,
        retry: RetryPolicy::Never,
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::HotkeyConfig,
        id: "hotkey.config_invalid",
        i18n_key: "errors.hotkey.config_invalid",
        severity: Severity::Warning,
        retry: RetryPolicy::Never,
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::Registry,
        id: "registry.failure",
        i18n_key: "errors.registry.failure",
        severity: Severity::Warning,
        retry: RetryPolicy::Never,
        user_action: UserAction::Elevate,
    },
    ErrorSpec {
        code: ErrorCode::ElevatedBridge,
        id: "elevated.bridge_failed",
        i18n_key: "errors.elevated.bridge_failed",
        severity: Severity::Error,
        retry: RetryPolicy::Backoff {
            base_ms: 1000,
            max_ms: 30000,
        },
        user_action: UserAction::Elevate,
    },
    ErrorSpec {
        code: ErrorCode::AiRequest,
        id: "ai.request_failed",
        i18n_key: "errors.ai.request_failed",
        severity: Severity::Warning,
        retry: RetryPolicy::Backoff {
            base_ms: 1000,
            max_ms: 15000,
        },
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::TaskJoin,
        id: "task.join_failed",
        i18n_key: "errors.task.join_failed",
        severity: Severity::Error,
        retry: RetryPolicy::Never,
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::SpawnTimeout,
        id: "spawn.timeout",
        i18n_key: "errors.spawn.timeout",
        severity: Severity::Error,
        retry: RetryPolicy::Backoff {
            base_ms: 1000,
            max_ms: 30000,
        },
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::StoreCorrupt,
        id: "store.corrupt",
        i18n_key: "errors.store.corrupt",
        severity: Severity::Error,
        retry: RetryPolicy::Never,
        user_action: UserAction::Reinstall,
    },
    ErrorSpec {
        code: ErrorCode::PolicyInvalid,
        id: "policy.invalid",
        i18n_key: "errors.policy.invalid",
        severity: Severity::Warning,
        retry: RetryPolicy::Never,
        user_action: UserAction::Retry,
    },
    ErrorSpec {
        code: ErrorCode::WatchdogRejected,
        id: "watchdog.rejected",
        i18n_key: "errors.watchdog.rejected",
        severity: Severity::Fatal,
        retry: RetryPolicy::Never,
        user_action: UserAction::ContactSupport,
    },
];

/// The application error payload: a catalog code plus where and why it fired.
///
/// Construct only through [`AppError::new`] or [`AppError::with_source`]
/// (or `From<HardwareError>`); the constructor resolves the code through the
/// catalog, so a code missing from the catalog fails loudly instead of
/// producing an anonymous error.
#[derive(Debug)]
pub struct AppError {
    /// Catalog code describing the failure class.
    pub code: ErrorCode,
    /// Where and what failed, in the caller's own words (goes to logs).
    pub context: String,
    /// The underlying error, when there is one.
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl AppError {
    /// Build an error from a catalog code and a context string.
    ///
    /// The code is resolved through the catalog, so construction fails loudly
    /// if the code has no `ErrorSpec` entry.
    pub fn new(code: ErrorCode, context: impl Into<String>) -> Self {
        // Touch the catalog so an uncatalogued code cannot slip through.
        let _ = code.spec();
        Self {
            code,
            context: context.into(),
            source: None,
        }
    }

    /// Build an error with an underlying source error.
    pub fn with_source(
        code: ErrorCode,
        context: impl Into<String>,
        source: Box<dyn std::error::Error + Send + Sync>,
    ) -> Self {
        let _ = code.spec();
        Self {
            code,
            context: context.into(),
            source: Some(source),
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Log-facing format: stable id first, then context. The localized
        // message comes from the catalog's i18n key, never from this Display.
        write!(f, "{}: {}", self.code.spec().id, self.context)?;
        if let Some(source) = &self.source {
            write!(f, " (caused by: {source})")?;
        }
        Ok(())
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| &**e as &(dyn std::error::Error + 'static))
    }
}

impl From<HardwareError> for AppError {
    fn from(e: HardwareError) -> Self {
        // `code()` is exhaustive over every HardwareError variant, so this
        // mapping cannot forget one; a new variant fails compilation until it
        // is classified in `hw::errors`.
        Self::new(e.code(), e.to_string())
    }
}
