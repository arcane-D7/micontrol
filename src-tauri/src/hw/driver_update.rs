//! Official Xiaomi driver update checking.
//!
//! Scrapes Xiaomi's public driver download portal at
//! `https://www.mi.com/service/notebook/drivers/{model_code}` to fetch
//! the list of official drivers for this laptop model, then compares
//! them against the locally installed drivers (via WMI) to identify
//! available updates.
//!
//! This ensures MiControl checks against Xiaomi's official driver
//! database rather than relying solely on Windows Update (which may
//! provide generic drivers instead of Xiaomi-specific ones).

use crate::hw::errors::{HardwareError, HardwareResult};
use crate::hw::update::DriverDetail;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Constants ────────────────────────────────────────────────────────────────

const DRIVER_PORTAL_BASE: &str = "https://www.mi.com/service/notebook/drivers";

// ── Data structures ──────────────────────────────────────────────────────────

/// A driver listed on Xiaomi's official download portal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfficialDriver {
    /// Category: "BIOS", "WLAN", "GPU", "Audio", "Bluetooth", etc.
    pub category: String,
    /// Human-readable name, e.g. "Intel AX211 WLAN Driver"
    pub name: String,
    /// Version string extracted from the page, e.g. "23.170.0.1G"
    pub version: String,
    /// Release date if available, e.g. "2026-06-12"
    pub date: String,
    /// Direct CDN download URL
    pub download_url: String,
    /// File size if listed
    pub file_size: Option<String>,
}

/// Result of comparing installed drivers against official ones.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverUpdateCheck {
    /// Detected model code, e.g. "TM2424"
    pub model_code: String,
    /// Drivers listed on Xiaomi's portal
    pub official_drivers: Vec<OfficialDriver>,
    /// Currently installed drivers (from WMI)
    pub installed_drivers: Vec<DriverDetail>,
    /// Drivers where the official version is newer than installed
    pub updates_available: Vec<DriverUpdate>,
}

/// A single driver update recommendation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverUpdate {
    /// Device name from WMI
    pub device_name: String,
    /// Currently installed version
    pub installed_version: String,
    /// Official version from Xiaomi portal
    pub official_version: String,
    /// Direct download URL
    pub download_url: String,
    /// Driver category
    pub category: String,
    /// Official driver name
    pub official_name: String,
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Detect the laptop model code (e.g. "TM2424") from WMI/registry.
pub fn detect_model_code() -> HardwareResult<String> {
    // Strategy 1: WMI Win32_BaseBoard Product field
    #[cfg(windows)]
    {
        use crate::hw::wmi_cache;
        use crate::util::wmi_extract;

        let model = wmi_cache::with_cimv2(|wmi| {
            let results: Vec<HashMap<String, wmi::Variant>> =
                wmi.raw_query("SELECT Product FROM Win32_BaseBoard")?;
            Ok(results
                .into_iter()
                .next()
                .map(|r| wmi_extract::extract_string_or(&r, "Product", ""))
                .unwrap_or_default())
        })?;

        if !model.is_empty() && model != "Default string" {
            return Ok(model);
        }
    }

    // Strategy 2: Registry HKLM\HARDWARE\DESCRIPTION\System\BIOS\SystemProductName
    #[cfg(windows)]
    {
        use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        if let Ok(key) = hklm.open_subkey("HARDWARE\\DESCRIPTION\\System\\BIOS") {
            if let Ok(name) = key.get_value::<String, _>("SystemProductName") {
                if !name.is_empty() && name != "Default string" {
                    return Ok(name);
                }
            }
        }
    }

    // Strategy 3: WMI Win32_ComputerSystemProduct
    #[cfg(windows)]
    {
        use crate::hw::wmi_cache;
        use crate::util::wmi_extract;

        let name = wmi_cache::with_cimv2(|wmi| {
            let results: Vec<HashMap<String, wmi::Variant>> =
                wmi.raw_query("SELECT Name FROM Win32_ComputerSystemProduct")?;
            Ok(results
                .into_iter()
                .next()
                .map(|r| wmi_extract::extract_string_or(&r, "Name", ""))
                .unwrap_or_default())
        })?;

        if !name.is_empty() {
            return Ok(name);
        }
    }

    Err(HardwareError::Other(
        "Could not detect laptop model code. \
         Set it manually in MiControl settings."
            .to_string(),
    ))
}

