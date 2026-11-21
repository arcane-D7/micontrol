//! Universal persisted-state store (UNIVERSAL-CONTRACTS.md section 3).
//!
//! One template for every state file the app or its services persist. Each
//! store is declared as a `StoreSpec` (name, resolved path, schema version,
//! ACL policy, size ceiling) and owned by a `Store<T>` that guarantees:
//!
//! - atomic writes: content is written to a sibling `.tmp` file, flushed to
//!   disk, then swapped into place with `ReplaceFileW` (or `rename` when the
//!   target does not exist yet). A process killed between the tmp write and
//!   the swap leaves the previous file intact and the tmp file is ignored on
//!   load, so no partial state is ever observed.
//! - versioned payloads: every file carries `{"schema": N, "value": ...}`.
//!   A schema older than the declared one runs the store's declared
//!   migrator; a schema newer than the declared one is refused
//!   (`StoreError::SchemaNewer`), never reinterpreted.
//! - locked read-modify-write: `update` takes an exclusive lock on a sibling
//!   `.lock` file (`LockFileEx` through `fs2`) with a bounded wait, so
//!   concurrent updaters never lose writes.
//! - purge as a first-class, idempotent operation.
//! - explicit ACLs for machine-wide stores: `System` and `SystemAndAdmins`
//!   folders and files get a protected DACL (inheritance broken) so the
//!   inherited `%PROGRAMDATA%` "Users can write" ACL (GSEC-02 class) can
//!   never reach them.
//!
//! Path resolution lives ONLY in `StoreRegistry`: modules never choose their
//! own state paths. `%LOCALAPPDATA%\MiControl\state\<name>.json` for user
//! stores, `%PROGRAMDATA%\MiControl\<dotted name>.json` for system stores
//! (dots create subdirectories: `governor.policy` becomes
//! `governor\policy.json`).
//!
//! The three C0 pilots live at the bottom of this file:
//! `brightness` (with legacy `brightness_override.json` migration), `fan`
//! (new preference) and `governor.policy` (SystemAndAdmins, not yet used by
//! the app). They are declarations only; wiring existing call sites into
//! them is the C3 migration phase.

use fs2::FileExt;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Maximum time `update` waits for the exclusive state lock before giving up.
const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_millis(2000);

/// Short lock wait used by `purge`; purge is a teardown op, it never queues
/// behind a long update.
const PURGE_LOCK_WAIT: Duration = Duration::from_millis(250);

/// `ERROR_LOCK_VIOLATION`: how LockFileEx (through fs2) reports contention.
#[cfg(windows)]
const ERROR_LOCK_VIOLATION: i32 = 33;

/// Default size ceiling for pilot stores; state payloads here are tiny.
pub const DEFAULT_STATE_MAX_BYTES: u64 = 64 * 1024;

/// Error surface of the state store. Every variant is actionable by the
/// caller: `Corrupt` and `SchemaNewer` mean "do not trust the file",
/// `TooLarge` means "the payload outgrew its declared ceiling", `Io` is
/// transport-level and `LockTimeout` is retryable.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The file exists but its payload cannot be trusted (bad JSON, missing
    /// schema field, undeclared schema version, value shape mismatch).
    #[error("state payload is corrupt at {path}: {reason}")]
    Corrupt { path: PathBuf, reason: String },

    /// Filesystem or ACL failure while reading/writing the store.
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// Serialized payload or on-disk file exceeds the declared ceiling.
    #[error("state payload too large: {size} bytes exceeds max {max} bytes ({path})")]
    TooLarge { size: u64, max: u64, path: PathBuf },

    /// The file was written by a build that declares a NEWER schema than
    /// this process knows. Never reinterpret; surface to the user as an
    /// upgrade requirement.
    #[error("state schema {found} is newer than supported {current} ({path})")]
    SchemaNewer {
        found: u32,
        current: u32,
        path: PathBuf,
    },

    /// Another writer held the exclusive lock for longer than the budget.
    #[error("state lock acquisition timed out after {waited_ms} ms ({path})")]
    LockTimeout { waited_ms: u64, path: PathBuf },
}

/// Who may access the store.
///
/// - `User`: per-user data under `%LOCALAPPDATA%`; the inherited per-user
///   ACL is already private, no explicit DACL is applied.
/// - `System`: machine data writable by SYSTEM only.
/// - `SystemAndAdmins`: machine data writable by SYSTEM, readable by
///   Administrators. Inheritance is broken so the insecure `%PROGRAMDATA%`
///   inherited ACL (Users:Write, GSEC-02 class) can never reach the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AclPolicy {
    User,
    System,
    SystemAndAdmins,
}

/// Readback of a file or folder DACL, used to verify that a restricted ACL
/// was really applied (no state is trusted without the readback).
#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaclSummary {
    /// `SE_DACL_PROTECTED` is set: the DACL does not inherit from the parent.
    pub protected: bool,
    /// SID string of every access-allowed ACE in the DACL, in DACL order.
    pub ace_sids: Vec<String>,
}

/// Declared shape of one persisted state file. Built through
/// `StoreRegistry`, never by the owning module (paths are a registry
/// decision so ACL and location rules stay in one place).
#[derive(Debug, Clone)]
pub struct StoreSpec {
    /// Stable store name, e.g. `brightness` or `governor.policy`.
    pub name: &'static str,
    /// Absolute file path, resolved by the registry.
    pub path: PathBuf,
    /// Current schema version this process writes and reads.
    pub schema: u32,
    /// Access policy applied to the folder and the file.
    pub acl: AclPolicy,
    /// Payload ceiling; anything above is refused before it reaches disk.
    pub max_bytes: u64,
}

/// Wire envelope around every persisted value. The `schema` field is the
/// migration key; a payload without it is corrupt by definition (the store
/// never guesses).
#[derive(Serialize, Deserialize)]
struct Envelope {
    schema: u32,
    value: serde_json::Value,
}

/// A declared migration: converts a payload of schema version `from` into
/// the store's current type. Plain function pointer so `Store` stays
/// `Send + Sync` and migrators stay declarative.
pub type MigrateFn<T> = fn(u32, &serde_json::Value) -> Result<T, StoreError>;

/// Typed handle over one persisted state file.
pub struct Store<T: Serialize + DeserializeOwned> {
    spec: StoreSpec,
    lock_timeout: Duration,
    migrator: Option<MigrateFn<T>>,
}

impl<T: Serialize + DeserializeOwned> Store<T> {
    /// Build a store from a registry-resolved spec.
    pub fn new(spec: StoreSpec) -> Self {
        Self {
            spec,
            lock_timeout: DEFAULT_LOCK_TIMEOUT,
            migrator: None,
        }
    }

    /// Declare the migration used when a payload carries an older schema.
    pub fn with_migrator(mut self, migrator: MigrateFn<T>) -> Self {
        self.migrator = Some(migrator);
        self
    }

    /// Override the lock wait budget for `update`.
    pub fn with_lock_timeout(mut self, timeout: Duration) -> Self {
        self.lock_timeout = timeout;
        self
    }

    /// The registry-resolved spec behind this store.
    pub fn spec(&self) -> &StoreSpec {
        &self.spec
    }

    /// Resolved file path of this store.
    pub fn path(&self) -> &Path {
        &self.spec.path
    }

    /// Load the persisted value. `Ok(None)` means "no state yet" (file
    /// absent). Older schemas run the declared migrator; newer schemas are
    /// refused. Corrupt payloads produce `StoreError::Corrupt`, never a
    /// panic and never a silent default.
    pub fn load(&self) -> Result<Option<T>, StoreError> {
        let Some(raw) = self.read_raw_file()? else {
            return Ok(None);
        };
        let text = String::from_utf8_lossy(&raw);
        let envelope: Envelope =
            serde_json::from_str(&text).map_err(|e| self.corrupt(format!("bad envelope: {e}")))?;
        if envelope.value.is_null() {
            // A null value is never a valid state payload for the concrete
            // state types this store carries; refuse instead of handing the
            // caller an Option-inside-Option confusion.
            return Err(self.corrupt("envelope value is null".into()));
        }
        match envelope.schema.cmp(&self.spec.schema) {
            std::cmp::Ordering::Greater => Err(StoreError::SchemaNewer {
                found: envelope.schema,
                current: self.spec.schema,
                path: self.spec.path.clone(),
            }),
            std::cmp::Ordering::Equal => serde_json::from_value(envelope.value)
                .map(Some)
                .map_err(|e| self.corrupt(format!("value does not match schema: {e}"))),
            std::cmp::Ordering::Less => {
                let migrator = self.migrator.ok_or_else(|| {
                    self.corrupt(format!(
                        "schema {} has no declared migration path to {}",
                        envelope.schema, self.spec.schema
                    ))
                })?;
                migrator(envelope.schema, &envelope.value)
                    .map(Some)
                    .map_err(|e| match e {
                        // Keep the store path in the error even when the
                        // migrator reports a generic failure.
                        StoreError::Corrupt { reason, .. } => self.corrupt(reason),
                        other => other,
                    })
            }
        }
    }

