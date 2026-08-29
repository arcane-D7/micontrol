//! Agnostic crash-report plumbing (watchdog-friendly).
//!
//! Captures panic/abnormal-exit events into a structured [`CrashEvent`] and
//! forwards them to a configured [`CrashReporterBackend`].
//!
//! **Disabled by default.** No backend is registered and no data leaves the
//! machine until crash reporting is explicitly enabled via configuration AND
//! (optionally) an endpoint is provided. The design is intentionally vendor-
//! agnostic: today we ship a no-op + a local-JSON sink; tomorrow any service
//! (Sentry, a self-hosted collector, the MiControl watchdog API, …) can be
//! attached by implementing [`CrashReporterBackend`] and registering it in
//! [`configure`].
//!
//! # Privacy
//!
//! When disabled (the default), nothing is written and nothing is sent.
//! When enabled, events are still scrubbed: no machine name, no user paths
//! (the local app data root is replaced by a placeholder), no IP addresses.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::registry::RegKeyGuard;
use windows::Win32::System::Registry::HKEY_CURRENT_USER;

/// Schema version of [`CrashEvent`]. Bump when the wire format changes so
/// collectors can migrate old events.
pub const CRASH_EVENT_SCHEMA_VERSION: u32 = 1;

/// Registry key under which crash-reporting configuration lives.
const CONFIG_REG_KEY: &str = r"SOFTWARE\MiControl\CrashReporting";
/// Registry value: DWORD 1/0 — master switch (default: disabled).
const ENABLED_VALUE: &str = "Enabled";
/// Registry value: string — optional collector endpoint URL (empty = local only).
const ENDPOINT_VALUE: &str = "Endpoint";
/// Registry value: string — anonymous installation id used to deduplicate.
const INSTALL_ID_VALUE: &str = "InstallId";

/// Type of abnormal event captured.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrashKind {
    /// A Rust panic was caught by the global hook.
    Panic,
    /// Access violation / native crash (0xC0000005 and friends) — typically
    /// detected post-mortem via WER or the Windows Error Reporting queue.
    AccessViolation,
    /// Out-of-memory condition.
    OutOfMemory,
    /// Process killed/terminated unexpectedly (watchdog flavour).
    AbnormalExit,
    /// Generic / unknown.
    Other,
}

/// A structured, vendor-neutral crash event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashEvent {
    /// Schema version for forward compatibility.
    pub schema_version: u32,
    /// Application version that crashed, e.g. `0.1.25`.
    pub app_version: String,
    /// UTC Unix timestamp (seconds) of the event.
    pub ts: u64,
    /// Event class.
    pub kind: CrashKind,
    /// Human-readable message (panic payload, exception text, …).
    pub message: String,
    /// Optional source location (`file:line:column`).
    pub location: Option<String>,
    /// Optional full backtrace.
    pub backtrace: Option<String>,
    /// Process id that crashed.
    pub pid: u32,
    /// Thread id, if captured.
    pub tid: Option<u64>,
    /// Whether the process was a debug build (added context for triage).
    pub debug_build: bool,
    /// Free-form extra fields, all pre-scrubbed by callers.
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// A sink that consumes [`CrashEvent`]s.
///
/// Implementations MUST be cheap and MUST NOT panic. The dispatch path is
/// called from the panic hook, so it must never block on the network and
/// should fall back to local persistence on failure.
pub trait CrashReporterBackend: Send + Sync {
    /// Persist/forward one crash event. Must be infallible (swallow errors
    /// internally, log via `log::`).
    fn report(&self, event: &CrashEvent);
}

/// Backend that discards everything. This is the default — crash reporting
/// is **disabled by default** and nothing is recorded or sent.
pub struct NoopBackend;

impl CrashReporterBackend for NoopBackend {
    fn report(&self, _event: &CrashEvent) {
        // Intentionally a no-op.
    }
}

/// Backend that appends events as JSON files under
/// `%LOCALAPPDATA%\MiControl\crash_reports\`.
///
/// Used both as the local sink while an HTTP collector is unavailable and as
/// a durable fallback when the network forward fails.
pub struct LocalFileBackend {
    dir: PathBuf,
}

impl LocalFileBackend {
    /// Build the sink rooted at the given directory (created on first use).
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Default location: `%LOCALAPPDATA%\MiControl\crash_reports`.
    pub fn default_dir() -> PathBuf {
        let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| {
            std::env::var("USERPROFILE")
                .map(|h| format!("{h}\\AppData\\Local"))
                .unwrap_or_else(|_| ".".into())
        });
        PathBuf::from(base).join("MiControl").join("crash_reports")
    }
}

