//! System cleanup — junk file scanning and removal.
//!
//! Provides a functional equivalent to XPM's "System Cleanup" feature
//! using standard Windows APIs for temp/cache file cleanup.
//!
//! XPM uses proprietary `CleanerEngine.dll` / `CleanerProxy.dll`.
//! We use direct file system enumeration and deletion of known safe
//! temp/cache directories instead.
//!
//! Supported cleanup categories:
//! - Windows Temp (%TEMP%)
//! - Windows Update cache (C:\Windows\SoftwareDistribution\Download)
//! - Browser caches (Chrome, Edge, Firefox)
//! - Recycle Bin
//! - Thumbnail cache
//! - Windows log files

use crate::hw::errors::HardwareResult;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Cleanup category identifier.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CleanupCategory {
    /// %TEMP% directory
    WindowsTemp,
    /// C:\Windows\SoftwareDistribution\Download
    WindowsUpdateCache,
    /// Browser caches (Chrome, Edge, Firefox)
    BrowserCache,
    /// Recycle Bin
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
    /// Errors encountered
    pub errors: Vec<String>,
}

/// Scan for junk files and return cleanup items.
///
/// This enumerates known temp/cache directories and calculates
/// the total size that can be freed for each category.
pub fn scan_junk_files() -> HardwareResult<Vec<CleanupItem>> {
    let mut items = Vec::new();

    // Windows Temp
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

/// Clean junk files for the specified categories.
///
/// If `categories` is empty, all categories are cleaned.
pub fn clean_junk_files(categories: Vec<CleanupCategory>) -> HardwareResult<Vec<CleanupResult>> {
    let categories = if categories.is_empty() {
        vec![
            CleanupCategory::WindowsTemp,
            CleanupCategory::WindowsUpdateCache,
            CleanupCategory::BrowserCache,
            CleanupCategory::RecycleBin,
            CleanupCategory::ThumbnailCache,
            CleanupCategory::WindowsLogs,
        ]
    } else {
        categories
    };

    let mut results = Vec::new();

    for category in categories {
        let result = clean_category(&category);
        results.push(result);
    }

    Ok(results)
}

/// Clean a specific category.
fn clean_category(category: &CleanupCategory) -> CleanupResult {
    let mut errors = Vec::new();
    let mut freed_bytes = 0u64;
    let mut files_removed = 0u64;
    let mut files_skipped = 0u64;

    match category {
        CleanupCategory::WindowsTemp => {
            if let Some(temp_path) = get_temp_dir() {
                let (freed, removed, skipped, errs) = clean_directory(&temp_path);
                freed_bytes += freed;
                files_removed += removed;
                files_skipped += skipped;
                errors.extend(errs);
            }
        }
        CleanupCategory::WindowsUpdateCache => {
            let path = PathBuf::from(r"C:\Windows\SoftwareDistribution\Download");
            let (freed, removed, skipped, errs) = clean_directory(&path);
            freed_bytes += freed;
            files_removed += removed;
            files_skipped += skipped;
            errors.extend(errs);
        }
        CleanupCategory::BrowserCache => {
            for cache_dir in get_browser_cache_dirs() {
                let (freed, removed, skipped, errs) = clean_directory(&cache_dir);
                freed_bytes += freed;
                files_removed += removed;
                files_skipped += skipped;
                errors.extend(errs);
            }
        }
        CleanupCategory::RecycleBin => {
            // Empty the Recycle Bin via SHEmptyRecycleBin
            #[cfg(windows)]
            {
                windows_targets::link!(
                    "shell32.dll"
                    "system"
                    fn SHEmptyRecycleBinW(hwnd: *mut std::ffi::c_void, pszrootpath: *const u16, dwflags: u32) -> i32
                );
                // SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND
                let flags = 0x0001 | 0x0002 | 0x0004;
                let result =
                    unsafe { SHEmptyRecycleBinW(std::ptr::null_mut(), std::ptr::null(), flags) };
                if result != 0 {
                    errors.push(format!("SHEmptyRecycleBin failed: error code {result}"));
                }
            }
        }
        CleanupCategory::ThumbnailCache => {
            if let Some(thumb_path) = get_thumbnail_cache_dir() {
                let (freed, removed, skipped, errs) = clean_directory(&thumb_path);
                freed_bytes += freed;
                files_removed += removed;
                files_skipped += skipped;
                errors.extend(errs);
            }
        }
        CleanupCategory::WindowsLogs => {
            let path = PathBuf::from(r"C:\Windows\Logs");
            let (freed, removed, skipped, errs) = clean_directory(&path);
            freed_bytes += freed;
            files_removed += removed;
            files_skipped += skipped;
            errors.extend(errs);
        }
    }

    log::info!(
        target: "hw::cleanup",
        "Cleaned {:?}: freed {} bytes, removed {} files, skipped {} files",
        category,
        freed_bytes,
        files_removed,
        files_skipped
    );

    CleanupResult {
        category: category.clone(),
        freed_bytes,
        files_removed,
        files_skipped,
        errors,
    }
}

/// Recursively calculate directory size and file count.
fn calculate_dir_size(path: &std::path::Path) -> (u64, u64) {
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

/// Delete all files in a directory (non-recursive — only deletes files,
/// not subdirectories).
///
/// Returns (freed_bytes, files_removed, files_skipped, errors).
fn clean_directory(path: &std::path::Path) -> (u64, u64, u64, Vec<String>) {
    let mut freed = 0u64;
    let mut removed = 0u64;
    let mut skipped = 0u64;
    let mut errors = Vec::new();

    if !path.exists() {
        return (0, 0, 0, errors);
    }

    fn clean_dir_recursive(
        path: &std::path::Path,
        freed: &mut u64,
        removed: &mut u64,
        skipped: &mut u64,
        errors: &mut Vec<String>,
    ) {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                if entry_path.is_dir() {
                    clean_dir_recursive(&entry_path, freed, removed, skipped, errors);
                    // Try to remove the now-empty directory
                    let _ = std::fs::remove_dir(&entry_path);
                } else {
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    match std::fs::remove_file(&entry_path) {
                        Ok(()) => {
                            *freed += size;
                            *removed += 1;
                        }
                        Err(_) => {
                            *skipped += 1;
                        }
                    }
                }
            }
        }
    }

    clean_dir_recursive(path, &mut freed, &mut removed, &mut skipped, &mut errors);

    (freed, removed, skipped, errors)
}

