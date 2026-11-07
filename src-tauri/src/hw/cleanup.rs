//! System cleanup — junk file scanning and removal.
//!
//! Provides a functional equivalent to XPM's "System Cleanup" feature
//! using standard Windows APIs for temp/cache file cleanup.
//!
//! ## Execution context is part of the contract (audit F2.1)
//!
//! `scan_junk_files` and the user-side cleanup run **inside the app process**,
//! which runs as the interactive user — so `TEMP`, `LOCALAPPDATA`, `APPDATA`
//! and the Recycle Bin all resolve to the user's own profile. That is the
//! profile the UI measures.
//!
//! It did not always: the whole cleanup used to be dispatched through
//! `elev_bridge::run_elevated`, whose normal path is the `MiControlBridge`
//! service running as **LocalSystem**. There `TEMP` resolves to
//! `C:\Windows\Temp` and `LOCALAPPDATA` to the system profile's — 4 of the 6
//! categories cleaned a directory the user never saw (and that was usually
//! empty), while the UI announced "cleanup complete" over the user's 1.5 GB.
//!
//! The split is now explicit and enforced by the API:
//!
//! * [`clean_user_junk`] — user-owned categories, **unprivileged**, in-process.
//! * [`clean_system_junk`] — the two machine-wide categories, **elevated**
//!   through the bridge, addressed by absolute paths only (never by env vars,
//!   which would resolve to the SYSTEM profile again).
//!
//! `commands::system::clean_junk_files` partitions a request and calls the
//! right side for each category, so a caller cannot accidentally run a
//! user-owned category as SYSTEM.
//!
//! Supported cleanup categories:
//! - User Temp (%TEMP%)
//! - Windows Update cache (C:\Windows\SoftwareDistribution\Download)
//! - Browser caches (Chrome, Edge, Firefox)
//! - Recycle Bin
//! - Thumbnail cache
//! - Windows log files

use crate::hw::errors::{HardwareError, HardwareResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Hard cap on the number of per-category error strings kept for the UI.
///
/// A category can fail thousands of times (e.g. every file in a locked cache
/// directory); the UI only needs enough to explain *why*.
const MAX_RECORDED_ERRORS: usize = 20;

/// Cleanup category identifier.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CleanupCategory {
    /// The interactive user's `%TEMP%` (the same directory the scan measures).
    WindowsTemp,
    /// C:\Windows\SoftwareDistribution\Download
    WindowsUpdateCache,
    /// Browser caches (Chrome, Edge, Firefox)
    BrowserCache,
    /// Recycle Bin of the interactive user
    RecycleBin,
    /// Thumbnail cache
    ThumbnailCache,
    /// Windows log files
    WindowsLogs,
}

/// Information about a cleanup category.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupItem {
    pub category: CleanupCategory,
    /// Human-readable description
    pub description: String,
    /// Size in bytes that can be freed
    pub size_bytes: u64,
    /// Number of files that would be removed
    pub file_count: u64,
}

/// Result of a cleanup operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupResult {
    pub category: CleanupCategory,
    /// Bytes freed
    pub freed_bytes: u64,
    /// Files removed
    pub files_removed: u64,
    /// Files that could not be removed (locked, in use)
    pub files_skipped: u64,
    /// Errors encountered (capped at [`MAX_RECORDED_ERRORS`])
    pub errors: Vec<String>,
}

impl CleanupResult {
    /// A result that reports a category-level failure instead of pretending the
    /// category was cleaned (bridge down, bad contract, refused context).
    pub fn failure(category: CleanupCategory, message: impl Into<String>) -> Self {
        Self {
            category,
            freed_bytes: 0,
            files_removed: 0,
            files_skipped: 0,
            errors: vec![message.into()],
        }
    }
}

/// Categories whose paths resolve to the interactive user's profile.
///
/// These MUST run in the app process (or any process owned by that user) —
/// never in the LocalSystem bridge.
pub const ALL_USER: [CleanupCategory; 4] = [
    CleanupCategory::WindowsTemp,
    CleanupCategory::BrowserCache,
    CleanupCategory::ThumbnailCache,
    CleanupCategory::RecycleBin,
];