    /// Persist a value atomically. The caller does not need the lock for a
    /// single-shot write (last writer wins with an atomic swap); read
    /// -modify-write cycles MUST go through `update`.
    pub fn save(&self, value: &T) -> Result<(), StoreError> {
        let envelope = Envelope {
            schema: self.spec.schema,
            value: serde_json::to_value(value)
                .map_err(|e| self.io_other(format!("serialize failed: {e}")))?,
        };
        let json = serde_json::to_string(&envelope)
            .map_err(|e| self.io_other(format!("serialize envelope failed: {e}")))?;
        let bytes = json.as_bytes();
        let size = bytes.len() as u64;
        if size > self.spec.max_bytes {
            return Err(StoreError::TooLarge {
                size,
                max: self.spec.max_bytes,
                path: self.spec.path.clone(),
            });
        }
        self.ensure_dir()?;
        write_atomic(&self.spec.path, bytes)?;
        if self.spec.acl != AclPolicy::User {
            // The folder already carries the protected DACL with inheritable
            // ACEs, so the renamed file starts with the right inherited ACL;
            // the explicit file DACL pins it against future folder drift.
            apply_acl(&self.spec.path, self.spec.acl)?;
        }
        Ok(())
    }

    /// Locked read-modify-write. Absent state is seeded from `T::default()`.
    /// The closure runs while the exclusive file lock is held; concurrent
    /// updaters serialize and never lose writes.
    pub fn update<F>(&self, f: F) -> Result<(), StoreError>
    where
        F: FnOnce(&mut T),
        T: Default,
    {
        // The lock file lives beside the state file; the folder must exist
        // before either can be opened.
        self.ensure_dir()?;
        let _lock = self.acquire_lock(self.lock_timeout)?;
        let mut value = self.load()?.unwrap_or_default();
        f(&mut value);
        self.save(&value)
    }

    /// Remove the data file, the tmp file and the lock file. Idempotent:
    /// purging a store that does not exist succeeds. Machine-scoped stores
    /// purge only from a context the DACL allows (SYSTEM).
    pub fn purge(&self) -> Result<(), StoreError> {
        let Some(parent) = self.spec.path.parent() else {
            return Ok(());
        };
        if !parent.exists() {
            // The whole folder is gone: nothing to purge.
            return Ok(());
        }
        // Take the lock briefly so a concurrent update finishes first; if it
        // times out, report rather than racing a live writer.
        let lock = match self.acquire_lock(PURGE_LOCK_WAIT) {
            Ok(lock) => Some(lock),
            Err(StoreError::LockTimeout { waited_ms, path }) => {
                return Err(StoreError::LockTimeout { waited_ms, path })
            }
            Err(other) => return Err(other),
        };
        let result = (|| -> Result<(), StoreError> {
            remove_if_exists(&self.tmp_path())?;
            remove_if_exists(&self.spec.path)?;
            Ok(())
        })();
        // Drop the lock (and delete the lock file) only after the data files
        // are gone; the lock handle is closed before the lock file removal.
        drop(lock);
        remove_if_exists(&self.lock_path())?;
        result
    }

    // ---- internals ------------------------------------------------------

    fn read_raw_file(&self) -> Result<Option<Vec<u8>>, StoreError> {
        let meta = match fs::metadata(&self.spec.path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(self.io_err(e)),
        };
        if meta.len() > self.spec.max_bytes {
            return Err(StoreError::TooLarge {
                size: meta.len(),
                max: self.spec.max_bytes,
                path: self.spec.path.clone(),
            });
        }
        let bytes = fs::read(&self.spec.path).map_err(|e| self.io_err(e))?;
        if bytes.len() as u64 > self.spec.max_bytes {
            // The file grew between metadata and read; still refuse.
            return Err(StoreError::TooLarge {
                size: bytes.len() as u64,
                max: self.spec.max_bytes,
                path: self.spec.path.clone(),
            });
        }
        Ok(Some(bytes))
    }

    fn ensure_dir(&self) -> Result<(), StoreError> {
        if let Some(dir) = self.spec.path.parent() {
            StoreRegistry::ensure_dir(dir, self.spec.acl)?;
        }
        Ok(())
    }

    fn tmp_path(&self) -> PathBuf {
        file_sibling(&self.spec.path, "tmp")
    }

    fn lock_path(&self) -> PathBuf {
        file_sibling(&self.spec.path, "lock")
    }

    fn corrupt(&self, reason: String) -> StoreError {
        log::warn!(target: "util::state_store", "corrupt state at {}: {}", self.spec.path.display(), reason);
        StoreError::Corrupt {
            path: self.spec.path.clone(),
            reason,
        }
    }

    fn io_err(&self, e: io::Error) -> StoreError {
        StoreError::Io {
            path: self.spec.path.clone(),
            source: e,
        }
    }

    fn io_other(&self, msg: String) -> StoreError {
        StoreError::Io {
            path: self.spec.path.clone(),
            source: io::Error::other(msg),
        }
    }

    fn acquire_lock(&self, timeout: Duration) -> Result<FileLock, StoreError> {
        FileLock::acquire(&self.lock_path(), timeout)
    }
}

/// Sibling path of `file` in the same directory with the given extension
/// appended: `brightness.json` + `tmp` becomes `brightness.json.tmp`.
fn file_sibling(file: &Path, suffix: &str) -> PathBuf {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut sibling = file.to_path_buf();
    sibling.set_file_name(format!("{name}.{suffix}"));
    sibling
}

fn remove_if_exists(path: &Path) -> Result<(), StoreError> {
    if path.exists() {
        fs::remove_file(path).map_err(|e| StoreError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
    }
    Ok(())
}

/// Atomic write: tmp + fsync + swap. A kill before the swap leaves the
/// previous target intact and a harmless tmp orphan behind; the tmp file is
/// never read by `load` and is cleaned by the next save or purge.
fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let tmp = file_sibling(target, "tmp");
    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir).map_err(|e| StoreError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
    }
    {
        let mut file = fs::File::create(&tmp).map_err(|e| StoreError::Io {
            path: tmp.clone(),
            source: e,
        })?;
        file.write_all(bytes).map_err(|e| StoreError::Io {
            path: tmp.clone(),
            source: e,
        })?;
        // fsync before the swap: after the swap the content must be on disk,
        // otherwise a power loss can leave the target pointing at nothing.
        file.sync_all().map_err(|e| StoreError::Io {
            path: tmp.clone(),
            source: e,
        })?;
    }
    if let Err(e) = swap_tmp_into_place(&tmp, target) {
        // Best-effort cleanup; the next save rewrites the tmp anyway.
        let _ = fs::remove_file(&tmp);
        return Err(StoreError::Io {
            path: target.to_path_buf(),
            source: e,
        });
    }
    Ok(())
}

/// Swap the fully-written tmp file into the target position. `ReplaceFileW`
/// preserves the target's attributes and ACL; when the target does not exist
/// (or ReplaceFileW fails) a plain rename does the same atomic move on the
/// same volume.
fn swap_tmp_into_place(tmp: &Path, target: &Path) -> io::Result<()> {
    if target.exists() {
        #[cfg(windows)]
        match replace_file_w(tmp, target) {
            Ok(()) => return Ok(()),
            Err(e) => {
                log::warn!(target: "util::state_store", "ReplaceFileW failed ({e}), falling back to rename for {}", target.display());
            }
        }
        #[cfg(not(windows))]
        let _ = ();
    }
    fs::rename(tmp, target)
}

#[cfg(windows)]
fn replace_file_w(replacement: &Path, replaced: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        ReplaceFileW, REPLACEFILE_IGNORE_MERGE_ERRORS, REPLACEFILE_WRITE_THROUGH,
        REPLACE_FILE_FLAGS,
    };

    let replaced_w: Vec<u16> = replaced
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let replacement_w: Vec<u16> = replacement
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        ReplaceFileW(
            PCWSTR(replaced_w.as_ptr()),
            PCWSTR(replacement_w.as_ptr()),
            PCWSTR::null(),
            REPLACE_FILE_FLAGS(REPLACEFILE_WRITE_THROUGH.0 | REPLACEFILE_IGNORE_MERGE_ERRORS.0),
            None,
            None,
        )
        .map_err(|e| io::Error::other(format!("ReplaceFileW: {e}")))
    }
}

/// Exclusive lock over one store, held across `update`'s read-modify-write.
/// Backed by `fs2` (LockFileEx on Windows) with a bounded wait; unlocking
/// happens on drop.
struct FileLock {
    _file: fs::File,
}

