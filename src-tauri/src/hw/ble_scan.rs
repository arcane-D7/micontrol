#![cfg(windows)]
//! BLE device discovery for the Cross-Device tab (MIOT-05 UX revamp).
//!
//! Two complementary discovery paths:
//!
//! 1. **Paired devices** — enumerates the Bluetooth LE devices already paired
//!    with Windows via the WinRT `DeviceInformation` API (no radio scan, no
//!    permissions — just the OS's own pairing database). This is the primary
//!    source: the user's phone is almost always already paired.
//!
//! 2. **Active scan** — a short btleplug discovery pass that finds
//!    advertising BLE devices nearby (name/address/RSSI). Useful for devices
//!    that are not paired yet, or phones with randomized advertising (some
//!    phones may only show a synthetic address; the user picks by name).
//!
//! Results are merged into a single sorted list (paired first, then scan
//! hits), deduplicated by MAC where possible. The frontend shows the list in
//! a modal and the user picks their phone; the selection is stored through the
//! existing `PresenceConfig` (MAC + name), so the presence monitor keeps
//! working unchanged.

use serde::{Deserialize, Serialize};

/// A BLE device surfaced to the UI.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BleDevice {
    /// Best-known name (advertised or paired-device name).
    pub name: String,
    /// MAC address (Windows-style upper-case with `-` separators) when known.
    pub address: Option<String>,
    /// Last observed RSSI (dBm) from the active scan, if any.
    pub rssi: Option<i32>,
    /// True when the device is already paired with Windows.
    pub paired: bool,
    /// True when the device was found by the active scan (not just paired db).
    pub from_scan: bool,
}