/// Machine-wide categories, addressed by absolute paths only.
///
/// These are the only ones the elevated (SYSTEM) side is allowed to touch.
pub const ALL_SYSTEM: [CleanupCategory; 2] = [
    CleanupCategory::WindowsUpdateCache,
    CleanupCategory::WindowsLogs,
];

/// `false` for the two machine-wide categories, `true` for everything else.
///
/// Deliberately exhaustive: adding a category without deciding its context is
/// a compile error, not a silent routing bug.
pub fn is_user_owned(category: &CleanupCategory) -> bool {
    match category {
        CleanupCategory::WindowsTemp
        | CleanupCategory::BrowserCache
        | CleanupCategory::ThumbnailCache
        | CleanupCategory::RecycleBin => true,
        CleanupCategory::WindowsUpdateCache | CleanupCategory::WindowsLogs => false,
    }
}

/// `true` for the two machine-wide categories.
pub fn is_system_owned(category: &CleanupCategory) -> bool {
    !is_user_owned(category)
}

/// Split a requested category list into (user-owned, system-owned).
///
/// An empty request means "everything" and keeps the historical semantics:
/// every user category runs unprivileged and the two system categories run
/// elevated.
pub fn partition_context(
    categories: Vec<CleanupCategory>,
) -> (Vec<CleanupCategory>, Vec<CleanupCategory>) {
    if categories.is_empty() {
        return (ALL_USER.to_vec(), ALL_SYSTEM.to_vec());
    }
    let mut user = Vec::new();
    let mut system = Vec::new();
    for category in categories {
        if is_user_owned(&category) {
            user.push(category);
        } else {
            system.push(category);
        }
    }
    (user, system)
}

/// Scan for junk files and return cleanup items.
///
/// This enumerates known temp/cache directories and calculates
/// the total size that can be freed for each category.
///
/// Runs in the caller's process — the app, therefore the interactive user —
/// so the numbers match what [`clean_user_junk`] will actually delete.
pub fn scan_junk_files() -> HardwareResult<Vec<CleanupItem>> {
    let mut items = Vec::new();

    // Windows Temp (the user's %TEMP%)
    if let Some(temp_path) = get_temp_dir() {
        let (size, count) = calculate_dir_size(&temp_path);
        items.push(CleanupItem {
            category: CleanupCategory::WindowsTemp,
            description: format!("Windows Temp ({})", temp_path.display()),
            size_bytes: size,
            file_count: count,
        });
    }

    // Windows Update cache
    let wu_path = PathBuf::from(r"C:\Windows\SoftwareDistribution\Download");
    if wu_path.exists() {
        let (size, count) = calculate_dir_size(&wu_path);
        items.push(CleanupItem {
            category: CleanupCategory::WindowsUpdateCache,
            description: "Windows Update cache".to_string(),
            size_bytes: size,
            file_count: count,
        });
    }

    // Browser caches
    let (size, count) = get_browser_cache_size();
    items.push(CleanupItem {
        category: CleanupCategory::BrowserCache,
        description: "Browser caches (Chrome, Edge, Firefox)".to_string(),
        size_bytes: size,
        file_count: count,
    });

    // Recycle Bin
    let (size, count) = get_recycle_bin_size();
    items.push(CleanupItem {
        category: CleanupCategory::RecycleBin,
        description: "Recycle Bin".to_string(),
        size_bytes: size,
        file_count: count,
    });

    // Thumbnail cache
    if let Some(thumb_path) = get_thumbnail_cache_dir() {
        let (size, count) = calculate_dir_size(&thumb_path);
        items.push(CleanupItem {
            category: CleanupCategory::ThumbnailCache,
            description: "Thumbnail cache".to_string(),
            size_bytes: size,
            file_count: count,
        });
    }

    // Windows logs
    let logs_path = PathBuf::from(r"C:\Windows\Logs");
    if logs_path.exists() {
        let (size, count) = calculate_dir_size(&logs_path);
        items.push(CleanupItem {
            category: CleanupCategory::WindowsLogs,
            description: "Windows log files".to_string(),
            size_bytes: size,
            file_count: count,
        });
    }

    Ok(items)
}