impl FileLock {
    fn acquire(lock_path: &Path, timeout: Duration) -> Result<FileLock, StoreError> {
        let started = Instant::now();
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(|e| StoreError::Io {
                path: lock_path.to_path_buf(),
                source: e,
            })?;
        let mut attempts = 0u32;
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(FileLock { _file: file }),
                // Contended: retry until the wait budget runs out. fs2 on
                // Windows surfaces LockFileEx contention as raw error 33
                // (ERROR_LOCK_VIOLATION, mapped to `Uncategorized`), not
                // `WouldBlock`.
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.raw_os_error() == Some(ERROR_LOCK_VIOLATION) => {}
                Err(e) => {
                    return Err(StoreError::Io {
                        path: lock_path.to_path_buf(),
                        source: e,
                    })
                }
            }
            if started.elapsed() >= timeout {
                return Err(StoreError::LockTimeout {
                    waited_ms: started.elapsed().as_millis() as u64,
                    path: lock_path.to_path_buf(),
                });
            }
            attempts += 1;
            if attempts <= 50 {
                // A competing writer releases and re-acquires in microseconds
                // between its own updates; sleeping past that window starves
                // this acquisition forever. Spin first.
                std::thread::yield_now();
            } else {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        // Best-effort release; the OS drops the lock with the handle anyway.
        let _ = self._file.unlock();
    }
}

/// `ACE_HEADER.AceType` value for access-allowed ACEs (`ACCESS_ALLOWED_ACE_TYPE`
/// in the Windows SDK; the windows crate keeps it in a feature this crate does
/// not enable, so mirror the constant locally).
#[cfg(windows)]
const ACE_TYPE_ACCESS_ALLOWED: u8 = 0;

// ---- ACL application (Windows) -------------------------------------------

/// Apply the declared ACL policy to an existing file or folder.
///
/// `System` and `SystemAndAdmins` break DACL inheritance (protected DACL) so
/// the insecure inherited `%PROGRAMDATA%` ACL (Users:Write, GSEC-02 class)
/// cannot reach the store. The folder variant carries OICI inheritance flags
/// so the ACEs reach children; the file variant is the SDDL from the C0c
/// contract verbatim. `User` leaves the inherited per-user ACL untouched.
///
/// Fail-closed: an ACL failure is an error, never a silent "saved anyway".
pub fn apply_acl(path: &Path, policy: AclPolicy) -> Result<(), StoreError> {
    match policy {
        AclPolicy::User => Ok(()),
        AclPolicy::System => {
            let sddl = if path.is_dir() {
                "D:(A;OICI;FA;;;SY)"
            } else {
                "D:(A;;FA;;;SY)"
            };
            apply_sddl(path, sddl)
        }
        AclPolicy::SystemAndAdmins => {
            let sddl = if path.is_dir() {
                "D:(A;OICI;FA;;;SY)(A;OICI;FR;;;BA)"
            } else {
                "D:(A;;FA;;;SY)(A;;FR;;;BA)"
            };
            apply_sddl(path, sddl)
        }
    }
}

/// The ACE SIDs a correctly applied folder DACL must contain, in SDDL order.
#[cfg(windows)]
fn expected_ace_sids(policy: AclPolicy) -> Vec<String> {
    match policy {
        AclPolicy::User => Vec::new(),
        AclPolicy::System => vec![SYSTEM_SID.to_string()],
        AclPolicy::SystemAndAdmins => vec![SYSTEM_SID.to_string(), ADMINS_SID.to_string()],
    }
}

#[cfg(windows)]
const SYSTEM_SID: &str = "S-1-5-18";
#[cfg(windows)]
const ADMINS_SID: &str = "S-1-5-32-544";

#[cfg(windows)]
fn apply_sddl(path: &Path, sddl: &str) -> Result<(), StoreError> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, ERROR_SUCCESS, HLOCAL};
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW,
        SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{
        GetSecurityDescriptorDacl, ACL, DACL_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    };

    let acl_error = |msg: String| -> StoreError {
        StoreError::Io {
            path: path.to_path_buf(),
            source: io::Error::new(io::ErrorKind::PermissionDenied, msg),
        }
    };
    let path_w: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let sddl_w: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut psd = PSECURITY_DESCRIPTOR(std::ptr::null_mut());
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl_w.as_ptr()),
            SDDL_REVISION_1,
            &mut psd,
            None,
        )
        .map_err(|e| acl_error(format!("SDDL conversion failed for {sddl}: {e}")))?;
        if psd.0.is_null() {
            return Err(acl_error("SDDL conversion returned null descriptor".into()));
        }
        // Extract the DACL INSIDE the descriptor: SetNamedSecurityInfoW wants
        // the ACL pointer, not the descriptor pointer (passing the descriptor
        // itself silently produced an empty DACL).
        let mut dacl_present = windows::Win32::Foundation::BOOL::default();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut dacl_defaulted = windows::Win32::Foundation::BOOL::default();
        GetSecurityDescriptorDacl(psd, &mut dacl_present, &mut dacl, &mut dacl_defaulted)
            .map_err(|e| acl_error(format!("GetSecurityDescriptorDacl failed: {e}")))?;
        if dacl_present.as_bool() && dacl.is_null() {
            let _ = LocalFree(HLOCAL(psd.0.cast()));
            return Err(acl_error(format!("SDDL {sddl} produced no DACL")));
        }
        let result = SetNamedSecurityInfoW(
            PCWSTR(path_w.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(dacl),
            None,
        );
        let _ = LocalFree(HLOCAL(psd.0.cast()));
        if result != ERROR_SUCCESS {
            return Err(acl_error(format!(
                "SetNamedSecurityInfoW failed with error {}",
                result.0
            )));
        }
    }
    Ok(())
}

/// Read back the DACL of a file or folder: whether inheritance is broken
/// (protected) and which ACE trustees it grants. Used by the registry to
/// skip redundant ACL writes and by tests to verify the GSEC-02 property.
#[cfg(windows)]
pub fn read_dacl_summary(path: &Path) -> Result<DaclSummary, StoreError> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{LocalFree, ERROR_SUCCESS, HLOCAL};
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{
        AclSizeInformation, GetAce, GetAclInformation, GetSecurityDescriptorControl,
        ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED, SID,
    };

    let acl_error = |msg: String| -> StoreError {
        StoreError::Io {
            path: path.to_path_buf(),
            source: io::Error::new(io::ErrorKind::PermissionDenied, msg),
        }
    };
    let path_w: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let mut psd = PSECURITY_DESCRIPTOR(std::ptr::null_mut());
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let result = GetNamedSecurityInfoW(
            PCWSTR(path_w.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut dacl),
            None,
            &mut psd,
        );
        if result != ERROR_SUCCESS {
            let _ = LocalFree(HLOCAL(psd.0.cast()));
            return Err(acl_error(format!(
                "GetNamedSecurityInfoW failed with error {}",
                result.0
            )));
        }

        let mut control = 0u16;
        let mut revision = 0u32;
        let protected = match GetSecurityDescriptorControl(psd, &mut control, &mut revision) {
            Ok(()) => control & SE_DACL_PROTECTED.0 != 0,
            Err(e) => {
                let _ = LocalFree(HLOCAL(psd.0.cast()));
                return Err(acl_error(format!(
                    "GetSecurityDescriptorControl failed: {e}"
                )));
            }
        };

        let mut ace_sids = Vec::new();
        if !dacl.is_null() {
            let mut info = ACL_SIZE_INFORMATION::default();
            if let Err(e) = GetAclInformation(
                dacl.cast(),
                (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            ) {
                let _ = LocalFree(HLOCAL(psd.0.cast()));
                return Err(acl_error(format!("GetAclInformation failed: {e}")));
            }
            for index in 0..info.AceCount {
                let mut header: *mut core::ffi::c_void = std::ptr::null_mut();
                if let Err(e) = GetAce(dacl, index, &mut header) {
                    let _ = LocalFree(HLOCAL(psd.0.cast()));
                    return Err(acl_error(format!("GetAce({index}) failed: {e}")));
                }
                let ace = header as *const ACCESS_ALLOWED_ACE;
                if (*ace).Header.AceType != ACE_TYPE_ACCESS_ALLOWED {
                    continue;
                }
                let sid = std::ptr::addr_of!((*ace).SidStart) as *const SID;
                let mut sid_text = PWSTR::null();
                if let Err(e) =
                    ConvertSidToStringSidW(PSID(sid as *mut core::ffi::c_void), &mut sid_text)
                {
                    let _ = LocalFree(HLOCAL(psd.0.cast()));
                    return Err(acl_error(format!("ConvertSidToStringSidW failed: {e}")));
                }
                let sid_string = String::from_utf16_lossy(sid_text.as_wide());
                ace_sids.push(sid_string.trim_end_matches('\0').to_string());
                let _ = LocalFree(HLOCAL(sid_text.0.cast()));
            }
        }
        let _ = LocalFree(HLOCAL(psd.0.cast()));
        Ok(DaclSummary {
            protected,
            ace_sids,
        })
    }
}

#[cfg(not(windows))]
pub fn apply_acl(_path: &Path, _policy: AclPolicy) -> Result<(), StoreError> {
    Ok(())
}

// ---- StoreRegistry --------------------------------------------------------

/// Which root a store lives under. Resolution lives here and nowhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreRoot {
    /// `%LOCALAPPDATA%`: per-user state, stored under `MiControl\state`.
    User,
    /// `%PROGRAMDATA%`: machine-wide state, stored under `MiControl`.
    ProgramData,
}