/// Fetch the list of official drivers from Xiaomi's driver portal.
///
/// Scrapes the HTML page at `mi.com/service/notebook/drivers/{model_code}`
/// and extracts driver download links.
pub async fn fetch_official_drivers(model_code: &str) -> HardwareResult<Vec<OfficialDriver>> {
    let url = format!("{}/{}", DRIVER_PORTAL_BASE, model_code);

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) MiControl/0.1")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| HardwareError::Other(format!("HTTP client build failed: {e}")))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| HardwareError::Other(format!("Failed to fetch driver page: {e}")))?;

    if !response.status().is_success() {
        return Err(HardwareError::Other(format!(
            "Xiaomi driver portal returned HTTP {}",
            response.status()
        )));
    }

    let html = response
        .text()
        .await
        .map_err(|e| HardwareError::Other(format!("Failed to read page body: {e}")))?;

    parse_driver_page(&html)
}

/// Compare installed drivers against official Xiaomi drivers.
///
/// Returns a list of drivers where the official version appears newer
/// than the installed version.
pub async fn check_driver_updates() -> HardwareResult<DriverUpdateCheck> {
    let model_code = detect_model_code()?;
    let official = fetch_official_drivers(&model_code).await?;
    let installed = crate::hw::update::get_drivers_detail()?;

    let mut updates = Vec::new();
    for off in &official {
        for inst in &installed {
            if drivers_match(off, inst) {
                if version_is_newer(&off.version, &inst.driver_version) {
                    updates.push(DriverUpdate {
                        device_name: inst.device_name.clone(),
                        installed_version: inst.driver_version.clone(),
                        official_version: off.version.clone(),
                        download_url: off.download_url.clone(),
                        category: off.category.clone(),
                        official_name: off.name.clone(),
                    });
                }
            }
        }
    }

    Ok(DriverUpdateCheck {
        model_code,
        official_drivers: official,
        installed_drivers: installed,
        updates_available: updates,
    })
}

/// Download a driver package to a temp directory.
///
/// Returns the path to the downloaded file.
pub async fn download_driver_package(url: &str) -> HardwareResult<std::path::PathBuf> {
    let temp_dir = std::env::temp_dir().join("micontrol_drivers");
    std::fs::create_dir_all(&temp_dir).map_err(|e| HardwareError::Io(e))?;

    let filename = url.rsplit('/').next().unwrap_or("driver.zip");
    let dest = temp_dir.join(filename);

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) MiControl/0.1")
        .timeout(std::time::Duration::from_secs(300)) // 5 min for large files
        .build()
        .map_err(|e| HardwareError::Other(format!("HTTP client: {e}")))?;

    let bytes = client
        .get(url)
        .send()
        .await
        .map_err(|e| HardwareError::Other(format!("Download failed: {e}")))?
        .bytes()
        .await
        .map_err(|e| HardwareError::Other(format!("Read bytes: {e}")))?;

    std::fs::write(&dest, &bytes).map_err(|e| HardwareError::Io(e))?;

    log::info!(
        "Downloaded driver package: {} ({} bytes)",
        dest.display(),
        bytes.len()
    );

    Ok(dest)
}

// ── HTML parsing ─────────────────────────────────────────────────────────────