/// Clean the categories that belong to the interactive user's profile.
///
/// **Unprivileged by design.** This is the function the app calls in its own
/// process; `TEMP` / `LOCALAPPDATA` / `APPDATA` and the Recycle Bin therefore
/// resolve to the same profile `scan_junk_files` measured.
///
/// An empty `categories` list means "all user categories". System-owned
/// categories are rejected loudly — silently dropping them would recreate the
/// original bug (the UI would report a category as cleaned when it never ran).
pub fn clean_user_junk(categories: Vec<CleanupCategory>) -> HardwareResult<Vec<CleanupResult>> {
    let categories = resolve_categories(categories, ALL_USER.as_slice(), "clean_user_junk")?;
    Ok(categories.iter().map(clean_category).collect())
}

/// Clean the two machine-wide categories.
///
/// **Elevated by design.** Only absolute paths are touched here; no environment
/// variable is resolved on this side, because under LocalSystem `TEMP` and
/// `LOCALAPPDATA` point at the service profile.
///
/// An empty `categories` list means "all system categories". User-owned
/// categories are rejected: use [`clean_user_junk`] for those.
pub fn clean_system_junk(categories: Vec<CleanupCategory>) -> HardwareResult<Vec<CleanupResult>> {
    let categories = resolve_categories(categories, ALL_SYSTEM.as_slice(), "clean_system_junk")?;
    Ok(categories.iter().map(clean_category).collect())
}

/// Validate a category list against the categories a context may clean.
fn resolve_categories(
    categories: Vec<CleanupCategory>,
    allowed: &[CleanupCategory],
    context: &str,
) -> HardwareResult<Vec<CleanupCategory>> {
    if categories.is_empty() {
        return Ok(allowed.to_vec());
    }
    for category in &categories {
        if !allowed.contains(category) {
            return Err(HardwareError::Other(format!(
                "{context} refused category {category:?}: it belongs to the other execution context"
            )));
        }
    }
    Ok(categories)
}

/// Clean a specific category.
fn clean_category(category: &CleanupCategory) -> CleanupResult {
    let mut totals = CategoryTotals::default();

    match category {
        CleanupCategory::WindowsTemp => {
            if let Some(temp_path) = get_temp_dir() {
                totals.absorb(&temp_path, clean_directory(&temp_path));
            } else {
                totals.errors.push("%TEMP% is not set".to_string());
            }
        }
        CleanupCategory::WindowsUpdateCache => {
            let path = PathBuf::from(r"C:\Windows\SoftwareDistribution\Download");
            totals.absorb(&path, clean_directory(&path));
        }
        CleanupCategory::BrowserCache => {
            for cache_dir in get_browser_cache_dirs() {
                if cache_dir.exists() {
                    totals.absorb(&cache_dir, clean_directory(&cache_dir));
                }
            }
        }
        CleanupCategory::RecycleBin => {
            // Measure BEFORE emptying: SHEmptyRecycleBinW does not report how
            // much it freed, so the (honest) freed figure is the pre-size.
            let (pre_size, pre_items) = get_recycle_bin_size();
            match empty_recycle_bin() {
                Ok(()) => {
                    totals.freed += pre_size;
                    totals.removed += pre_items;
                    log::info!(
                        target: "hw::cleanup",
                        "category=RecycleBin freed={pre_size} items={pre_items}"
                    );
                }
                Err(e) => totals.errors.push(e),
            }
            return totals.into_result(category);
        }
        CleanupCategory::ThumbnailCache => {
            if let Some(thumb_path) = get_thumbnail_cache_dir() {
                totals.absorb(&thumb_path, clean_directory(&thumb_path));
            }
        }
        CleanupCategory::WindowsLogs => {
            let path = PathBuf::from(r"C:\Windows\Logs");
            totals.absorb(&path, clean_directory(&path));
        }
    }

    totals.into_result(category)
}

/// Accumulator for one category (avoids four parallel locals per arm).
#[derive(Default)]
struct CategoryTotals {
    freed: u64,
    removed: u64,
    skipped: u64,
    errors: Vec<String>,
}