pub struct StoreRegistry;

impl StoreRegistry {
    /// Production root for a store class (read from the environment).
    ///
    /// The user root can be redirected wholesale through the
    /// `MICONTROL_STATE_ROOT` environment variable. Production never sets it;
    /// tests and support tooling use it to keep state files away from the
    /// real user profile without touching each store's declared spec.
    pub fn root(kind: StoreRoot) -> Result<PathBuf, StoreError> {
        if kind == StoreRoot::User {
            if let Some(root) = std::env::var_os("MICONTROL_STATE_ROOT") {
                if !root.is_empty() {
                    return Ok(PathBuf::from(root));
                }
            }
        }
        let var = match kind {
            StoreRoot::User => "LOCALAPPDATA",
            StoreRoot::ProgramData => "PROGRAMDATA",
        };
        std::env::var_os(var)
            .map(PathBuf::from)
            .ok_or_else(|| StoreError::Io {
                path: PathBuf::from(var),
                source: io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("environment variable {var} is not set"),
                ),
            })
    }

    /// Resolve a store file path under an explicit root (tests, services).
    /// Dots in the name create subdirectories: `governor.policy` becomes
    /// `<root>\MiControl\governor\policy.json`.
    pub fn resolve_in(kind: StoreRoot, root: &Path, name: &str) -> PathBuf {
        let base = match kind {
            StoreRoot::User => root.join("MiControl").join("state"),
            StoreRoot::ProgramData => root.join("MiControl"),
        };
        let mut path = base;
        for segment in name.split('.') {
            path.push(segment);
        }
        path.set_extension("json");
        path
    }

    /// Resolve a store file path from the production roots.
    pub fn resolve(kind: StoreRoot, name: &str) -> Result<PathBuf, StoreError> {
        Ok(Self::resolve_in(kind, &Self::root(kind)?, name))
    }

    /// Build a fully resolved `StoreSpec` from the production roots.
    pub fn spec(
        kind: StoreRoot,
        name: &'static str,
        schema: u32,
        acl: AclPolicy,
        max_bytes: u64,
    ) -> Result<StoreSpec, StoreError> {
        let path = Self::resolve(kind, name)?;
        Ok(StoreSpec {
            name,
            path,
            schema,
            acl,
            max_bytes,
        })
    }

    /// Build a fully resolved `StoreSpec` under an explicit root (tests).
    pub fn spec_in(
        kind: StoreRoot,
        root: &Path,
        name: &'static str,
        schema: u32,
        acl: AclPolicy,
        max_bytes: u64,
    ) -> StoreSpec {
        StoreSpec {
            name,
            path: Self::resolve_in(kind, root, name),
            schema,
            acl,
            max_bytes,
        }
    }

    /// Guarantee the folder exists and carries the right ACL. Restricted
    /// policies get a protected DACL (inheritance broken) with inheritable
    /// ACEs; the readback gate keeps repeated saves cheap and honest.
    pub fn ensure_dir(dir: &Path, acl: AclPolicy) -> Result<(), StoreError> {
        fs::create_dir_all(dir).map_err(|e| StoreError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
        if acl == AclPolicy::User {
            return Ok(());
        }
        #[cfg(windows)]
        {
            if let Ok(summary) = read_dacl_summary(dir) {
                if summary.protected && summary.ace_sids == expected_ace_sids(acl) {
                    return Ok(());
                }
            }
        }
        apply_acl(dir, acl)
    }
}

// ---- C0 pilots ------------------------------------------------------------

// Pilot 1: brightness manual-override state (mirrors hw::display's
// OverrideState; hw::display migrates onto this store in phase C3).

/// Where a stored brightness offset came from (same semantics as the
/// adaptive loop's `OffsetOrigin`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BrightnessOffsetOrigin {
    /// Captured automatically by the adaptive loop; also the default for
    /// files written before the user/captured split.
    #[default]
    Captured,
    /// Derived from an explicit user action (slider, OSD, hotkey).
    User,
}

impl BrightnessOffsetOrigin {
    /// Wire code for the origin (`0` captured, `1` user). Mirrors the
    /// display module's `OffsetOrigin` numbering.
    pub fn as_u8(self) -> u8 {
        match self {
            Self::Captured => 0,
            Self::User => 1,
        }
    }
}

/// Manual-override state persisted for crash/relaunch survival.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BrightnessOverrideState {
    /// Wall-clock ms (since UNIX epoch) of the last manual brightness change.
    pub last_manual_ms: u64,
    /// Signed curve offset the user's manual value implies.
    pub offset: i16,
    /// Whether the offset is active (user has expressed a preference).
    pub offset_active: bool,
    /// Where the offset came from; decides the clamp applied on restore.
    #[serde(default)]
    pub origin: BrightnessOffsetOrigin,
}

/// Schema 0 migrator: schema 0 carried the raw legacy payload shape (no
/// envelope-era transforms), so v0 -> v1 is a re-envelope only.
fn migrate_brightness_v0(
    _from: u32,
    value: &serde_json::Value,
) -> Result<BrightnessOverrideState, StoreError> {
    serde_json::from_value(value.clone()).map_err(|e| StoreError::Corrupt {
        path: PathBuf::from("brightness"),
        reason: format!("legacy brightness payload does not parse: {e}"),
    })
}

/// Production brightness store: `%LOCALAPPDATA%\MiControl\state\brightness.json`.
pub fn brightness_store() -> Result<Store<BrightnessOverrideState>, StoreError> {
    Ok(Store::new(StoreRegistry::spec(
        StoreRoot::User,
        "brightness",
        1,
        AclPolicy::User,
        DEFAULT_STATE_MAX_BYTES,
    )?)
    .with_migrator(migrate_brightness_v0))
}

/// Brightness store under an explicit root (tests, services).
pub fn brightness_store_in(root: &Path) -> Store<BrightnessOverrideState> {
    Store::new(StoreRegistry::spec_in(
        StoreRoot::User,
        root,
        "brightness",
        1,
        AclPolicy::User,
        DEFAULT_STATE_MAX_BYTES,
    ))
    .with_migrator(migrate_brightness_v0)
}