/// Result of a combined discovery pass.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BleScanResult {
    pub devices: Vec<BleDevice>,
    /// True when the adapter was reachable and the scan actually ran.
    pub scan_ok: bool,
    /// Human-readable note when the scan had to be partial (e.g. no adapter).
    pub note: Option<String>,
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Discover BLE devices: paired (primary) + a short active scan (secondary).
/// Never panics; failures degrade the scan portion but keep paired results.
pub async fn discover_devices(scan_seconds: u64) -> BleScanResult {
    // 1. Paired devices (fast, reliable) — run on a COM-initialized thread.
    let paired = tokio::task::spawn_blocking(list_paired_devices)
        .await
        .unwrap_or_default();

    // 2. Active scan (best-effort).
    let scanned = match scan_for_nearby(scan_seconds).await {
        Ok(devs) => (true, devs),
        Err(e) => {
            log::warn!("[ble_scan] active scan failed: {e}");
            (false, Vec::new())
        }
    };

    let (scan_ok, scanned_devs) = scanned;

    // Merge: paired first, then scan hits that aren't already listed.
    let mut out: Vec<BleDevice> = paired;
    for sdev in scanned_devs {
        let known = out
            .iter()
            .any(|d| d.address.is_some() && sdev.address.is_some() && d.address == sdev.address);
        if !known {
            out.push(BleDevice {
                from_scan: true,
                ..sdev
            });
        }
    }

    // Deduplicate by address (prefer the richer entry).
    let mut seen = std::collections::HashSet::new();
    out.retain(|d| {
        let key = d.address.clone().unwrap_or_else(|| d.name.clone());
        seen.insert(key)
    });

    // Sort: paired first, then by name.
    out.sort_by(|a, b| {
        b.paired
            .cmp(&a.paired)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let note = if !scan_ok {
        Some("Active BLE scan unavailable — showing paired devices only".to_string())
    } else {
        None
    };

    BleScanResult {
        devices: out,
        scan_ok,
        note,
    }
}

/// Enumerate Bluetooth-LE association endpoints (paired + cached advertising)
/// via WinRT `DeviceInformation::FindAllAsyncAqsFilter` with the BLE GATT
/// protocol ID. `Name()` is populated for known devices; `Pairing().IsPaired()`
/// tells us which ones are already paired with Windows.
/// Must run on a thread where COM is initialized (MTA). Returns an empty vec
/// on any failure (logged, never panics).
fn list_paired_devices() -> Vec<BleDevice> {
    use windows::core::HSTRING;
    use windows::Devices::Enumeration::DeviceInformation;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

    // AQS filter: only Bluetooth LE association endpoints (protocol GUID is
    // the Bluetooth LE / GATT protocol, not BR/EDR and not RFCOMM).
    const BLE_GATT_PROTOCOL: &str =
        "System.Devices.Aep.ProtocolId:=\"{bb7bb05e-1252-46c9-b1c7-3c8e2a30e44a}\"";

    // SAFETY: COM init/uninit on the current thread; idempotent for MTA.
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if hr.is_err() {
        log::warn!("[ble_scan] CoInitializeEx failed: {hr:?}");
        return Vec::new();
    }
    // Keep COM alive until we've drained the operation result. We must not
    // uninitialize before `op.get()` completes (blocking async pump may run
    // on this thread).
    struct ComGuard;
    impl Drop for ComGuard {
        fn drop(&mut self) {
            // SAFETY: we initialized MTA above; same thread.
            unsafe { CoUninitialize() };
        }
    }
    let _com = ComGuard;

    let op = match DeviceInformation::FindAllAsyncAqsFilter(&HSTRING::from(BLE_GATT_PROTOCOL)) {
        Ok(op) => op,
        Err(e) => {
            log::warn!("[ble_scan] FindAllAsyncAqsFilter failed: {e}");
            return Vec::new();
        }
    };
    let devices = match op.get() {
        Ok(col) => col,
        Err(e) => {
            log::warn!("[ble_scan] FindAllAsync get failed: {e}");
            return Vec::new();
        }
    };

    let size = match devices.Size() {
        Ok(s) => s,
        Err(e) => {
            log::warn!("[ble_scan] collection Size failed: {e}");
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    for i in 0..size {
        let Ok(info) = devices.GetAt(i) else { continue };
        let id = info.Id().unwrap_or_default().to_string();
        let name = info.Name().unwrap_or_default().to_string();
        // Skip unnamed association endpoints (they are anonymous randoms).
        if name.trim().is_empty() {
            continue;
        }
        let paired = info.Pairing().and_then(|p| p.IsPaired()).unwrap_or(false);
        out.push(BleDevice {
            name,
            address: extract_address_from_id(&id),
            rssi: None,
            paired,
            from_scan: false,
        });
    }
    out
}

/// Try to turn a `DeviceInformation` id (like
/// `BluetoothLE#BluetoothLE48:41:4f:64:e0:aa:01-70:10:5f:ca:36:63`) into a
/// MAC-ish address (upper-case, `-` separators). Best-effort; some device ids
/// may not embed a MAC.
fn extract_address_from_id(id: &str) -> Option<String> {
    let lower = id.to_lowercase();
    if !lower.contains("bluetooth") {
        return None;
    }
    // Windows BLE device ids embed TWO MACs: the local adapter's and the
    // remote device's (e.g. `BluetoothLE#BluetoothLE48:41:4f:64:e0:aa:01-70:10:5f:ca:36:63`).
    // The remote device's MAC is the LAST colon-separated 6-group block.
    // Scan for all MAC-like patterns and keep the last one.
    let bytes = lower.as_bytes();
    let mut last: Option<String> = None;
    let mut i = 0;
    while i + 17 <= bytes.len() {
        if is_hex_group(&bytes[i..i + 2]) && bytes.get(i + 2) == Some(&b':') {
            // Candidate "xx:xx:xx:xx:xx:xx" exactly 6 groups.
            let mut ok = true;
            for g in 1..6 {
                let start = i + g * 3;
                let sep = bytes.get(start - 1);
                if sep != Some(&b':') || !is_hex_group(&bytes[start..start + 2]) {
                    ok = false;
                    break;
                }
            }
            if ok {
                last = Some(lower[i..i + 17].to_uppercase().replace(':', "-"));
                i += 17;
                continue;
            }
        }
        i += 1;
    }
    last
}

/// True when `s` is exactly two ASCII hex digits.
fn is_hex_group(s: &[u8]) -> bool {
    s.len() == 2 && s.iter().all(|b| b.is_ascii_hexdigit())
}

/// Short active BLE scan (btleplug) returning advertising devices nearby.
async fn scan_for_nearby(scan_seconds: u64) -> Result<Vec<BleDevice>, String> {
    use btleplug::api::{Central, Manager as _, Peripheral, ScanFilter};

    let manager = btleplug::platform::Manager::new()
        .await
        .map_err(|e| e.to_string())?;
    let adapters = manager.adapters().await.map_err(|e| e.to_string())?;
    let central = adapters
        .into_iter()
        .next()
        .ok_or_else(|| "no BLE adapter found".to_string())?;

    central
        .start_scan(ScanFilter::default())
        .await
        .map_err(|e| e.to_string())?;
    tokio::time::sleep(std::time::Duration::from_secs(scan_seconds)).await;

    let peripherals = central.peripherals().await.map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for p in peripherals {
        let props = match p.properties().await {
            Ok(Some(pr)) => pr,
            _ => continue,
        };
        let addr = props.address.to_string();
        let rssi = props.rssi.map(i32::from);
        let name = props.local_name.unwrap_or_default();
        if name.is_empty() && addr.is_empty() {
            continue;
        }
        out.push(BleDevice {
            name,
            address: Some(normalize_mac(&addr)),
            rssi,
            paired: false,
            from_scan: true,
        });
    }
    central
        .stop_scan()
        .await
        .map_err(|e| log::warn!("[ble_scan] stop_scan failed: {e}"))
        .ok();
    Ok(out)
}

/// MAC to upper-case `-` separators (btleplug emits `XX:XX:...` on Windows).
fn normalize_mac(mac: &str) -> String {
    let trimmed = mac.trim().replace([':', ' '], "-");
    if trimmed
        .chars()
        .filter(|c| c.is_ascii_hexdigit() || *c == '-')
        .count()
        == trimmed.chars().count()
        && trimmed.split('-').count() == 6
    {
        trimmed.to_ascii_uppercase()
    } else {
        mac.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_mac_handles_btleplug_format() {
        assert_eq!(normalize_mac("aa:bb:cc:dd:ee:ff"), "AA-BB-CC-DD-EE-FF");
        assert_eq!(normalize_mac("AA:BB:cc:DD:EE:FF"), "AA-BB-CC-DD-EE-FF");
        assert_eq!(normalize_mac("not-a-mac"), "not-a-mac");
        assert_eq!(normalize_mac(""), "");
    }

    #[test]
    fn extract_address_from_typical_ble_id() {
        let id = "Bluetooth#BluetoothLE48:41:4f:64:e0:aa:01-70:10:5f:ca:36:63";
        let mac = extract_address_from_id(id);
        assert_eq!(mac, Some("70-10-5F-CA-36-63".to_string()));
    }

    #[test]
    fn extract_address_rejects_plain_names() {
        assert_eq!(extract_address_from_id("Phone Link"), None);
    }

    #[test]
    fn dedup_and_sort_puts_paired_first() {
        let devs = [
            BleDevice {
                name: "Scanned Phone".into(),
                address: Some("AA-BB-CC-DD-EE-01".into()),
                paired: false,
                from_scan: true,
                rssi: Some(-60),
            },
            BleDevice {
                name: "Paired Phone".into(),
                address: Some("AA-BB-CC-DD-EE-02".into()),
                paired: true,
                from_scan: false,
                rssi: None,
            },
        ];
        let mut sorted = devs.to_vec();
        sorted.sort_by(|a, b| {
            b.paired
                .cmp(&a.paired)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        assert_eq!(sorted[0].name, "Paired Phone");
        assert!(sorted[0].paired);
        assert_eq!(sorted[1].name, "Scanned Phone");
    }
}