impl CategoryTotals {
    /// Absorb one `clean_directory` outcome and log the path that was tried.
    fn absorb(&mut self, path: &Path, outcome: (u64, u64, u64, Vec<String>)) {
        let (freed, removed, skipped, errors) = outcome;
        self.freed += freed;
        self.removed += removed;
        self.skipped += skipped;
        self.errors.extend(errors);
        log::info!(
            target: "hw::cleanup",
            "path={} freed={} removed={} skipped={}",
            path.display(),
            freed,
            removed,
            skipped
        );
    }

    fn into_result(self, category: &CleanupCategory) -> CleanupResult {
        log::info!(
            target: "hw::cleanup",
            "Cleaned {:?}: freed {} bytes, removed {} files, skipped {} files, errors {}",
            category,
            self.freed,
            self.removed,
            self.skipped,
            self.errors.len()
        );
        CleanupResult {
            category: category.clone(),
            freed_bytes: self.freed,
            files_removed: self.removed,
            files_skipped: self.skipped,
            errors: self.errors,
        }
    }
}

/// Recursively calculate directory size and file count.
fn calculate_dir_size(path: &Path) -> (u64, u64) {
    let mut size = 0u64;
    let mut count = 0u64;

    if path.is_file() {
        if let Ok(meta) = path.metadata() {
            size += meta.len();
            count += 1;
        }
        return (size, count);
    }

    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                let (s, c) = calculate_dir_size(&entry_path);
                size += s;
                count += c;
            } else if let Ok(meta) = entry.metadata() {
                size += meta.len();
                count += 1;
            }
        }
    }

    (size, count)
}

/// Delete all files in a directory (recursing into subdirectories, deleting
/// them when they end up empty).
///
/// Returns (freed_bytes, files_removed, files_skipped, errors).
fn clean_directory(path: &Path) -> (u64, u64, u64, Vec<String>) {
    let mut freed = 0u64;
    let mut removed = 0u64;
    let mut skipped = 0u64;
    let mut errors = Vec::new();

    if !path.exists() {
        return (0, 0, 0, errors);
    }

    clean_dir_recursive(path, &mut freed, &mut removed, &mut skipped, &mut errors, 0);

    (freed, removed, skipped, errors)
}

fn clean_dir_recursive(
    path: &Path,
    freed: &mut u64,
    removed: &mut u64,
    skipped: &mut u64,
    errors: &mut Vec<String>,
    depth: usize,
) {
    // S32-002: Hard depth limit — never recurse deeper than 12 levels and
    // never follow directory symlinks/junctions/reparse points. A junction
    // planted in %TEMP% could otherwise point at an arbitrary directory
    // (e.g. C:\Windows\System32) and get recursively deleted.
    if depth > 12 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let entry_path = entry.path();
        // Check for reparse points BEFORE is_dir() — on Windows,
        // is_dir() follows symlinks/junctions, so a junction to a
        // directory would pass is_dir() and be recursed into.
        let is_reparse = entry.file_type().map(|ft| ft.is_symlink()).unwrap_or(false)
            || is_reparse_point(&entry_path);
        if is_reparse {
            // Skip symlinks/junctions entirely; only delete the link
            // itself (not its target).
            if let Ok(meta) = entry.metadata() {
                let size = meta.len();
                match std::fs::remove_file(&entry_path) {
                    Ok(()) => {
                        *freed += size;
                        *removed += 1;
                    }
                    Err(e) => {
                        *skipped += 1;
                        record_error(errors, &entry_path, &e);
                    }
                }
            }
            continue;
        }
        if entry_path.is_dir() {
            clean_dir_recursive(&entry_path, freed, removed, skipped, errors, depth + 1);
            // Try to remove the now-empty directory
            let _ = std::fs::remove_dir(&entry_path);
        } else {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            match std::fs::remove_file(&entry_path) {
                Ok(()) => {
                    *freed += size;
                    *removed += 1;
                }
                Err(e) => {
                    *skipped += 1;
                    record_error(errors, &entry_path, &e);
                }
            }
        }
    }
}