/// Legacy file written by `hw::display` before the state store existed.
pub fn legacy_brightness_override_path() -> PathBuf {
    if let Ok(override_path) = std::env::var("MICONTROL_BRIGHTNESS_OVERRIDE_PATH") {
        return PathBuf::from(override_path);
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("MiControl").join("brightness_override.json")
}

/// Legacy file for a given LOCALAPPDATA root (tests).
fn legacy_brightness_override_path_in(root: &Path) -> PathBuf {
    root.join("MiControl").join("brightness_override.json")
}

/// Import the legacy `brightness_override.json` into the new store.
///
/// Reads the legacy file if it exists, converts it and saves it under the
/// new path. The legacy file is KEPT until the next successful `save` of the
/// new store removes it, so a crash mid-migration leaves both copies valid.
/// Returns `Ok(false)` when there is nothing to do (no legacy file, or the
/// new store already has state; existing state is never overwritten).
pub fn migrate_legacy_brightness_override_in(
    root: &Path,
    legacy_path: &Path,
) -> Result<bool, StoreError> {
    let text = match fs::read_to_string(legacy_path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => {
            return Err(StoreError::Io {
                path: legacy_path.to_path_buf(),
                source: e,
            })
        }
    };
    let legacy: BrightnessOverrideState =
        serde_json::from_str(&text).map_err(|e| StoreError::Corrupt {
            path: legacy_path.to_path_buf(),
            reason: format!("legacy brightness_override.json does not parse: {e}"),
        })?;
    let store = brightness_store_in(root);
    if store.load()?.is_some() {
        return Ok(false);
    }
    store.save(&legacy)?;
    log::info!(target: "util::state_store", "migrated legacy brightness_override.json into the state store");
    Ok(true)
}

/// Production convenience wrapper for the legacy import.
pub fn migrate_legacy_brightness_override() -> Result<bool, StoreError> {
    let root = StoreRegistry::root(StoreRoot::User)?;
    let legacy = legacy_brightness_override_path();
    migrate_legacy_brightness_override_in(&root, &legacy)
}

/// Retire the legacy `brightness_override.json` after the store save
/// succeeded ("keep the old file until the next save"). Best-effort: a
/// removal failure only logs, the next save retries.
fn retire_legacy_brightness_override_at(legacy: &Path) {
    if legacy.exists() {
        if let Err(e) = fs::remove_file(legacy) {
            log::warn!(target: "util::state_store", "could not remove legacy brightness_override.json: {e}");
        }
    }
}

/// Save the brightness override through the store and retire the legacy
/// file on success ("keep the old file until the next save").
pub fn save_brightness_override_in(
    root: &Path,
    value: &BrightnessOverrideState,
) -> Result<(), StoreError> {
    let store = brightness_store_in(root);
    store.save(value)?;
    retire_legacy_brightness_override_at(&legacy_brightness_override_path_in(root));
    Ok(())
}

/// Production convenience wrapper for the brightness save. Honors the same
/// environment overrides as the load side (`MICONTROL_STATE_ROOT` for the
/// store root, `MICONTROL_BRIGHTNESS_OVERRIDE_PATH` for the legacy file) so
/// tests exercise exactly the production code path.
pub fn save_brightness_override(value: &BrightnessOverrideState) -> Result<(), StoreError> {
    let root = StoreRegistry::root(StoreRoot::User)?;
    let store = brightness_store_in(&root);
    store.save(value)?;
    retire_legacy_brightness_override_at(&legacy_brightness_override_path());
    Ok(())
}

// Pilot 4 (C3a): smart brightness learning model. The payload type
// (`hw::display::SmartModel`) is owned by the display module; the store only
// fixes the location, schema version and ceiling. The legacy-file migration
// is driven by the owning module because the payload semantics (Welford
// decay pruning) live there too.

/// Production smart-model store spec:
/// `%LOCALAPPDATA%\MiControl\state\smart_model.json`, schema 1, user ACL.
pub fn smart_model_spec() -> Result<StoreSpec, StoreError> {
    StoreRegistry::spec(
        StoreRoot::User,
        "smart_model",
        1,
        AclPolicy::User,
        DEFAULT_STATE_MAX_BYTES,
    )
}

/// Smart-model store spec under an explicit user root (tests, services).
pub fn smart_model_spec_in(root: &Path) -> StoreSpec {
    StoreRegistry::spec_in(
        StoreRoot::User,
        root,
        "smart_model",
        1,
        AclPolicy::User,
        DEFAULT_STATE_MAX_BYTES,
    )
}

/// Legacy file written by `hw::display` before the state store existed (the
/// learned smart brightness model). Consumed once by the display module's
/// migration; retired by the next successful store save.
pub fn legacy_smart_model_path() -> PathBuf {
    if let Ok(path) = std::env::var("MICONTROL_SMART_MODEL_PATH") {
        return PathBuf::from(path);
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("MiControl").join("smart_brightness.json")
}

/// Legacy smart-model file for a given LOCALAPPDATA root (tests pin the
/// resolution; the production read goes through `legacy_smart_model_path`).
#[cfg(test)]
fn legacy_smart_model_path_in(root: &Path) -> PathBuf {
    root.join("MiControl").join("smart_brightness.json")
}

/// Remove the legacy `smart_brightness.json` after the store has taken over.
/// Best-effort: a failure only logs; the next successful save retries the
/// retirement.
pub fn retire_legacy_smart_model() {
    let legacy = legacy_smart_model_path();
    if legacy.exists() {
        if let Err(e) = fs::remove_file(legacy) {
            log::warn!(target: "util::state_store", "could not remove legacy smart_brightness.json: {e}");
        }
    }
}

// Pilot 2: preferred fan profile (new; no UI yet).

/// Preferred fan profile persisted across sessions.
///
/// The Windows registry (`hw::fan`'s HKLM values) remains the source of truth
/// for the LIVE fan preference. This store carries the last-known-good
/// snapshot of the last successfully applied mode so the fan tab can recover
/// it when the registry cannot be read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FanProfileState {
    /// Preferred profile name (for example `silence`, `balance`, `turbo`,
    /// `smart`); empty string means unset.
    pub profile: String,
    /// Wall-clock ms (since UNIX epoch) of the last change.
    pub updated_at_ms: u64,
    /// Last-known-good fan mode as its registry code (`0` auto, `1` fixed,
    /// `2` off); `None` while the snapshot carries no mode.
    #[serde(default)]
    pub last_mode_code: Option<u8>,
    /// Last-known-good requested speed percent (20-100); `None` while unset.
    #[serde(default)]
    pub last_speed_percent: Option<u8>,
}

impl FanProfileState {
    pub fn new(profile: impl Into<String>) -> Self {
        Self {
            profile: profile.into(),
            updated_at_ms: now_ms(),
            last_mode_code: None,
            last_speed_percent: None,
        }
    }
}

/// Production fan store: `%LOCALAPPDATA%\MiControl\state\fan.json`.
pub fn fan_store() -> Result<Store<FanProfileState>, StoreError> {
    Ok(Store::new(StoreRegistry::spec(
        StoreRoot::User,
        "fan",
        1,
        AclPolicy::User,
        DEFAULT_STATE_MAX_BYTES,
    )?))
}

/// Fan store under an explicit root (tests, services).
pub fn fan_store_in(root: &Path) -> Store<FanProfileState> {
    Store::new(StoreRegistry::spec_in(
        StoreRoot::User,
        root,
        "fan",
        1,
        AclPolicy::User,
        DEFAULT_STATE_MAX_BYTES,
    ))
}

// Pilot 3: governor policy (SystemAndAdmins; not yet used by the app).

/// Governor policy state. The runtime contract (schema validation of
/// watchdog payloads, signed author fields) belongs to the governor wave;
/// this store only fixes the location and the ACL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GovernorPolicyState {
    /// Whether the governor is enabled by policy.
    pub enabled: bool,
    /// Wall-clock ms (since UNIX epoch) of the last change.
    pub updated_at_ms: u64,
}

/// Production governor policy store:
/// `%PROGRAMDATA%\MiControl\governor\policy.json` with a protected
/// SYSTEM-full / Administrators-read DACL. Writers are SYSTEM services; a
/// non-elevated user process that tries to save fails closed.
pub fn governor_policy_store() -> Result<Store<GovernorPolicyState>, StoreError> {
    Ok(Store::new(StoreRegistry::spec(
        StoreRoot::ProgramData,
        "governor.policy",
        1,
        AclPolicy::SystemAndAdmins,
        DEFAULT_STATE_MAX_BYTES,
    )?))
}

/// Governor policy store under an explicit PROGRAMDATA root (tests).
pub fn governor_policy_store_in(program_root: &Path) -> Store<GovernorPolicyState> {
    Store::new(StoreRegistry::spec_in(
        StoreRoot::ProgramData,
        program_root,
        "governor.policy",
        1,
        AclPolicy::SystemAndAdmins,
        DEFAULT_STATE_MAX_BYTES,
    ))
}

// ---- C3b stores ------------------------------------------------------------

/// Remove a legacy state file after the store has taken over its content.
/// Callers must only invoke this AFTER a successful store save, so a failure
/// here never loses data; it leaves the legacy file in place and logs.
pub fn retire_legacy_state_file(legacy: &Path) {
    if let Err(e) = fs::remove_file(legacy) {
        if e.kind() != io::ErrorKind::NotFound {
            log::warn!(
                target: "util::state_store",
                "could not remove legacy state file {}: {e}",
                legacy.display()
            );
        }
    }
}

// Store: os_turbo_log. Anonymous bounded OS Turbo action log. The payload
// type (`ActionLogState`) lives in `hw::os_turbo_log` next to `ActionRecord`;
// only the location decision is made here. Ceiling: the log caps itself at
// 256 KiB of records; the envelope overhead fits inside 512 KiB.

/// Production os_turbo_log spec: `%LOCALAPPDATA%\MiControl\state\os_turbo_log.json`.
pub fn os_turbo_log_spec() -> Result<StoreSpec, StoreError> {
    StoreRegistry::spec(
        StoreRoot::User,
        "os_turbo_log",
        1,
        AclPolicy::User,
        512 * 1024,
    )
}

/// os_turbo_log spec under an explicit root (tests).
pub fn os_turbo_log_spec_in(root: &Path) -> StoreSpec {
    StoreRegistry::spec_in(
        StoreRoot::User,
        root,
        "os_turbo_log",
        1,
        AclPolicy::User,
        512 * 1024,
    )
}

// Store: face_consent. The DPAPI-encrypted enrollment gallery shared by the
// app (writes) and the SYSTEM face service (reads, LogonUI authentication).
// Machine scope (`StoreRoot::ProgramData`) is REQUIRED: the service answers
// at the logon screen before any user session exists. The ACL stays untouched
// (`User` policy = inherit), matching the legacy `faces.dat` behavior; the
// ProgramData ACL question remains SEC-04, owned elsewhere.

/// Opaque gallery payload: the DPAPI machine-scope blob, base64-encoded so it
/// travels inside the JSON envelope byte-exact. The plaintext never touches
/// the envelope; the encryption boundary stays in `hw::face::store`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FaceGalleryState {
    /// Base64 of the DPAPI-protected gallery blob (magic + encrypted JSON).
    pub blob_b64: String,
}

/// Production face_consent store: `%PROGRAMDATA%\MiControl\face_consent.json`.
pub fn face_consent_store() -> Result<Store<FaceGalleryState>, StoreError> {
    Ok(Store::new(StoreRegistry::spec(
        StoreRoot::ProgramData,
        "face_consent",
        1,
        AclPolicy::User,
        1024 * 1024,
    )?))
}

/// face_consent store under an explicit PROGRAMDATA root (tests).
pub fn face_consent_store_in(program_root: &Path) -> Store<FaceGalleryState> {
    Store::new(StoreRegistry::spec_in(
        StoreRoot::ProgramData,
        program_root,
        "face_consent",
        1,
        AclPolicy::User,
        1024 * 1024,
    ))
}

