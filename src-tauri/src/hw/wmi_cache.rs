//! WMI Connection Cache — S4-001
//!
//! Caches WMI connections per-thread to eliminate per-query connection churn.
//! WMI connections are COM objects (thread-affine), so we use `thread_local!`
//! storage. All access must happen inside `spawn_blocking` (per S3-004).
//!
//! Two namespaces are cached:
//! - `ROOT\CIMV2` — system info, display, discovery
//! - `ROOT\WMI` — battery, brightness

use anyhow::Context;
use std::cell::RefCell;
use wmi::{COMLibrary, WMIConnection};

/// Namespace for system-level WMI queries (Win32_* classes).
pub const NS_CIMV2: &str = "ROOT\\CIMV2";
/// Namespace for WMI-level queries (BatteryStatus, WmiMonitorBrightness, etc.).
pub const NS_WMI: &str = "ROOT\\WMI";

thread_local! {
    /// Per-thread cached WMI connections.
    /// Each `WMIConnection` internally owns its `COMLibrary`, so COM stays
    /// initialised for the lifetime of the cache entry.
    static WMI_CACHE: RefCell<Option<WmiThreadCache>> = const { RefCell::new(None) };
}

struct WmiThreadCache {
    cimv2: WMIConnection,
    wmi: WMIConnection,
}

impl WmiThreadCache {
    fn init() -> anyhow::Result<Self> {
        // COM is reference-counted per thread, so creating two COMLibrary
        // instances on the same thread is safe — CoInitializeEx is called
        // twice and CoUninitialize is called twice on drop.
        let com_cimv2 = COMLibrary::new().context("WMI cache: COM init (cimv2)")?;
        let cimv2 = WMIConnection::with_namespace_path(NS_CIMV2, com_cimv2)
            .context("WMI cache: connect cimv2")?;
        let com_wmi = COMLibrary::new().context("WMI cache: COM init (wmi)")?;
        let wmi = WMIConnection::with_namespace_path(NS_WMI, com_wmi)
            .context("WMI cache: connect root\\wmi")?;
        Ok(Self { cimv2, wmi })
    }
}

/// Execute a closure with the cached `ROOT\CIMV2` connection.
///
/// On first call (or after [`invalidate`]), initialises COM and creates the
/// connection. If the closure returns an error, the cache is invalidated so
/// the next call will recreate the connection transparently.
pub fn with_cimv2<F, T>(f: F) -> anyhow::Result<T>
where
    F: FnOnce(&WMIConnection) -> anyhow::Result<T>,
{
    WMI_CACHE.with(|cell| {
        let mut cache_ref = cell.borrow_mut();
        if cache_ref.is_none() {
            *cache_ref = Some(WmiThreadCache::init()?);
        }
        let result = {
            let c = cache_ref.as_ref().unwrap();
            f(&c.cimv2)
        };
        if result.is_err() {
            log::info!("WMI cache: cimv2 query failed, invalidating connection");
            *cache_ref = None;
        }
        result
    })
}

/// Execute a closure with the cached `ROOT\WMI` connection.
///
/// Same semantics as [`with_cimv2`].
pub fn with_wmi<F, T>(f: F) -> anyhow::Result<T>
where
    F: FnOnce(&WMIConnection) -> anyhow::Result<T>,
{
    WMI_CACHE.with(|cell| {
        let mut cache_ref = cell.borrow_mut();
        if cache_ref.is_none() {
            *cache_ref = Some(WmiThreadCache::init()?);
        }
        let result = {
            let c = cache_ref.as_ref().unwrap();
            f(&c.wmi)
        };
        if result.is_err() {
            log::info!("WMI cache: wmi query failed, invalidating connection");
            *cache_ref = None;
        }
        result
    })
}

/// Invalidate the cached connections on the current thread.
///
/// The next call to [`with_cimv2`] or [`with_wmi`] will recreate them.
/// Call this from error-recovery paths when a WMI query fails with a
/// connection-level error.
pub fn invalidate() {
    WMI_CACHE.with(|cell| {
        *cell.borrow_mut() = None;
    });
}
