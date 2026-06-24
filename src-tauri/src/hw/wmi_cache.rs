//! WMI Connection Cache — S4-001
//!
//! Caches WMI connections per-thread to eliminate per-query connection churn.
//! WMI connections are COM objects (thread-affine), so we use `thread_local!`
//! storage. All access must happen inside `spawn_blocking` (per S3-004).
//! Two namespaces are cached:
//! - `ROOT\CIMV2` — system info, display, discovery
//! - `ROOT\WMI` — battery, brightness

use crate::hw::errors::{HardwareError, HardwareResult};
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
        let com_cimv2 = COMLibrary::new().map_err(|e| HardwareError::WmiQuery {
            query: "COMLibrary::new cimv2".into(),
            source: Box::new(e),
        })?;
        let cimv2 = WMIConnection::with_namespace_path(NS_CIMV2, com_cimv2).map_err(|e| {
            HardwareError::WmiQuery {
                query: format!("WMIConnection cimv2 namespace={NS_CIMV2}"),
                source: Box::new(e),
            }
        })?;
        let com_wmi = COMLibrary::new().map_err(|e| HardwareError::WmiQuery {
            query: "COMLibrary::new wmi".into(),
            source: Box::new(e),
        })?;
        let wmi = WMIConnection::with_namespace_path(NS_WMI, com_wmi).map_err(|e| {
            HardwareError::WmiQuery {
                query: format!("WMIConnection wmi namespace={NS_WMI}"),
                source: Box::new(e),
            }
        })?;
        Ok(Self { cimv2, wmi })
    }
}

/// Execute a closure with the cached `ROOT\CIMV2` connection, with one retry.
///
/// On first call (or after [`invalidate`]), initialises COM and creates the
/// connection. If the closure returns an error, the cache is ONLY invalidated
/// for connection-level errors (COM init failure, namespace binding failure),
/// NOT for transient query errors.
pub fn with_cimv2<F, T>(f: F) -> HardwareResult<T>
where
    F: Fn(&WMIConnection) -> anyhow::Result<T>,
{
    let result: anyhow::Result<T> = crate::util::retry::with_retry("WMI cimv2 query", || {
        WMI_CACHE.with(|cell| {
            let mut cache_ref = cell.borrow_mut();
            if cache_ref.is_none() {
                *cache_ref = Some(WmiThreadCache::init()?);
            }
            let c = cache_ref.as_ref().unwrap();
            f(&c.cimv2)
        })
    });
    match &result {
        Err(e) if is_connection_error(e) => {
            log::info!("WMI cache: cimv2 connection error, invalidating: {e}");
            WMI_CACHE.with(|cell| {
                *cell.borrow_mut() = None;
            });
        }
        Err(e) => {
            log::debug!("WMI cache: cimv2 transient query error (cache preserved): {e}");
        }
        Ok(_) => {}
    }
    result.map_err(HardwareError::from)
}

/// Execute a closure with the cached `ROOT\WMI` connection, with one retry.
///
/// Same semantics as [`with_cimv2`].
pub fn with_wmi<F, T>(f: F) -> HardwareResult<T>
where
    F: Fn(&WMIConnection) -> anyhow::Result<T>,
{
    let result: anyhow::Result<T> = crate::util::retry::with_retry("WMI wmi query", || {
        WMI_CACHE.with(|cell| {
            let mut cache_ref = cell.borrow_mut();
            if cache_ref.is_none() {
                *cache_ref = Some(WmiThreadCache::init()?);
            }
            let c = cache_ref.as_ref().unwrap();
            f(&c.wmi)
        })
    });
    match &result {
        Err(e) if is_connection_error(e) => {
            log::info!("WMI cache: wmi connection error, invalidating: {e}");
            WMI_CACHE.with(|cell| {
                *cell.borrow_mut() = None;
            });
        }
        Err(e) => {
            log::debug!("WMI cache: wmi transient query error (cache preserved): {e}");
        }
        Ok(_) => {}
    }
    result.map_err(HardwareError::from)
}

/// Returns `true` if the error message indicates a connection-level failure
/// (COM init, namespace binding, or WMI infrastructure) vs. a transient
/// query error (class not found, invalid query syntax).
fn is_connection_error(e: &anyhow::Error) -> bool {
    let msg = e.to_string().to_lowercase();
    msg.contains("com")
        || msg.contains("connection")
        || msg.contains("binding")
        || msg.contains("namespace")
}

#[expect(dead_code)]
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