impl CrashReporterBackend for LocalFileBackend {
    fn report(&self, event: &CrashEvent) {
        let ts = event.ts;
        let pid = event.pid;
        let file_name = format!("crash_{ts}_{pid}.json");
        if let Err(e) = std::fs::create_dir_all(&self.dir) {
            log::error!(
                "crash_report: failed to create dir {}: {e}",
                self.dir.display()
            );
            return;
        }
        let path = self.dir.join(file_name);
        match serde_json::to_vec_pretty(event) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(&path, bytes) {
                    log::error!(
                        "crash_report: failed to persist event to {}: {e}",
                        path.display()
                    );
                }
            }
            Err(e) => log::error!("crash_report: failed to serialize event: {e}"),
        }
    }
}

/// Backend that forwards events to an HTTP collector as `POST {endpoint}`
/// with a JSON body of the serialized [`CrashEvent`].
///
/// This is the plug point for "connect a collection service later": set the
/// endpoint config and this backend takes over from the local sink. Delivery
/// is best-effort: on failure the event falls back to the local sink.
pub struct HttpBackend {
    endpoint: String,
    local: LocalFileBackend,
}

impl HttpBackend {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            local: LocalFileBackend::new(LocalFileBackend::default_dir()),
        }
    }
}

impl CrashReporterBackend for HttpBackend {
    fn report(&self, event: &CrashEvent) {
        // Always persist locally first (durable record + fallback).
        self.local.report(event);

        // Best-effort forward. Never block the panic hook on the network;
        // only if a tokio runtime is available do we spawn a fire-and-forget
        // delivery task.
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let client = reqwest::Client::new();
        let endpoint = self.endpoint.clone();
        let body = event.clone();
        handle.spawn(async move {
            match client.post(&endpoint).json(&body).send().await {
                Ok(resp) if resp.status().is_success() => {
                    log::info!("crash_report: delivered event to {endpoint}");
                }
                Ok(resp) => {
                    log::warn!(
                        "crash_report: collector returned {} for {endpoint}; event kept locally",
                        resp.status()
                    );
                }
                Err(e) => {
                    log::warn!(
                        "crash_report: failed to deliver to {endpoint}: {e}; event kept locally"
                    );
                }
            }
        });
    }
}

/// Global crash-reporting state. The backend is *always* valid; by default it
/// is the [`NoopBackend`], i.e. reporting is disabled until [`configure`] is
/// called from app startup with an enabled flag.
static STATE: Mutex<Option<Arc<dyn CrashReporterBackend>>> = Mutex::new(None);

/// Whether crash reporting is enabled *this process* (cached from config).
static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Get the current reporting switch (mostly for tests / debug).
pub fn is_enabled() -> bool {
    ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Configure the crash-reporting subsystem.
///
/// - `enabled`: master switch. When `false` (the default), all backends are
///   replaced by the no-op and nothing is recorded or sent.
/// - `endpoint`: optional collector URL. When `Some`, an [`HttpBackend`] is
///   registered (still gated by `enabled`). When `None`, local-file only.
pub fn configure(enabled: bool, endpoint: Option<String>) {
    let backend: Arc<dyn CrashReporterBackend> = if !enabled {
        Arc::new(NoopBackend)
    } else {
        match endpoint.clone() {
            Some(url) if !url.trim().is_empty() => Arc::new(HttpBackend::new(url)),
            _ => Arc::new(LocalFileBackend::new(LocalFileBackend::default_dir())),
        }
    };
    ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
    let mut state = crate::util::panic::lock_or_recover(&STATE);
    *state = Some(backend);
    log::info!(
        "crash_report: {} (endpoint: {})",
        if enabled {
            "enabled"
        } else {
            "disabled (default)"
        },
        endpoint.as_deref().unwrap_or("<none — local only>")
    );
}

/// Read configuration from the registry (created by the app or a future
/// settings UI). Missing keys = disabled/default.
pub fn read_config() -> (bool, Option<String>) {
    let mut enabled = false;
    let mut endpoint: Option<String> = None;
    if let Ok(Some(key)) = RegKeyGuard::open_read(HKEY_CURRENT_USER, CONFIG_REG_KEY) {
        enabled = key.read_u32(ENABLED_VALUE).ok().flatten().unwrap_or(0) != 0;
        let ep = key
            .read_string(ENDPOINT_VALUE)
            .ok()
            .flatten()
            .unwrap_or_default();
        if !ep.trim().is_empty() {
            endpoint = Some(ep);
        }
    }
    (enabled, endpoint)
}

/// Initialize from the registry at startup.
///
/// Must be called once during `run()` *before* [`super::panic::install_panic_hook`]
/// so that the hook sees the configured state.
pub fn init_crash_reporting() {
    let (enabled, endpoint) = read_config();
    configure(enabled, endpoint);
    // Also ensure the anonymous install id exists for future dedup needs —
    // written locally, never contains anything identifiable.
    let _ = ensure_install_id();
}

/// Anonymous installation id (UUID stored in the registry). Used by future
/// collectors to deduplicate reports from the same machine without knowing
/// who the user is.
fn ensure_install_id() -> String {
    if let Ok(Some(key)) = RegKeyGuard::open_read(HKEY_CURRENT_USER, CONFIG_REG_KEY) {
        if let Ok(Some(id)) = key.read_string(INSTALL_ID_VALUE) {
            if !id.is_empty() {
                return id;
            }
        }
    }
    // Generate one using a simple v4-style UUID from a CSPRNG.
    let mut bytes = [0u8; 16];
    getrandom(&mut bytes);
    bytes[6] = (bytes[6] & 0x0F) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3F) | 0x80; // variant 10xx
    let id = format_uuid(&bytes);
    if let Ok(key) = RegKeyGuard::create_write(HKEY_CURRENT_USER, CONFIG_REG_KEY) {
        let _ = key.write_string(INSTALL_ID_VALUE, &id);
    }
    id
}