/// Legacy enrollment gallery written by pre-store builds (app AND service).
pub fn legacy_face_gallery_path() -> PathBuf {
    match std::env::var_os("PROGRAMDATA") {
        Some(base) => legacy_face_gallery_path_in(&PathBuf::from(base)),
        None => PathBuf::from(r"C:\ProgramData\MiControl\face\faces.dat"),
    }
}

/// Legacy gallery path under an explicit PROGRAMDATA root (tests).
pub fn legacy_face_gallery_path_in(program_root: &Path) -> PathBuf {
    program_root
        .join("MiControl")
        .join("face")
        .join("faces.dat")
}

// Store: charging. The user's requested charging limit. Replaces the HKLM
// `ChargingThreshold` cache, which a non-elevated app could not even write;
// the legacy registry value is imported read-only on first load.

/// Requested charging limit persisted for crash survival and as the fallback
/// when the firmware read fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ChargingThresholdState {
    /// Requested limit in percent (clamped to 40..=100 by the caller).
    pub threshold: u8,
}

/// Production charging store: `%LOCALAPPDATA%\MiControl\state\charging.json`.
pub fn charging_store() -> Result<Store<ChargingThresholdState>, StoreError> {
    Ok(Store::new(StoreRegistry::spec(
        StoreRoot::User,
        "charging",
        1,
        AclPolicy::User,
        DEFAULT_STATE_MAX_BYTES,
    )?))
}

/// Charging store under an explicit root (tests).
pub fn charging_store_in(root: &Path) -> Store<ChargingThresholdState> {
    Store::new(StoreRegistry::spec_in(
        StoreRoot::User,
        root,
        "charging",
        1,
        AclPolicy::User,
        DEFAULT_STATE_MAX_BYTES,
    ))
}

// Store: eye_protection. Enabled flag + intensity for the gamma-ramp filter.
// Replaces the HKCU `EyeProtection` registry values.

/// Eye protection persisted state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EyeProtectionState {
    /// Whether the warm gamma ramp is currently applied.
    pub enabled: bool,
    /// 0-100 filter intensity (0 = no filter).
    pub intensity: u8,
}

/// Production eye_protection store:
/// `%LOCALAPPDATA%\MiControl\state\eye_protection.json`.
pub fn eye_protection_store() -> Result<Store<EyeProtectionState>, StoreError> {
    Ok(Store::new(StoreRegistry::spec(
        StoreRoot::User,
        "eye_protection",
        1,
        AclPolicy::User,
        DEFAULT_STATE_MAX_BYTES,
    )?))
}

/// eye_protection store under an explicit root (tests).
pub fn eye_protection_store_in(root: &Path) -> Store<EyeProtectionState> {
    Store::new(StoreRegistry::spec_in(
        StoreRoot::User,
        root,
        "eye_protection",
        1,
        AclPolicy::User,
        DEFAULT_STATE_MAX_BYTES,
    ))
}

// Store: audio_effects. The three saved-but-not-applied audio preferences.
// Each field stays `Option<bool>` so "never set" remains distinguishable from
// "explicitly off": the legacy gap-fill migration only fills never-set slots
// and must not overwrite an explicit off with a stranded SYSTEM-hive value.

/// Audio effects persisted state (`None` = the user never set the toggle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AudioEffectsState {
    /// Mic noise canceling preference.
    #[serde(default)]
    pub mic_noise_canceling: Option<bool>,
    /// Speaker noise canceling preference.
    #[serde(default)]
    pub speaker_noise_canceling: Option<bool>,
    /// Voice focus preference.
    #[serde(default)]
    pub voice_focus: Option<bool>,
}

/// Production audio_effects store:
/// `%LOCALAPPDATA%\MiControl\state\audio_effects.json`.
pub fn audio_effects_store() -> Result<Store<AudioEffectsState>, StoreError> {
    Ok(Store::new(StoreRegistry::spec(
        StoreRoot::User,
        "audio_effects",
        1,
        AclPolicy::User,
        DEFAULT_STATE_MAX_BYTES,
    )?))
}

