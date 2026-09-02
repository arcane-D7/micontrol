#![cfg(windows)]
//! BLE advertising (PC discoverability) — MIOT-38 Round 3.
//!
//! Windows can advertise a Bluetooth LE service with
//! `BluetoothLEAdvertisementPublisher`, which makes this PC visible to phones
//! scanning for BLE devices in range. By default MiControl does NOT advertise
//! (privacy/battery); when enabled (registry `BLE_ADVERTISE_KEY` "Enabled"),
//! the publisher announces:
//!
//!   - Advertising type: connectable undirected (`BluetoothLEAdvertisementType::ConnectableUndirected`)
//!   - Local name: "MiControl-PC" (data section type 0x09, Complete Local Name)
//!   - A manufacturer-specific marker so MiControl can identify itself:
//!     company id 0x0453 (Micro-Star International) with payload `b"MC"`.
//!     (Company id is informational only; the phone just sees a BLE device.)
//!
//! The publisher is started/stopped on a dedicated thread that owns a
//! single-threaded COM apartment (WinRT requires one). Because Tauri's async
//! runtime is multi-threaded, we never touch WinRT objects from it — the
//! publisher is owned by the advertising thread and reported through a plain
//! shared status.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use windows::Devices::Bluetooth::Advertisement::{
    BluetoothLEAdvertisementDataSection, BluetoothLEAdvertisementDataTypes,
    BluetoothLEAdvertisementPublisher, BluetoothLEAdvertisementPublisherStatus,
};
use windows::Storage::Streams::DataWriter;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

const CONFIG_KEY: &str = r"SOFTWARE\MiControl\BleAdvertise";
const K_ENABLED: &str = "Enabled";
const K_NAME: &str = "Name";

/// Default advertised local name (data section type 9 = Complete Local Name).
pub const DEFAULT_ADV_NAME: &str = "MiControl-PC";

/// Manufacturer company id (Micro-Star International, informational only).
const COMPANY_ID: u16 = 0x0453;

/// Custom manufacturer payload so the app can tell its own advertisement.
const MAGIC: &[u8] = b"MC";

/// Shared advertising state.
struct AdvertiseState {
    running: AtomicBool,
    enabled: AtomicBool,
}

static STATE: OnceLock<AdvertiseState> = OnceLock::new();

fn state() -> &'static AdvertiseState {
    STATE.get_or_init(|| AdvertiseState {
        running: AtomicBool::new(false),
        enabled: AtomicBool::new(false),
    })
}

/// Registry-backed configuration for BLE advertising.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BleAdvertiseConfig {
    pub enabled: bool,
    pub name: String,
}

impl Default for BleAdvertiseConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            name: DEFAULT_ADV_NAME.into(),
        }
    }
}

impl BleAdvertiseConfig {
    pub fn load() -> Self {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_CURRENT_USER;

        let mut cfg = Self::default();
        let Ok(Some(key)) = RegKeyGuard::open_read(HKEY_CURRENT_USER, CONFIG_KEY) else {
            return cfg;
        };
        cfg.enabled = key
            .read_u32(K_ENABLED)
            .ok()
            .flatten()
            .map(|v| v != 0)
            .unwrap_or(false);
        cfg.name = key
            .read_string(K_NAME)
            .ok()
            .flatten()
            .filter(|v| !v.is_empty() && v.len() <= 40)
            .unwrap_or_else(|| DEFAULT_ADV_NAME.into());
        cfg
    }

