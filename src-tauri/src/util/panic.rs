//! Panic recovery utilities.
//!
//! Provides:
//! - `lock_or_recover`: recover from poisoned mutexes instead of panicking
//! - `install_panic_hook`: global panic hook that logs to file and stderr

use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Lock a `Mutex`, recovering from poison instead of panicking.
///
/// When a mutex is poisoned (a previous holder panicked while holding the lock),
/// `Mutex::lock().unwrap()` would propagate the panic. This helper instead
/// recovers the inner data via `into_inner()`, allowing the app to continue
/// running with potentially-stale but usable state.
///
/// # Example
/// ```ignore
/// let guard = lock_or_recover(&state.performance_mode);
/// *guard = new_mode;
/// ```
pub fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| {
        log::error!("Mutex poisoned, recovering with inner data: {}", e);
        e.into_inner()
    })
}

/// Lock a `RwLock` for reading, recovering from poison instead of panicking.
///
/// Similar to `lock_or_recover` but for `RwLock` read guards.
pub fn lock_read_or_recover<T>(rwlock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    rwlock.read().unwrap_or_else(|e| {
        log::error!("RwLock (read) poisoned, recovering with inner data: {}", e);
        e.into_inner()
    })
}

/// Lock a `RwLock` for writing, recovering from poison instead of panicking.
///
/// Similar to `lock_or_recover` but for `RwLock` write guards.
pub fn lock_write_or_recover<T>(rwlock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    rwlock.write().unwrap_or_else(|e| {
        log::error!("RwLock (write) poisoned, recovering with inner data: {}", e);
        e.into_inner()
    })
}

/// Install a global panic hook that logs the panic to both stderr and the
/// application log file.
///
/// This should be called once at startup (in `lib.rs::run()` or `main.rs`).
/// The hook captures the panic payload, location, and a backtrace, then
/// delegates to the previous hook (which typically aborts or unwinds).
pub fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info.location();
        let payload = info.payload();
        let payload_str = if let Some(s) = payload.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "Box<dyn Any>".to_string()
        };

        let loc_str = location
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());

        // Log to the application logger (fern → file + stderr)
        log::error!("PANIC at {}: {}", loc_str, payload_str);

        // Also print to stderr directly in case the logger isn't initialized
        eprintln!("PANIC at {}: {}", loc_str, payload_str);

        // Delegate to the default hook for standard behavior (unwind/abort)
        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_lock_or_recover_normal() {
        let m = Mutex::new(42);
        let guard = lock_or_recover(&m);
        assert_eq!(*guard, 42);
    }

    #[test]
    fn test_lock_or_recover_after_poison() {
        let m = Arc::new(Mutex::new(10));
        let m2 = Arc::clone(&m);

        // Poison the mutex by panicking while holding the lock
        let handle = thread::spawn(move || {
            let _guard = m2.lock().unwrap();
            panic!("intentional poison");
        });
        let _ = handle.join();

        // lock_or_recover should recover instead of panicking
        let guard = lock_or_recover(&m);
        assert_eq!(*guard, 10); // The inner value is still accessible
    }

    #[test]
    fn test_install_panic_hook_does_not_panic() {
        // Just verify it can be called without error
        install_panic_hook();
    }
}
