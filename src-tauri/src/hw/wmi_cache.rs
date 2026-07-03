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
                match WmiThreadCache::init() {
                    Ok(cache) => *cache_ref = Some(cache),
                    Err(e) => {
                        log::warn!("WMI cache: cimv2 cache initialization failed: {e}");
                        return Err(e);
                    }
                }
            }
            match cache_ref.as_ref() {
                Some(c) => f(&c.cimv2),
                None => {
                    log::error!("WMI cache: cimv2 cache unavailable after init");
                    Err(anyhow::anyhow!(HardwareError::WmiConnection(
                        "cimv2 cache unavailable after initialization".to_string(),
                    )))
                }
            }
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
///
/// WORKING FORM — DO NOT MODIFY: This function uses `cell.borrow_mut()` which
/// holds a RefCell borrow for the entire duration of the closure. If any code
/// inside the closure calls `with_wmi()` or `with_cimv2()` again (nested call),
/// the second `borrow_mut()` will panic with "RefCell already borrowed".
///
/// Callers MUST NOT nest WMI calls. If you need data from multiple WMI sources,
/// collect all data from one source into a struct, exit the closure, then call
/// the second source separately. See `battery.rs::get_battery_info()` for the
/// correct pattern (BatterySnapshot struct).
pub fn with_wmi<F, T>(f: F) -> HardwareResult<T>
where
    F: Fn(&WMIConnection) -> anyhow::Result<T>,
{
    let result: anyhow::Result<T> = crate::util::retry::with_retry("WMI wmi query", || {
        WMI_CACHE.with(|cell| {
            let mut cache_ref = cell.borrow_mut();
            if cache_ref.is_none() {
                match WmiThreadCache::init() {
                    Ok(cache) => *cache_ref = Some(cache),
                    Err(e) => {
                        log::warn!("WMI cache: wmi cache initialization failed: {e}");
                        return Err(e);
                    }
                }
            }
            match cache_ref.as_ref() {
                Some(c) => f(&c.wmi),
                None => {
                    log::error!("WMI cache: wmi cache unavailable after init");
                    Err(anyhow::anyhow!(HardwareError::WmiConnection(
                        "wmi cache unavailable after initialization".to_string(),
                    )))
                }
            }
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

/// Returns `true` if the error indicates a connection-level failure
/// (COM init, namespace binding, or WMI infrastructure) vs. a transient
/// query error (class not found, invalid query syntax).
///
/// Uses structured error type checking via downcasting instead of fragile
/// substring matching on error messages.
fn is_connection_error(e: &anyhow::Error) -> bool {
    // First, try downcasting to our structured HardwareError
    if let Some(hw_err) = e.downcast_ref::<HardwareError>() {
        return matches!(hw_err, HardwareError::WmiConnection(_));
    }

    // Then, try downcasting to common WMI/COM error types
    if e.downcast_ref::<wmi::WMIError>().is_some() {
        return true;
    }

    // Fallback: check the error chain for connection-related causes using
    // structured iteration rather than blind substring matching
    let mut source: Option<&dyn std::error::Error> = Some(e.as_ref());
    while let Some(err) = source {
        let type_name = std::any::type_name_of_val(err);
        if type_name.contains("WMIError")
            || type_name.contains("COMError")
            || type_name.contains("HResult")
        {
            return true;
        }
        source = err.source();
    }

    false
}

#[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_connection_error_hardware_error_wmi_connection() {
        let err: anyhow::Error =
            anyhow::Error::from(HardwareError::WmiConnection("COM init failed".to_string()));
        assert!(is_connection_error(&err));
    }

    #[test]
    fn test_is_connection_error_hardware_error_wmi_query_is_not_connection() {
        // WmiQuery is a transient query error, NOT a connection error
        let err: anyhow::Error = anyhow::Error::from(HardwareError::WmiQuery {
            query: "SELECT * FROM Win32_Processor".to_string(),
            source: Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                "query failed",
            )),
        });
        assert!(!is_connection_error(&err));
    }

    #[test]
    fn test_is_connection_error_wmi_error_type() {
        // A wmi::WMIError should be detected as a connection error
        let wmi_err = wmi::WMIError::HResultError {
            hres: 0x80040154u32 as i32, // REGDB_E_CLASSNOTREG
        };
        let err: anyhow::Error = anyhow::Error::from(wmi_err);
        assert!(is_connection_error(&err));
    }

    #[test]
    fn test_is_connection_error_generic_string_is_not_connection() {
        // A generic string error should NOT be detected as a connection error
        let err: anyhow::Error = anyhow::Error::msg("some random transient error");
        assert!(!is_connection_error(&err));
    }

    #[test]
    fn test_is_connection_error_io_error_is_not_connection() {
        // A plain IO error should NOT be detected as a connection error
        let err: anyhow::Error = anyhow::Error::from(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "file not found",
        ));
        assert!(!is_connection_error(&err));
    }

    // ── S19-07: Cache lifecycle and thread-local tests ───────────────────────

    #[test]
    fn test_invalidate_clears_thread_local_cache() {
        // invalidate() should set the thread_local cache to None
        invalidate();
        WMI_CACHE.with(|cell| {
            assert!(
                cell.borrow().is_none(),
                "Cache should be None after invalidate()"
            );
        });
    }

    #[test]
    fn test_invalidate_is_idempotent() {
        // Calling invalidate() multiple times should not panic
        invalidate();
        invalidate();
        invalidate();
        WMI_CACHE.with(|cell| {
            assert!(cell.borrow().is_none());
        });
    }

    #[test]
    fn test_thread_local_cache_isolation() {
        // Invalidate on the main thread
        invalidate();

        // Spawn a thread that also invalidates its own cache
        let handle = std::thread::spawn(|| {
            invalidate();
            WMI_CACHE.with(|cell| {
                assert!(
                    cell.borrow().is_none(),
                    "Cache should be None on spawned thread"
                );
            });
        });
        handle.join().unwrap();

        // The main thread's cache should still be None (unchanged by the other thread)
        WMI_CACHE.with(|cell| {
            assert!(
                cell.borrow().is_none(),
                "Main thread cache should be unaffected"
            );
        });
    }

    #[cfg(windows)]
    #[test]
    fn test_with_wmi_transient_error_preserves_cache() {
        // First, try to initialize the cache with a successful query
        let init_result = with_wmi(|_conn| Ok::<(), anyhow::Error>(()));
        if init_result.is_err() {
            eprintln!("WMI not available in this environment, skipping test");
            return;
        }

        // Verify cache is initialized
        WMI_CACHE.with(|cell| {
            assert!(
                cell.borrow().is_some(),
                "Cache should be initialized after success"
            );
        });

        // Run a query that returns a transient error
        let result =
            with_wmi(|_conn| Err::<(), anyhow::Error>(anyhow::anyhow!("transient query error")));
        assert!(result.is_err(), "Transient error should be returned");

        // The cache should still be initialized (not invalidated for transient errors)
        WMI_CACHE.with(|cell| {
            assert!(
                cell.borrow().is_some(),
                "Cache should be preserved after transient error"
            );
        });
    }

    #[cfg(windows)]
    #[test]
    fn test_with_wmi_connection_error_invalidates_cache() {
        // First, try to initialize the cache
        let init_result = with_wmi(|_conn| Ok::<(), anyhow::Error>(()));
        if init_result.is_err() {
            eprintln!("WMI not available in this environment, skipping test");
            return;
        }

        // Run a query that returns a connection-level error
        let result = with_wmi(|_conn| {
            Err::<(), anyhow::Error>(anyhow::Error::from(HardwareError::WmiConnection(
                "forced connection error".to_string(),
            )))
        });
        assert!(result.is_err(), "Connection error should be returned");

        // The cache should be invalidated after a connection error
        WMI_CACHE.with(|cell| {
            assert!(
                cell.borrow().is_none(),
                "Cache should be invalidated after connection error"
            );
        });
    }
}