    pub fn save(&self) {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_CURRENT_USER;

        if let Ok(key) = RegKeyGuard::create_write(HKEY_CURRENT_USER, CONFIG_KEY) {
            let _ = key.write_u32(K_ENABLED, self.enabled as u32);
            let _ = key.write_string(K_NAME, &self.name);
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.save();
        if enabled {
            start_advertising();
        } else {
            stop_advertising();
        }
    }
}

/// True while the publisher thread is running.
pub fn is_advertising() -> bool {
    state().running.load(Ordering::SeqCst) && state().enabled.load(Ordering::SeqCst)
}

/// Read the current config (for the UI toggle).
pub fn get_config() -> BleAdvertiseConfig {
    BleAdvertiseConfig::load()
}

/// Start the BLE advertiser. Idempotent. Spawns a dedicated thread that owns
/// its own COM apartment so WinRT calls stay on a single thread.
pub fn start_advertising() {
    let cfg = BleAdvertiseConfig::load();
    if !cfg.enabled {
        state().enabled.store(false, Ordering::SeqCst);
        return;
    }
    if state().running.swap(true, Ordering::SeqCst) {
        return;
    }
    let name = cfg.name.clone();
    std::thread::Builder::new()
        .name("ble-advertise".into())
        .spawn(move || advertise_loop(name))
        .expect("failed to spawn BLE advertise thread");
}

/// Stop the publisher (the thread self-terminates because the publisher is
/// dropped when `advertise_loop` exits).
pub fn stop_advertising() {
    state().enabled.store(false, Ordering::SeqCst);
    // The loop checks `enabled` on each iteration and exits; `running` flips
    // back to false in `advertise_loop`'s `finally`.
}

/// Builder thread. Owns the publisher for as long as advertising is enabled.
fn advertise_loop(name: String) {
    // CoInitializeEx is idempotent per-thread for the same apartment model.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let com_uninit = || unsafe { CoUninitialize() };

    let Ok(publisher) = BluetoothLEAdvertisementPublisher::new() else {
        log::warn!("[ble_advertise] BluetoothLEAdvertisementPublisher::new failed");
        state().running.store(false, Ordering::SeqCst);
        com_uninit();
        return;
    };

    // Prepare the advertisement once. The publisher is connectable-undirected
    // by default; we just fill in the data sections (local name + manufacturer).
    let prepared = build_advertisement(&name);
    let Ok(_) = publisher.Advertisement().and_then(|adv| {
        if let Some(section) = prepared {
            let vec = adv.DataSections()?;
            vec.Append(&section.0)?;
            vec.Append(&section.1)?;
        }
        Ok(())
    }) else {
        log::warn!("[ble_advertise] failed to configure advertisement");
        state().running.store(false, Ordering::SeqCst);
        com_uninit();
        return;
    };

    if let Err(e) = publisher.Start() {
        log::warn!("[ble_advertise] publisher.Start failed: {e}");
        state().running.store(false, Ordering::SeqCst);
        com_uninit();
        return;
    }

    let status = publisher
        .Status()
        .unwrap_or(BluetoothLEAdvertisementPublisherStatus::Created);
    match status {
        BluetoothLEAdvertisementPublisherStatus::Started => {
            log::info!("[ble_advertise] publisher Started — PC now BLE-discoverable as '{name}'");
        }
        BluetoothLEAdvertisementPublisherStatus::Aborted => {
            log::warn!("[ble_advertise] publisher Aborted (radio off / not supported)");
            let _ = publisher.Stop();
            state().running.store(false, Ordering::SeqCst);
            com_uninit();
            return;
        }
        _ => {
            log::debug!("[ble_advertise] publisher Status after Start: {status:?}");
        }
    }

    state().enabled.store(true, Ordering::SeqCst);

    // Keep the publisher alive (and Windows advertising) until disabled.
    // Re-check periodically; a radio toggle may abort the publisher silently.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(5));
        if !state().enabled.load(Ordering::SeqCst) {
            let _ = publisher.Stop();
            log::info!("[ble_advertise] stopped by config change");
            break;
        }
        if state().running.load(Ordering::SeqCst)
            && publisher.Status().ok() == Some(BluetoothLEAdvertisementPublisherStatus::Aborted)
        {
            log::warn!("[ble_advertise] publisher aborted by OS — retrying in 10s");
            let _ = publisher.Stop();
            std::thread::sleep(std::time::Duration::from_secs(10));
            let _ = publisher.Start();
        }
    }

    state().running.store(false, Ordering::SeqCst);
    com_uninit();
}

/// Build the advertisement data sections: `(local name section, manufacturer section)`.
fn build_advertisement(
    name: &str,
) -> Option<(
    BluetoothLEAdvertisementDataSection,
    BluetoothLEAdvertisementDataSection,
)> {
    // Build an `IBuffer` from bytes via `DataWriter` (no extra deps).
    fn buffer_from(bytes: &[u8]) -> Option<windows::Storage::Streams::IBuffer> {
        let writer = DataWriter::new().ok()?;
        writer.WriteBytes(bytes).ok()?;
        writer.DetachBuffer().ok()
    }

    // 0x09 = Complete Local Name.
    let name_bytes = name.as_bytes();
    let name_section = BluetoothLEAdvertisementDataSection::new().ok()?;
    name_section
        .SetDataType(BluetoothLEAdvertisementDataTypes::CompleteLocalName().ok()?)
        .ok()?;
    if let Some(buf) = buffer_from(name_bytes) {
        name_section.SetData(&buf).ok()?;
    }

    // Manufacturer data: company id (LE order) + magic payload.
    let mut payload = Vec::with_capacity(2 + MAGIC.len());
    payload.push((COMPANY_ID & 0xFF) as u8);
    payload.push(((COMPANY_ID >> 8) & 0xFF) as u8);
    payload.extend_from_slice(MAGIC);

    let mfr = BluetoothLEAdvertisementDataSection::new().ok()?;
    mfr.SetDataType(0xFF).ok()?;
    if let Some(buf) = buffer_from(&payload) {
        mfr.SetData(&buf).ok()?;
    }
    Some((name_section, mfr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_disables_advertising() {
        let c = BleAdvertiseConfig::default();
        assert!(!c.enabled);
        assert_eq!(c.name, DEFAULT_ADV_NAME);
    }

    #[test]
    fn magic_payload_has_company_and_marker() {
        let mut payload = vec![0x53, 0x04, b'M', b'C'];
        let company = u16::from_le_bytes([payload[0], payload[1]]);
        assert_eq!(company, COMPANY_ID);
        assert_eq!(&payload[2..], MAGIC);
        payload.clear();
    }
}