/// Parse the Xiaomi driver portal HTML page to extract driver entries.
///
/// The page contains download links in `<a href="...">` tags pointing to
/// CDN URLs like `cdn.cnbj1.fds.api.mi-img.com` or `mibook.cdn.pc.mi.com`.
/// Driver names and versions are in nearby text elements.
fn parse_driver_page(html: &str) -> HardwareResult<Vec<OfficialDriver>> {
    let mut drivers = Vec::new();

    // Find all download URLs — they match these CDN patterns
    let cdn_patterns = ["cdn.cnbj1.fds.api.mi-img.com", "mibook.cdn.pc.mi.com"];

    // Extract all <a> tags with href containing CDN URLs
    for line in html.lines() {
        let trimmed = line.trim();

        // Look for href attributes with CDN URLs
        for pattern in &cdn_patterns {
            if let Some(idx) = trimmed.find(pattern) {
                // Extract the full URL
                if let Some(url) = extract_url_from_line(&trimmed[idx..]) {
                    // Try to find a driver name and version near this link
                    let (name, version, date) = extract_driver_metadata(html, &url);

                    if !url.is_empty() {
                        let category = categorize_driver(&name, &url);
                        drivers.push(OfficialDriver {
                            category,
                            name: name.clone(),
                            version,
                            date,
                            download_url: url,
                            file_size: None,
                        });
                    }
                    break;
                }
            }
        }
    }

    // Deduplicate by download_url
    drivers.dedup_by(|a, b| a.download_url == b.download_url);

    if drivers.is_empty() {
        log::warn!("No drivers found on the portal page — the HTML structure may have changed");
    }

    Ok(drivers)
}

/// Extract a URL from a string starting at the URL position.
fn extract_url_from_line(s: &str) -> Option<String> {
    // Find the start of the URL (should be after href=" or src=")
    let start = s.find("http").or_else(|| s.find("https"))?;
    let rest = &s[start..];

    // URL ends at the next quote or space
    let end = rest
        .find('"')
        .or_else(|| rest.find('\''))
        .or_else(|| rest.find(' '))
        .unwrap_or(rest.len());

    Some(rest[..end].to_string())
}

/// Try to extract driver name, version, and date from the HTML near a download link.
fn extract_driver_metadata(html: &str, url: &str) -> (String, String, String) {
    // Find the position of this URL in the HTML
    let pos = match html.find(url) {
        Some(p) => p,
        None => return (String::new(), String::new(), String::new()),
    };

    // Look backwards from the URL for a name (usually in a <span> or <div> before the link)
    let before = &html[..pos];
    let name = extract_last_text(before, 500);

    // Try to extract version from the URL itself (common pattern: version in filename)
    let version = extract_version_from_url(url).or_else(|| extract_version_from_text(&name));

    // Try to extract date from URL (pattern: /YYYYMMDD/)
    let date = extract_date_from_url(url);

    (name, version.unwrap_or_default(), date.unwrap_or_default())
}