/// Record a per-file failure, capped so one locked directory cannot return
/// thousands of strings over IPC.
fn record_error(errors: &mut Vec<String>, path: &Path, error: &std::io::Error) {
    if errors.len() < MAX_RECORDED_ERRORS {
        errors.push(format!("{}: {error}", path.display()));
    }
}

/// S32-002: Detect whether a path is a reparse point (symlink, junction or
/// mount point) using `GetFileAttributesW` FILE_ATTRIBUTE_REPARSE_POINT.
#[cfg(windows)]
fn is_reparse_point(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        GetFileAttributesW, FILE_ATTRIBUTE_REPARSE_POINT, INVALID_FILE_ATTRIBUTES,
    };

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: path_w is a valid null-terminated wide string.
    let attrs = unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) };
    attrs != INVALID_FILE_ATTRIBUTES && (attrs & FILE_ATTRIBUTE_REPARSE_POINT.0) != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_path: &Path) -> bool {
    false
}

/// Get the Windows TEMP directory of the current process.
///
/// When this runs in the app process (the only place user cleanup runs) it is
/// the interactive user's `%TEMP%`.
fn get_temp_dir() -> Option<PathBuf> {
    std::env::var_os("TEMP").map(PathBuf::from)
}

/// Get browser cache directories (Chrome, Edge and Firefox, all profiles).
fn get_browser_cache_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        for user_data in [
            local_app_data.join(r"Google\Chrome\User Data"),
            local_app_data.join(r"Microsoft\Edge\User Data"),
        ] {
            for profile in browser_profile_dirs(&user_data) {
                for sub in BROWSER_CACHE_SUBDIRS {
                    dirs.push(profile.join(sub));
                }
            }
        }
    }

    dirs.extend(firefox_cache_dirs());

    dirs
}

/// Cache subdirectories inside a Chromium profile (relative to it).
const BROWSER_CACHE_SUBDIRS: [&str; 5] = [
    "Cache",
    "Code Cache",
    "GPUCache",
    r"Service Worker\CacheStorage",
    "ShaderCache",
];

/// Chromium profiles live in `Default` and `Profile 1..N`; the other entries
/// (`Crashpad`, `GrShaderCache`, …) are not per-profile caches.
fn browser_profile_dirs(user_data: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(user_data) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .collect();
    names.sort();
    names
        .into_iter()
        .filter(|name| name == "Default" || name.starts_with("Profile "))
        .map(|name| user_data.join(name))
        .collect()
}

/// Firefox keeps `cache2` under **LOCALAPPDATA** (`%APPDATA%` only holds
/// `profiles.ini` and preferences). Profiles may live elsewhere when
/// `IsRelative=0` in profiles.ini — those absolute paths are honoured too.
fn firefox_cache_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        let profiles = local_app_data.join(r"Mozilla\Firefox\Profiles");
        if let Ok(entries) = std::fs::read_dir(&profiles) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    dirs.push(entry.path().join("cache2"));
                }
            }
        }
    }

    if let Some(app_data) = std::env::var_os("APPDATA").map(PathBuf::from) {
        let base = app_data.join(r"Mozilla\Firefox");
        if let Ok(ini) = std::fs::read_to_string(base.join("profiles.ini")) {
            for profile in parse_firefox_absolute_profiles(&ini) {
                dirs.push(PathBuf::from(profile).join("cache2"));
            }
        }
    }

    dirs
}

/// Extract `Path=` values of `[ProfileN]` sections whose `IsRelative=0`.
///
/// `IsRelative` defaults to 1 (relative to the Firefox base directory), and
/// paths in profiles.ini use forward slashes on every platform.
fn parse_firefox_absolute_profiles(ini: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut relative = true;
    for line in ini.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            relative = true;
            continue;
        }
        if let Some(value) = line.strip_prefix("IsRelative=") {
            relative = value.trim() != "0";
            continue;
        }
        if let Some(value) = line.strip_prefix("Path=") {
            let path = value.trim();
            if !relative && !path.is_empty() {
                out.push(path.replace('/', "\\"));
            }
        }
    }
    out
}