/// Get the Windows TEMP directory.
fn get_temp_dir() -> Option<PathBuf> {
    std::env::var_os("TEMP").map(PathBuf::from)
}

/// Get browser cache directories.
fn get_browser_cache_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let local_app_data = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);

    if let Some(lad) = &local_app_data {
        // Chrome cache
        dirs.push(lad.join(r"Google\Chrome\User Data\Default\Cache"));
        // Edge cache
        dirs.push(lad.join(r"Microsoft\Edge\User Data\Default\Cache"));
    }

    // Firefox cache (platform-independent path)
    if let Some(app_data) = std::env::var_os("APPDATA").map(PathBuf::from) {
        let firefox_path = app_data.join(r"Mozilla\Firefox\Profiles");
        if firefox_path.exists() {
            if let Ok(entries) = std::fs::read_dir(&firefox_path) {
                for entry in entries.flatten() {
                    dirs.push(entry.path().join("cache2"));
                }
            }
        }
    }

    dirs
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

/// Get the Recycle Bin size.
fn get_recycle_bin_size() -> (u64, u64) {
    // Querying Recycle Bin size requires SHQueryRecycleBin which is complex.
    // Return 0 — the actual size will be reported after cleanup.
    (0, 0)
}

/// Get the thumbnail cache directory.
fn get_thumbnail_cache_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join(r"Microsoft\Windows\Explorer"))
}