/// audio_effects store under an explicit root (tests).
pub fn audio_effects_store_in(root: &Path) -> Store<AudioEffectsState> {
    Store::new(StoreRegistry::spec_in(
        StoreRoot::User,
        root,
        "audio_effects",
        1,
        AclPolicy::User,
        DEFAULT_STATE_MAX_BYTES,
    ))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    use std::sync::Arc;

    /// Unique per-test scratch root; safe under parallel test threads.
    fn scratch_root(tag: &str) -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, AtomicOrdering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "micontrol-state-store-{}-{}-{}",
            tag,
            std::process::id(),
            seq
        ));
        fs::create_dir_all(&dir).expect("create scratch root");
        dir
    }

    fn cleanup(root: &Path) {
        let _ = fs::remove_dir_all(root);
    }

    fn write_raw(path: &Path, bytes: &[u8]) {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).expect("create fixture dir");
        }
        fs::write(path, bytes).expect("write fixture");
    }

    fn plain_user_store(root: &Path) -> Store<serde_json::Value> {
        Store::new(StoreRegistry::spec_in(
            StoreRoot::User,
            root,
            "generic",
            1,
            AclPolicy::User,
            DEFAULT_STATE_MAX_BYTES,
        ))
    }

    #[test]
    fn save_then_load_roundtrip() {
        let root = scratch_root("roundtrip");
        let store = plain_user_store(&root);
        assert_eq!(store.load().expect("load empty"), None);
        let value = serde_json::json!({ "answer": 42 });
        store.save(&value).expect("save");
        assert_eq!(store.load().expect("load"), Some(value));
        cleanup(&root);
    }

    #[test]
    fn atomic_write_survives_leftover_tmp() {
        let root = scratch_root("atomic-tmp");
        let store = plain_user_store(&root);
        let first = serde_json::json!({ "v": 1 });
        store.save(&first).expect("save first");
        // Simulate a process killed between the tmp write and the swap: the
        // tmp file holds partial garbage, the target must stay intact.
        write_raw(
            &store.path().with_file_name("generic.json.tmp"),
            b"{\"schema\":1,\"valu",
        );
        assert_eq!(store.load().expect("load ignores tmp"), Some(first));
        // The next save replaces the tmp and the target cleanly.
        let second = serde_json::json!({ "v": 2 });
        store.save(&second).expect("save second");
        assert_eq!(store.load().expect("load after save"), Some(second));
        assert!(
            !store.path().with_file_name("generic.json.tmp").exists(),
            "tmp cleaned"
        );
        cleanup(&root);
    }

    #[test]
    fn corrupt_files_report_corrupt_without_panic() {
        let root = scratch_root("corrupt");
        let cases: Vec<&[u8]> = vec![
            b"not json at all",
            b"",
            b"42",
            b"{\"schema\":1}",
            b"{\"schema\":1,\"value\":",
            b"\xff\xfe\x00\x01garbage",
            b"{\"value\":{\"a\":1}}",
            b"{\"schema\":1,\"value\":null}",
        ];
        for payload in cases {
            let store = plain_user_store(&root);
            write_raw(store.path(), payload);
            match store.load() {
                Err(StoreError::Corrupt { .. }) => {}
                other => panic!("expected Corrupt for {payload:?}, got {other:?}"),
            }
        }
        cleanup(&root);
    }

    #[test]
    fn missing_file_loads_none() {
        let root = scratch_root("missing");
        let store = plain_user_store(&root);
        assert_eq!(store.load().expect("missing file"), None);
        cleanup(&root);
    }

    #[test]
    fn older_schema_runs_declared_migrator() {
        let root = scratch_root("migrate");
        let store = Store::new(StoreRegistry::spec_in(
            StoreRoot::User,
            &root,
            "versioned",
            2,
            AclPolicy::User,
            DEFAULT_STATE_MAX_BYTES,
        ))
        .with_migrator(|_from, value| {
            // v1 stored {"n": number}; v2 stores {"count": number}.
            let n = value.get("n").and_then(|v| v.as_u64()).unwrap_or(0);
            Ok(serde_json::json!({ "count": n }))
        });
        write_raw(store.path(), br#"{"schema":1,"value":{"n":7}}"#);
        assert_eq!(
            store.load().expect("migrated load"),
            Some(serde_json::json!({ "count": 7 }))
        );
        cleanup(&root);
    }

    #[test]
    fn older_schema_without_migrator_is_corrupt() {
        let root = scratch_root("migrate-none");
        let store = plain_user_store(&root);
        write_raw(store.path(), br#"{"schema":0,"value":{}}"#);
        match store.load() {
            Err(StoreError::Corrupt { .. }) => {}
            other => panic!("expected Corrupt for undeclared migration, got {other:?}"),
        }
        cleanup(&root);
    }

    #[test]
    fn newer_schema_refused() {
        let root = scratch_root("schema-newer");
        let store = plain_user_store(&root);
        write_raw(store.path(), br#"{"schema":99,"value":{}}"#);
        match store.load() {
            Err(StoreError::SchemaNewer { found, current, .. }) => {
                assert_eq!(found, 99);
                assert_eq!(current, 1);
            }
            other => panic!("expected SchemaNewer, got {other:?}"),
        }
        cleanup(&root);
    }

    #[test]
    fn oversized_payload_refused_on_save() {
        let root = scratch_root("too-large-save");
        let spec = StoreRegistry::spec_in(StoreRoot::User, &root, "tiny", 1, AclPolicy::User, 32);
        let store = Store::<serde_json::Value>::new(spec);
        let value = serde_json::json!({ "payload": "0123456789abcdef0123456789abcdef0123456789" });
        match store.save(&value) {
            Err(StoreError::TooLarge { size, max, .. }) => {
                assert!(size > max, "reported size must exceed max");
                assert!(!store.path().exists(), "nothing was written");
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
        cleanup(&root);
    }

    #[test]
    fn oversized_file_refused_on_load() {
        let root = scratch_root("too-large-load");
        let spec = StoreRegistry::spec_in(StoreRoot::User, &root, "tiny", 1, AclPolicy::User, 16);
        let store = Store::<serde_json::Value>::new(spec);
        write_raw(store.path(), br#"{"schema":1,"value":{"a":"b"}}"#);
        match store.load() {
            Err(StoreError::TooLarge { .. }) => {}
            other => panic!("expected TooLarge, got {other:?}"),
        }
        cleanup(&root);
    }

    #[test]
    fn purge_is_idempotent_and_removes_everything() {
        let root = scratch_root("purge");
        let store = plain_user_store(&root);
        store.save(&serde_json::json!({ "v": 1 })).expect("save");
        assert!(store.path().exists());
        store.purge().expect("purge with content");
        assert_eq!(store.load().expect("after purge"), None);
        assert!(!store.path().exists(), "data file removed");
        assert!(
            !store.path().with_file_name("generic.json.lock").exists(),
            "lock removed"
        );
        assert!(
            !store.path().with_file_name("generic.json.tmp").exists(),
            "tmp removed"
        );
        // Purging an already-empty store succeeds again.
        store.purge().expect("purge idempotent");
        store.purge().expect("purge idempotent twice");
        // A store keeps working after a purge.
        store
            .save(&serde_json::json!({ "v": 2 }))
            .expect("save after purge");
        assert_eq!(
            store.load().expect("reload"),
            Some(serde_json::json!({ "v": 2 }))
        );
        cleanup(&root);
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
    struct Counter {
        count: u32,
    }

    #[test]
    fn concurrent_updates_do_not_lose_writes() {
        let root = scratch_root("concurrent");
        // Generous lock budget: this test asserts no lost writes, not lock
        // latency. The suite runs parallel with fsync-heavy tests, so the
        // default 2 s budget can be exceeded by legitimate scheduling.
        let store = Arc::new(
            Store::new(StoreRegistry::spec_in(
                StoreRoot::User,
                &root,
                "counter",
                1,
                AclPolicy::User,
                DEFAULT_STATE_MAX_BYTES,
            ))
            .with_lock_timeout(Duration::from_secs(30)),
        );
        let mut handles = Vec::new();
        for _thread in 0..2 {
            let store = Arc::clone(&store);
            handles.push(std::thread::spawn(move || {
                for _ in 0..50 {
                    store
                        .update(|c: &mut Counter| c.count += 1)
                        .expect("concurrent update");
                }
            }));
        }
        for handle in handles {
            handle.join().expect("writer thread");
        }
        let final_value = store.load().expect("final load").expect("state present");
        assert_eq!(final_value.count, 100, "no lost updates");
        cleanup(&root);
    }

    #[test]
    fn lock_timeout_when_an_update_holds_the_lock() {
        let root = scratch_root("lock-timeout");
        let holder = Store::new(StoreRegistry::spec_in(
            StoreRoot::User,
            &root,
            "counter",
            1,
            AclPolicy::User,
            DEFAULT_STATE_MAX_BYTES,
        ));
        let contender = Store::new(StoreRegistry::spec_in(
            StoreRoot::User,
            &root,
            "counter",
            1,
            AclPolicy::User,
            DEFAULT_STATE_MAX_BYTES,
        ))
        .with_lock_timeout(Duration::from_millis(80));

        // Signal from INSIDE the update closure: by the time it fires the
        // exclusive lock is already held, so the contender's timeout below is
        // deterministic instead of a scheduling race.
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            holder
                .update(|c: &mut Counter| {
                    locked_tx.send(()).ok();
                    std::thread::sleep(Duration::from_millis(400));
                    c.count += 1;
                })
                .expect("holder update");
        });
        locked_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("holder signals while holding the lock");
        match contender.update(|c: &mut Counter| c.count += 1) {
            Err(StoreError::LockTimeout { .. }) => {}
            other => panic!("expected LockTimeout, got {other:?}"),
        }
        writer.join().expect("holder finishes");
        // The lock is released when the holder finishes; a retry works.
        contender
            .update(|c: &mut Counter| c.count += 1)
            .expect("retry after release");
        let value = contender.load().expect("load").expect("state");
        assert_eq!(value.count, 2);
        cleanup(&root);
    }

    #[test]
    fn registry_resolves_pilot_paths() {
        let user_root = PathBuf::from("C:/fake-user-root");
        let program_root = PathBuf::from("C:/fake-program-root");
        assert_eq!(
            StoreRegistry::resolve_in(StoreRoot::User, &user_root, "brightness"),
            user_root
                .join("MiControl")
                .join("state")
                .join("brightness.json")
        );
        assert_eq!(
            StoreRegistry::resolve_in(StoreRoot::User, &user_root, "fan"),
            user_root.join("MiControl").join("state").join("fan.json")
        );
        assert_eq!(
            StoreRegistry::resolve_in(StoreRoot::ProgramData, &program_root, "governor.policy"),
            program_root
                .join("MiControl")
                .join("governor")
                .join("policy.json")
        );
    }

    #[test]
    fn brightness_legacy_file_migrates_and_retires_on_save() {
        let root = scratch_root("legacy-brightness");
        // Legacy file in the pre-store format: no envelope, no origin field
        // (old builds never wrote one, the default must apply).
        let legacy_path = legacy_brightness_override_path_in(&root);
        write_raw(
            &legacy_path,
            br#"{"last_manual_ms":1700000000000,"offset":5,"offset_active":true}"#,
        );
        let migrated =
            migrate_legacy_brightness_override_in(&root, &legacy_path).expect("legacy migration");
        assert!(migrated, "legacy file was migrated");
        let store = brightness_store_in(&root);
        let loaded = store
            .load()
            .expect("load migrated state")
            .expect("state present");
        assert_eq!(loaded.last_manual_ms, 1_700_000_000_000);
        assert_eq!(loaded.offset, 5);
        assert!(loaded.offset_active);
        assert_eq!(loaded.origin, BrightnessOffsetOrigin::Captured);
        // The legacy file is kept until the next save.
        assert!(legacy_path.exists(), "legacy file kept after migration");
        // Next save retires the legacy file.
        save_brightness_override_in(
            &root,
            &BrightnessOverrideState {
                last_manual_ms: 1_700_000_005_000,
                offset: -3,
                offset_active: true,
                origin: BrightnessOffsetOrigin::User,
            },
        )
        .expect("save through pilot");
        assert!(!legacy_path.exists(), "legacy file retired by next save");
        let reloaded = store.load().expect("reload").expect("state");
        assert_eq!(reloaded.origin, BrightnessOffsetOrigin::User);
        assert_eq!(reloaded.offset, -3);
        cleanup(&root);
    }

    #[test]
    fn brightness_migration_is_noop_when_new_state_exists() {
        let root = scratch_root("legacy-noop");
        let store = brightness_store_in(&root);
        store
            .save(&BrightnessOverrideState {
                last_manual_ms: 1,
                offset: 0,
                offset_active: false,
                origin: BrightnessOffsetOrigin::User,
            })
            .expect("seed new state");
        let legacy_path = legacy_brightness_override_path_in(&root);
        write_raw(
            &legacy_path,
            br#"{"last_manual_ms":9,"offset":9,"offset_active":true}"#,
        );
        let migrated =
            migrate_legacy_brightness_override_in(&root, &legacy_path).expect("noop migration");
        assert!(!migrated, "existing state must not be overwritten");
        let loaded = store.load().expect("load").expect("state");
        assert_eq!(loaded.last_manual_ms, 1, "new state preserved");
        assert!(legacy_path.exists(), "legacy untouched on noop");
        cleanup(&root);
    }

    #[test]
    fn brightness_store_migrates_schema0_envelope_payload() {
        let root = scratch_root("brightness-v0");
        let store = brightness_store_in(&root);
        write_raw(
            store.path(),
            br#"{"schema":0,"value":{"last_manual_ms":123,"offset":4,"offset_active":true}}"#,
        );
        let loaded = store.load().expect("v0 load").expect("state");
        assert_eq!(loaded.last_manual_ms, 123);
        assert_eq!(loaded.offset, 4);
        assert!(loaded.offset_active);
        assert_eq!(loaded.origin, BrightnessOffsetOrigin::Captured);
        cleanup(&root);
    }

    #[test]
    fn fan_store_seeds_default_and_updates() {
        let root = scratch_root("fan");
        let store = fan_store_in(&root);
        assert_eq!(store.load().expect("empty fan store"), None);
        store
            .update(|fan: &mut FanProfileState| *fan = FanProfileState::new("silence"))
            .expect("seed via update");
        let loaded = store.load().expect("load").expect("state");
        assert_eq!(loaded.profile, "silence");
        store
            .update(|fan: &mut FanProfileState| fan.profile = "turbo".into())
            .expect("second update");
        let reloaded = store.load().expect("reload").expect("state");
        assert_eq!(reloaded.profile, "turbo");
        cleanup(&root);
    }

    /// C3 static gate (e): the migrated persistence functions in `hw::display`
    /// and `hw::fan` delegate to the state store; they never write the state
    /// files directly. Only production code is inspected (tests write fixtures
    /// freely and the state-writes gate ignores `#[cfg(test)]` sections).
    #[test]
    fn display_and_fan_state_writes_go_through_the_store() {
        for (file, source, fns) in [
            (
                "hw/display.rs",
                include_str!("../hw/display.rs"),
                &[
                    "persist_override_state(",
                    "restore_override_state(",
                    "load_smart_model(",
                    "persist_smart_model(",
                    "reset_smart_model(",
                ] as &[&str],
            ),
            (
                "hw/fan.rs",
                include_str!("../hw/fan.rs"),
                &[
                    "persist_fan_registry(",
                    "persist_fan_last_known_good(",
                    "persist_fan_last_known_good_in(",
                    "fan_last_known_good(",
                    "fan_last_known_good_from(",
                ],
            ),
        ] {
            // Production code only: everything from the tests module on is
            // test code, which may write fixtures freely. Split on the
            // `#[cfg(test)] mod tests` marker so inline `#[cfg(test)]`
            // hygiene guards inside production functions stay in scope.
            let production = source
                .split("#[cfg(test)]\nmod tests")
                .next()
                .expect("source text");
            for signature in fns {
                // Top-level definitions only (private `fn` or `pub fn`); a
                // bare call site inside another function never matches.
                let start = ["\nfn ", "\npub fn "]
                    .iter()
                    .find_map(|prefix| production.find(&format!("{prefix}{signature}")))
                    .unwrap_or_else(|| panic!("{file}: expected fn {signature} to exist"));
                let body = &production[start..];
                let end = body.find("\n}").unwrap_or(body.len());
                let body = &body[..end];
                assert!(
                    !body.contains("fs::write(") && !body.contains("File::create("),
                    "{file}: {signature} must persist through the state store, not write the state file directly"
                );
            }
        }
    }

    /// The C3a smart-model pilot resolves through the registry like the
    /// brightness/fan pilots (same root, same extension rules).
    #[test]
    fn smart_model_store_resolves_registry_paths() {
        let user_root = PathBuf::from("C:/fake-user-root");
        let spec = smart_model_spec_in(&user_root);
        assert_eq!(spec.name, "smart_model");
        assert_eq!(spec.schema, 1);
        assert_eq!(spec.acl, AclPolicy::User);
        assert_eq!(
            spec.path,
            user_root
                .join("MiControl")
                .join("state")
                .join("smart_model.json")
        );
        // The legacy file the display module imports from.
        assert_eq!(
            legacy_smart_model_path_in(&user_root),
            user_root.join("MiControl").join("smart_brightness.json")
        );
    }

    #[cfg(windows)]
    #[test]
    fn protected_dacls_break_inheritance() {
        let root = scratch_root("acl");
        let dir = root.join("protected");
        let file = dir.join("data.json");
        fs::create_dir_all(&dir).expect("create dir");
        write_raw(&file, b"{}");

        apply_acl(&dir, AclPolicy::SystemAndAdmins).expect("folder ACL");
        let summary = read_dacl_summary(&dir).expect("folder summary");
        assert!(summary.protected, "folder inheritance must be broken");
        assert_eq!(
            summary.ace_sids,
            vec!["S-1-5-18".to_string(), "S-1-5-32-544".to_string()],
            "folder DACL must be exactly SYSTEM full + Administrators read"
        );

        apply_acl(&file, AclPolicy::SystemAndAdmins).expect("file ACL");
        let summary = read_dacl_summary(&file).expect("file summary");
        assert!(summary.protected, "file inheritance must be broken");
        assert_eq!(
            summary.ace_sids,
            vec!["S-1-5-18".to_string(), "S-1-5-32-544".to_string()],
            "file DACL must be exactly SYSTEM full + Administrators read"
        );

        // Restore owner control so the scratch tree can be deleted.
        restore_owner_control(&dir);
        restore_owner_control(&file);
        cleanup(&root);
    }

    #[cfg(windows)]
    #[test]
    fn system_only_folder_rejects_writes_and_governor_store_stays_locked_out() {
        let root = scratch_root("governor-acl");
        let store = governor_policy_store_in(&root);
        // ensure_dir with the SystemAndAdmins policy applies the protected
        // DACL to the folder before any write is attempted.
        let dir = store.path().parent().expect("policy folder").to_path_buf();
        fs::create_dir_all(&dir).expect("pre-create");
        StoreRegistry::ensure_dir(&dir, AclPolicy::SystemAndAdmins).expect("ensure dir");
        let summary = read_dacl_summary(&dir).expect("folder summary");
        assert!(summary.protected, "governor folder inheritance broken");
        assert_eq!(
            summary.ace_sids,
            vec!["S-1-5-18".to_string(), "S-1-5-32-544".to_string()],
            "governor folder DACL is SYSTEM full + Administrators read"
        );

        // A user-context writer cannot create files in the folder (SYSTEM is
        // the writer by design); the save must fail closed, not silently.
        let save_result = store.save(&GovernorPolicyState {
            enabled: true,
            updated_at_ms: 1,
        });
        match save_result {
            Err(StoreError::Io { source, .. })
                if source.kind() == io::ErrorKind::PermissionDenied
                    || source.raw_os_error() == Some(5) =>
            {
                // Expected: the DACL blocks the non-SYSTEM writer.
            }
            other => panic!("expected access-denied Io error, got {other:?}"),
        }
        assert!(!store.path().exists(), "nothing was written by the user");

        restore_owner_control(&dir);
        cleanup(&root);
    }

    /// Restore full control for the current owner (SDDL OWNER_RIGHTS) so the
    /// test can delete a tree whose DACL it intentionally locked itself out
    /// of. The owner always keeps WRITE_DAC, so this works without elevation.
    #[cfg(windows)]
    fn restore_owner_control(path: &Path) {
        let result = super::apply_sddl(path, "D:(A;OICI;FA;;;OW)");
        if let Err(e) = result {
            panic!("could not restore owner control on {}: {e}", path.display());
        }
    }

    /// C3b static gate: the modules migrated onto the state store must not
    /// write state files directly anymore. Only the production prefix of each
    /// file is checked: the `#[cfg(test)]` sections create fixtures
    /// legitimately and the state-writes ratchet already ignores them. The
    /// tokens below are assembled dynamically so this source file itself
    /// never contains a literal match for the ratchet's patterns.
    #[test]
    fn migrated_modules_have_no_direct_state_writes() {
        let fs_write = concat!("fs:", ":write(");
        let file_create = concat!("File", "::create(");
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let migrated = [
            "src/hw/audio_effects.rs",
            "src/hw/eye_protection.rs",
            "src/hw/charging.rs",
            "src/hw/os_turbo_log.rs",
            "src/hw/face/store.rs",
            "src/bin/micontrol_face_svc.rs",
        ];
        for rel in migrated {
            let path = manifest.join(rel);
            let text =
                std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {rel}: {e}"));
            let marker = concat!("#[cfg", "(test)]");
            let production = text.split(marker).next().expect("prefix");
            assert!(
                !production.contains(fs_write) && !production.contains(file_create),
                "{rel} still writes files directly; route it through util::state_store"
            );
        }
    }

    /// C3b static gate: each migrated module actually consumes the state
    /// store (or the shared face gallery helpers that wrap it).
    #[test]
    fn migrated_modules_reference_the_state_store() {
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let adoption = [
            ("src/hw/audio_effects.rs", "state_store"),
            ("src/hw/eye_protection.rs", "state_store"),
            ("src/hw/charging.rs", "state_store"),
            ("src/hw/os_turbo_log.rs", "state_store"),
            ("src/hw/face/store.rs", "state_store"),
            ("src/bin/micontrol_face_svc.rs", "load_gallery"),
        ];
        for (rel, marker) in adoption {
            let text = std::fs::read_to_string(manifest.join(rel))
                .unwrap_or_else(|e| panic!("cannot read {rel}: {e}"));
            assert!(
                text.contains(marker),
                "{rel} does not reference {marker}; the migration regressed"
            );
        }
    }
}