/// Calculate total browser cache size.
fn get_browser_cache_size() -> (u64, u64) {
    let mut size = 0u64;
    let mut count = 0u64;
    for dir in get_browser_cache_dirs() {
        let (s, c) = calculate_dir_size(&dir);
        size += s;
        count += c;
    }
    (size, count)
}

/// Get the Recycle Bin size and item count across all drives (Shell API).
///
/// `SHQueryRecycleBinW(NULL, …)` reports every Recycle Bin on the machine for
/// the calling user. Before this, the function returned a hardcoded `(0, 0)`
/// with a comment saying the real size would be reported after cleanup — it
/// never was, so the UI always showed "0 B · 0 files" for a bin holding
/// hundreds of MB.
fn get_recycle_bin_size() -> (u64, u64) {
    #[cfg(windows)]
    {
        let mut info = SHQUERYRBINFO {
            cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
            i64Size: 0,
            i64NumItems: 0,
        };
        // SAFETY: `info` is a properly sized, initialised SHQUERYRBINFO and we
        // pass a null root path, which asks for all drives.
        let hr = unsafe { SHQueryRecycleBinW(std::ptr::null(), &mut info) };
        if hr_is_success(hr) {
            return recycle_raw_to_size(info.i64Size, info.i64NumItems);
        }
        log::warn!(
            target: "hw::cleanup",
            "SHQueryRecycleBinW failed: 0x{:08X}",
            hr as u32
        );
        (0, 0)
    }
    #[cfg(not(windows))]
    (0, 0)
}

/// Empty the Recycle Bin of the calling user.
///
/// Returns `Ok` for both `S_OK` (0) and `S_FALSE` (1): `S_FALSE` means the bin
/// was already empty, which is a normal outcome, not a failure.
fn empty_recycle_bin() -> Result<(), String> {
    // SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND
    const SHERB_FLAGS: u32 = 0x0001 | 0x0002 | 0x0004;
    #[cfg(windows)]
    {
        // SAFETY: no owner window, null root path (all drives), documented flags.
        let hr = unsafe { SHEmptyRecycleBinW(std::ptr::null_mut(), std::ptr::null(), SHERB_FLAGS) };
        if hr_is_success(hr) {
            Ok(())
        } else {
            Err(format!("SHEmptyRecycleBinW failed: 0x{:08X}", hr as u32))
        }
    }
    #[cfg(not(windows))]
    Err("Recycle Bin cleanup is only supported on Windows".to_string())
}

/// HRESULT success test (`S_OK` = 0, `S_FALSE` = 1, everything negative failed).
fn hr_is_success(hr: i32) -> bool {
    hr >= 0
}

/// Clamp the signed `SHQUERYRBINFO` counters into the unsigned wire format.
fn recycle_raw_to_size(size: i64, items: i64) -> (u64, u64) {
    (size.max(0) as u64, items.max(0) as u64)
}

/// `SHQUERYRBINFO` from `shellapi.h` (field names kept verbatim from C).
#[cfg(windows)]
#[repr(C)]
#[allow(non_snake_case)]
// The name is the Win32 struct name; renaming it would make the FFI contract
// harder to check against the SDK header.
#[allow(clippy::upper_case_acronyms)]
struct SHQUERYRBINFO {
    cbSize: u32,
    i64Size: i64,
    i64NumItems: i64,
}

#[cfg(windows)]
windows_targets::link!(
    "shell32.dll"
    "system"
    fn SHQueryRecycleBinW(pszrootpath: *const u16, pqbinfo: *mut SHQUERYRBINFO) -> i32
);

#[cfg(windows)]
windows_targets::link!(
    "shell32.dll"
    "system"
    fn SHEmptyRecycleBinW(hwnd: *mut std::ffi::c_void, pszrootpath: *const u16, dwflags: u32) -> i32
);

/// Get the thumbnail cache directory.
fn get_thumbnail_cache_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join(r"Microsoft\Windows\Explorer"))
}