/// Extract the last text content from HTML before a position.
fn extract_last_text(html: &str, max_lookback: usize) -> String {
    let start = html.len().saturating_sub(max_lookback);
    let slice = &html[start..];

    // Find the last text content between > and <
    let mut last_text = String::new();
    for segment in slice.rsplit('<') {
        if let Some(end) = segment.find('>') {
            let text = &segment[end + 1..];
            let cleaned = text.trim();
            if !cleaned.is_empty() && !cleaned.starts_with("http") {
                last_text = cleaned.to_string();
                break;
            }
        }
    }

    // Clean up HTML entities
    last_text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Try to extract a version string from a URL.
/// Common patterns: "23.170.0.1G", "31.0.101.5333", "1.0.0.1"
fn extract_version_from_url(url: &str) -> Option<String> {
    // Look for version-like patterns in the URL
    let parts: Vec<&str> = url.split('/').collect();
    for part in parts {
        // Version pattern: digits separated by dots, possibly with letters
        if part.chars().filter(|c| *c == '.').count() >= 2 {
            // Check if it looks like a version (starts with digit)
            if part.starts_with(|c: char| c.is_ascii_digit()) {
                // Extract just the version part (before any underscore)
                let version = part.split('_').next().unwrap_or(part);
                return Some(version.to_string());
            }
        }
    }
    None
}

/// Try to extract a version from text like "Version: 23.170.0.1G"
fn extract_version_from_text(text: &str) -> Option<String> {
    // Look for patterns like "XX.XX.XX.X" or "XX.XX.XX"
    let mut found = None;
    for word in text.split_whitespace() {
        if word.chars().filter(|c| *c == '.').count() >= 2
            && word.starts_with(|c: char| c.is_ascii_digit())
        {
            found = Some(word.trim_end_matches(',').to_string());
            break;
        }
    }
    found
}

/// Extract a date from URL patterns like "/20260612/"
fn extract_date_from_url(url: &str) -> Option<String> {
    // Look for 8-digit date patterns (YYYYMMDD)
    for part in url.split('/') {
        if part.len() == 8 && part.chars().all(|c| c.is_ascii_digit()) {
            let year = &part[..4];
            let month = &part[4..6];
            let day = &part[6..8];
            return Some(format!("{}-{}-{}", year, month, day));
        }
    }
    None
}

/// Categorize a driver based on its name and URL.
fn categorize_driver(name: &str, url: &str) -> String {
    let combined = format!("{} {}", name, url).to_lowercase();

    if combined.contains("bios") {
        "BIOS".to_string()
    } else if combined.contains("wlan")
        || combined.contains("wifi")
        || combined.contains("wireless")
    {
        "WLAN".to_string()
    } else if combined.contains("gpu")
        || combined.contains("gfx")
        || combined.contains("graphics")
        || combined.contains("intel_mtl")
        || combined.contains("nvidia")
        || combined.contains("display")
    {
        "GPU".to_string()
    } else if combined.contains("audio") || combined.contains("sound") {
        "Audio".to_string()
    } else if combined.contains("bluetooth") || combined.contains("bt") {
        "Bluetooth".to_string()
    } else if combined.contains("chipset") {
        "Chipset".to_string()
    } else if combined.contains("fingerprint") || combined.contains("fps") {
        "Fingerprint".to_string()
    } else if combined.contains("camera") || combined.contains("mep") {
        "Camera".to_string()
    } else if combined.contains("nfc") {
        "NFC".to_string()
    } else if combined.contains("thermal") || combined.contains("dtt") {
        "Thermal".to_string()
    } else if combined.contains("npu") || combined.contains("vpu") || combined.contains("gna") {
        "NPU/VPU".to_string()
    } else if combined.contains("me") || combined.contains("management engine") {
        "ME".to_string()
    } else if combined.contains("serial io") || combined.contains("serialio") {
        "Serial IO".to_string()
    } else if combined.contains("ish") {
        "ISH".to_string()
    } else if combined.contains("hdr") {
        "HDR".to_string()
    } else if combined.contains("recovery") || combined.contains("image") {
        "Recovery".to_string()
    } else if combined.contains("pcmanager") || combined.contains("xiaomipcmanager") {
        "Xiaomi PC Manager".to_string()
    } else if combined.contains("app") {
        "Application".to_string()
    } else {
        "Other".to_string()
    }
}

// ── Driver matching ──────────────────────────────────────────────────────────

/// Check if an official driver entry matches an installed driver.
///
/// Uses fuzzy matching on device name and category to find corresponding pairs.
fn drivers_match(official: &OfficialDriver, installed: &DriverDetail) -> bool {
    let off_name = official.name.to_lowercase();
    let inst_name = installed.device_name.to_lowercase();
    let inst_provider = installed.driver_provider_name.to_lowercase();
    let off_cat = official.category.to_lowercase();

    // Match by category + provider
    if off_cat.contains("wlan")
        && (inst_name.contains("wifi")
            || inst_name.contains("wireless")
            || inst_name.contains("wlan"))
    {
        return true;
    }
    if off_cat.contains("gpu")
        && (inst_name.contains("intel")
            || inst_name.contains("nvidia")
            || inst_name.contains("display")
            || inst_name.contains("graphics"))
    {
        return true;
    }
    if off_cat.contains("audio") && (inst_name.contains("audio") || inst_name.contains("sound")) {
        return true;
    }
    if off_cat.contains("bluetooth") && inst_name.contains("bluetooth") {
        return true;
    }
    if off_cat.contains("chipset")
        && (inst_name.contains("chipset") || inst_name.contains("system"))
    {
        return true;
    }

    // Match by provider name
    if inst_provider.contains("intel") && off_name.contains("intel") {
        // More specific: match on device type
        if (off_name.contains("ax") && inst_name.contains("ax"))
            || (off_name.contains("graphics") && inst_name.contains("graphics"))
        {
            return true;
        }
    }

    false
}

/// Compare two version strings and return true if `official` is newer than `installed`.
///
/// Handles version formats like "23.170.0.1G", "31.0.101.5333", "1.0.0.1".
fn version_is_newer(official: &str, installed: &str) -> bool {
    let off_parts = parse_version_parts(official);
    let inst_parts = parse_version_parts(installed);

    if off_parts.is_empty() || inst_parts.is_empty() {
        return false; // Can't compare — don't flag as update
    }

    for (o, i) in off_parts.iter().zip(inst_parts.iter()) {
        if o > i {
            return true;
        }
        if o < i {
            return false;
        }
    }

    // If all compared parts are equal, official is not newer
    // (unless official has more parts, which might indicate a newer version)
    off_parts.len() > inst_parts.len()
}

/// Parse a version string into comparable numeric parts.
fn parse_version_parts(version: &str) -> Vec<u64> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<u64>().ok())
        .collect()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_is_newer() {
        assert!(version_is_newer("23.170.0.2", "23.170.0.1"));
        assert!(version_is_newer("24.0.0.0", "23.170.0.1"));
        assert!(!version_is_newer("23.170.0.1", "23.170.0.1"));
        assert!(!version_is_newer("23.170.0.0", "23.170.0.1"));
    }

    #[test]
    fn test_version_with_suffix() {
        // "23.170.0.1G" should parse as [23, 170, 0, 1]
        assert!(version_is_newer("23.170.0.2G", "23.170.0.1G"));
    }

    #[test]
    fn test_extract_version_from_url() {
        let url = "https://cdn.cnbj1.fds.api.mi-img.com/mibook-drivers/Driver/N56N57/20260612/12.Intel_AX211_WLAN_23.170.0.1G_ICPS_40.25.926.173.zip";
        let version = extract_version_from_url(url);
        assert!(version.is_some());
        // Should find "23.170.0.1G" or similar
        assert!(version.unwrap().contains("23.170"));
    }

    #[test]
    fn test_extract_date_from_url() {
        let url =
            "https://cdn.cnbj1.fds.api.mi-img.com/mibook-drivers/Driver/N56N57/20260612/driver.zip";
        let date = extract_date_from_url(url);
        assert_eq!(date.as_deref(), Some("2026-06-12"));
    }

    #[test]
    fn test_categorize_driver() {
        assert_eq!(categorize_driver("Intel AX211 WLAN Driver", ""), "WLAN");
        assert_eq!(categorize_driver("BIOS Update", ""), "BIOS");
        assert_eq!(categorize_driver("Intel Graphics Driver", ""), "GPU");
        assert_eq!(categorize_driver("Realtek Audio Driver", ""), "Audio");
        assert_eq!(categorize_driver("Bluetooth Driver", ""), "Bluetooth");
    }

    #[test]
    fn test_parse_version_parts() {
        assert_eq!(parse_version_parts("23.170.0.1G"), vec![23, 170, 0, 1]);
        assert_eq!(parse_version_parts("1.0.0.1"), vec![1, 0, 0, 1]);
        assert_eq!(parse_version_parts("31.0.101.5333"), vec![31, 0, 101, 5333]);
    }
}