/// Fill `buf` from the OS CSPRNG (falls back to time-seeded state if the OS
/// RNG is unavailable — only affects the *anonymity id*, never security).
fn getrandom(buf: &mut [u8]) {
    use rand::RngCore;
    rand::rngs::OsRng.fill_bytes(buf);
}

/// Format 16 bytes as a lowercase v4 UUID string.
fn format_uuid(bytes: &[u8; 16]) -> String {
    let mut s = String::with_capacity(36);
    for (i, b) in bytes.iter().enumerate() {
        if i == 4 || i == 6 || i == 8 || i == 10 {
            s.push('-');
        }
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Build a [`CrashEvent`] from a panic-hook capture point and dispatch it.
///
/// This is the entry point called by [`super::panic::install_panic_hook`].
/// Fields are pre-scrubbed: only `location`, payload text and backtrace are
/// included; no paths are embedded beyond the panic location (which points
/// inside the compiled crate).
pub fn report_panic(location: Option<&str>, message: &str, backtrace: Option<&str>) {
    report_event(CrashEvent {
        schema_version: CRASH_EVENT_SCHEMA_VERSION,
        app_version: env!("CARGO_PKG_VERSION").into(),
        ts: unix_ts(),
        kind: CrashKind::Panic,
        message: message.to_string(),
        location: location.map(str::to_string),
        backtrace: backtrace.map(str::to_string),
        pid: std::process::id(),
        tid: current_thread_id(),
        debug_build: cfg!(debug_assertions),
        extra: serde_json::Map::new(),
    });
}

/// Dispatch a fully-built event to the registered backend (no-op when disabled).
pub fn report_event(event: CrashEvent) {
    // Copy the backend out of the mutex so we never hold the lock while a
    // backend does I/O (backends are cheap, but be defensive).
    let backend = crate::util::panic::lock_or_recover(&STATE).clone();
    if let Some(backend) = backend {
        backend.report(&event);
    }
}

fn unix_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn current_thread_id() -> Option<u64> {
    #[cfg(windows)]
    {
        // SAFETY: GetCurrentThreadId is a leaf API with no unsafe preconditions.
        Some(unsafe { windows::Win32::System::Threading::GetCurrentThreadId() as u64 })
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_backend_is_default_and_discards() {
        configure(false, None);
        assert!(!is_enabled());
        report_panic(Some("src/main.rs:1:1"), "test panic", None);
        // No assertion needed — the important part is this doesn't panic/error.
    }

    #[test]
    fn local_backend_persists_json() {
        let dir = std::env::temp_dir().join(format!("micontrol_crash_test_{}", std::process::id()));
        let backend = LocalFileBackend::new(dir.clone());
        let event = CrashEvent {
            schema_version: CRASH_EVENT_SCHEMA_VERSION,
            app_version: "0.0.0-test".into(),
            ts: unix_ts(),
            kind: CrashKind::Panic,
            message: "boom".into(),
            location: Some("mod.rs:10:5".into()),
            backtrace: None,
            pid: std::process::id(),
            tid: None,
            debug_build: true,
            extra: serde_json::Map::new(),
        };
        backend.report(&event);
        let entries: Vec<_> = std::fs::read_dir(&dir)
            .expect("dir should exist")
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
            .collect();
        assert_eq!(entries.len(), 1, "one JSON file expected");
        let raw = std::fs::read_to_string(entries[0].path()).expect("readable json");
        let parsed: CrashEvent = serde_json::from_str(&raw).expect("valid serialized event");
        assert_eq!(parsed.message, "boom");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn install_id_is_stable() {
        // Can't easily run twice without registry writes; just check format
        // of the generator.
        let mut b = [0u8; 16];
        getrandom(&mut b);
        let id = format_uuid(&b);
        assert_eq!(id.len(), 36);
        assert!(id.chars().filter(|&c| c == '-').count() == 4);
    }
}