// ── Tests ────────────────────────────────────────────────────────────────────
//
// Everything here is synthetic: a throwaway directory under %TEMP%, an
// in-memory profiles.ini, and synthetic SHQUERYRBINFO/HRESULT values. No test
// touches a real cleanup target (`C:\Windows\*`, the Recycle Bin, browser
// profiles) or dispatches anything through the elevated bridge.

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique, empty scratch directory under %TEMP% (removed on drop).
    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "micontrol-cleanup-test-{tag}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create scratch dir");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // ── Context routing (F2.1) ──────────────────────────────────────────────

    #[test]
    fn every_category_has_exactly_one_context() {
        for category in ALL_USER.iter().chain(ALL_SYSTEM.iter()) {
            assert_ne!(
                is_user_owned(category),
                is_system_owned(category),
                "{category:?} must belong to exactly one context"
            );
        }
        for category in ALL_USER {
            assert!(is_user_owned(&category), "{category:?} is user-owned");
        }
        for category in ALL_SYSTEM {
            assert!(is_system_owned(&category), "{category:?} is system-owned");
        }
    }

    #[test]
    fn partition_routes_by_context() {
        let (user, system) = partition_context(vec![
            CleanupCategory::RecycleBin,
            CleanupCategory::WindowsLogs,
            CleanupCategory::WindowsTemp,
            CleanupCategory::WindowsUpdateCache,
        ]);
        assert_eq!(
            user,
            vec![CleanupCategory::RecycleBin, CleanupCategory::WindowsTemp]
        );
        assert_eq!(
            system,
            vec![
                CleanupCategory::WindowsLogs,
                CleanupCategory::WindowsUpdateCache
            ]
        );
    }

    #[test]
    fn partition_empty_means_everything() {
        let (user, system) = partition_context(Vec::new());
        assert_eq!(user.len(), 4);
        assert_eq!(system.len(), 2);
        assert!(user.iter().all(is_user_owned));
        assert!(system.iter().all(is_system_owned));
    }

    #[test]
    fn user_context_refuses_system_categories() {
        let err = resolve_categories(
            vec![CleanupCategory::WindowsLogs],
            ALL_USER.as_slice(),
            "clean_user_junk",
        )
        .expect_err("system category must be refused by the user context");
        assert!(err.to_string().contains("clean_user_junk"), "{err}");
    }

    #[test]
    fn system_context_refuses_user_categories() {
        let err = resolve_categories(
            vec![CleanupCategory::RecycleBin],
            ALL_SYSTEM.as_slice(),
            "clean_system_junk",
        )
        .expect_err("user category must be refused by the system context");
        assert!(err.to_string().contains("clean_system_junk"), "{err}");
    }

    // ── Deletion accounting + error reporting (F2.3) ────────────────────────

    #[test]
    fn clean_directory_removes_files_and_reports_bytes() {
        let scratch = ScratchDir::new("bytes");
        std::fs::write(scratch.path().join("a.tmp"), vec![7u8; 1024]).unwrap();
        std::fs::write(scratch.path().join("b.tmp"), vec![7u8; 512]).unwrap();
        let nested = scratch.path().join("sub");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("c.tmp"), vec![7u8; 256]).unwrap();

        let (freed, removed, skipped, errors) = clean_directory(scratch.path());

        assert_eq!(freed, 1024 + 512 + 256);
        assert_eq!(removed, 3);
        assert_eq!(skipped, 0);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert!(std::fs::read_dir(scratch.path()).unwrap().next().is_none());
    }

    #[test]
    fn clean_directory_missing_path_is_a_noop() {
        let missing = std::env::temp_dir().join("micontrol-cleanup-test-missing-xyz");
        assert_eq!(clean_directory(&missing), (0, 0, 0, Vec::new()));
    }

    #[cfg(windows)]
    #[test]
    fn clean_directory_reports_why_a_file_was_skipped() {
        use std::os::windows::fs::OpenOptionsExt;

        let scratch = ScratchDir::new("locked");
        let locked = scratch.path().join("locked.tmp");
        std::fs::write(&locked, b"x").unwrap();
        // share_mode(0) = exclusive: no other handle may delete it.
        let _handle = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&locked)
            .expect("open exclusive handle");

        let (_, removed, skipped, errors) = clean_directory(scratch.path());

        assert_eq!(removed, 0);
        assert_eq!(skipped, 1);
        assert_eq!(errors.len(), 1, "the reason must reach the UI");
        assert!(errors[0].contains("locked.tmp"), "{errors:?}");
    }

    #[test]
    fn recorded_errors_are_capped() {
        let mut errors = Vec::new();
        let err = std::io::Error::other("busy");
        for i in 0..(MAX_RECORDED_ERRORS * 3) {
            record_error(&mut errors, Path::new(&format!("C:\\x\\{i}.tmp")), &err);
        }
        assert_eq!(errors.len(), MAX_RECORDED_ERRORS);
        assert!(errors[0].ends_with("busy"), "{:?}", errors.first());
    }

    #[test]
    fn failure_result_never_reports_success() {
        let result = CleanupResult::failure(CleanupCategory::WindowsLogs, "bridge offline");
        assert_eq!(result.freed_bytes, 0);
        assert_eq!(result.files_removed, 0);
        assert_eq!(result.errors, vec!["bridge offline".to_string()]);
    }

    // ── Recycle Bin (F2.2) ──────────────────────────────────────────────────

    #[test]
    fn hresult_s_false_is_success() {
        assert!(hr_is_success(0), "S_OK");
        assert!(hr_is_success(1), "S_FALSE = already-empty bin");
        assert!(!hr_is_success(0x8007_0005u32 as i32), "E_ACCESSDENIED");
        assert!(!hr_is_success(0x8000_4005u32 as i32), "E_FAIL");
    }

    #[test]
    fn recycle_counters_are_clamped() {
        assert_eq!(recycle_raw_to_size(122_124_452, 190), (122_124_452, 190));
        assert_eq!(recycle_raw_to_size(0, 0), (0, 0));
        assert_eq!(recycle_raw_to_size(-5, -2), (0, 0));
    }

    #[cfg(windows)]
    #[test]
    fn shquery_rbinfo_layout_matches_c() {
        // cbSize (4) + padding (4) + two i64 → 24 bytes with default alignment.
        assert_eq!(std::mem::size_of::<SHQUERYRBINFO>(), 24);
    }

    // ── Browser caches (F2.6 / F2.7) ────────────────────────────────────────

    #[test]
    fn firefox_ini_ignores_relative_profiles() {
        let ini = "\
[General]
StartWithLastProfile=1

[Profile0]
Name=default
IsRelative=1
Path=Profiles/abc.default
Default=1

[Profile1]
Name=portable
IsRelative=0
Path=D:/Portable/Firefox/Profiles/port.default
";
        assert_eq!(
            parse_firefox_absolute_profiles(ini),
            vec!["D:\\Portable\\Firefox\\Profiles\\port.default".to_string()]
        );
    }

    #[test]
    fn firefox_ini_defaults_to_relative() {
        // No IsRelative line → relative (the documented default).
        let ini = "[Profile0]\nPath=Profiles/abc.default\n";
        assert!(parse_firefox_absolute_profiles(ini).is_empty());
    }

    #[test]
    fn firefox_ini_tolerates_garbage() {
        assert!(parse_firefox_absolute_profiles("").is_empty());
        assert!(parse_firefox_absolute_profiles("Path=\n[Profile0]\nIsRelative=0\n").is_empty());
    }

    #[test]
    fn browser_profiles_include_default_and_numbered_only() {
        let scratch = ScratchDir::new("profiles");
        for name in [
            "Default",
            "Profile 1",
            "Profile 2",
            "Crashpad",
            "ShaderCache",
        ] {
            std::fs::create_dir_all(scratch.path().join(name)).unwrap();
        }
        std::fs::write(scratch.path().join("Local State"), b"{}").unwrap();

        let found: Vec<String> = browser_profile_dirs(scratch.path())
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert_eq!(found, vec!["Default", "Profile 1", "Profile 2"]);
    }

    #[test]
    fn browser_profile_dirs_missing_root_is_empty() {
        let missing = std::env::temp_dir().join("micontrol-cleanup-test-noprofiles");
        assert!(browser_profile_dirs(&missing).is_empty());
    }
}
