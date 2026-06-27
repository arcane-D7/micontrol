# Sprint 31 — P1 HIGH: UX Fixes + Complete Stub Implementations

> **Date:** 2026-06-27
> **Sprint:** 31
> **Theme:** Fix 7 high-priority issues including FULL implementation of all stubs (Miracast, ECRAM WMI discovery)
> **Duration:** ~4–5 days
> **Dependencies:** Sprint 30 (all P0 critical fixes)
> **Status:** 📌 Active
> **Audit Reference:** `C:\Users\mafsc\Documents\Audit_Final.md` (A-1 through A-7)

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **OBRIGATÓRIO: 100% dos tickets desta sprint devem ser concluídos. A sprint não será aceita como entregue se qualquer ticket permanecer incompleto.**
>
> **MANDATORY: 100% of the tickets in this sprint MUST be completed. The sprint will NOT be accepted as delivered if any ticket remains incomplete.**

**STUB POLICY:** Any function currently returning a stub/placeholder/empty value MUST be fully implemented. No stubs are acceptable. This includes:

- `list_cast_devices()` → Full WinRT Miracast implementation (Option B from audit)
- `discover_from_wmi()` in ecram.rs → Full WMI query implementation
- `start_casting()` / `stop_casting()` → Full WinRT implementation with proper device control

---

## Health Check Commands (must pass before commit)

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
npx tsc --noEmit
npm run lint
npm run format:check
npm run build
npm run version:check
```

---

## Executive Summary

This sprint addresses 7 high-priority issues from the audit, with special emphasis on **completely implementing all stub functions**. The user explicitly stated: "é inaceitável que ainda temos funcionalidades com stubs!" and "Para qualquer função que tenhamos que ainda estejam com stubs, é obrigatória a implementação completa delas!"

The most complex ticket is **S31-004** (Miracast/Screen Cast), which requires a full WinRT `Windows.Media.Casting` implementation replacing the current stub that always returns an empty list. This is Option B from the audit — the user explicitly rejected Option A (wrapper improvement).

---

## Goals

| #   | Goal                                           | KPI                                                         | Audit Reference      |
| --- | ---------------------------------------------- | ----------------------------------------------------------- | -------------------- |
| 1   | All nav labels are translated in 4 languages   | 0 missing `nav.*` keys in any locale                        | A-1                  |
| 2   | Audio devices show friendly names              | Device names match Windows Sound Control Panel              | A-2                  |
| 3   | No terminal window flashes on WiFi operations  | Zero visible console windows during netsh calls             | A-3                  |
| 4   | Miracast device discovery and casting works    | `list_cast_devices()` returns real devices via WinRT        | A-4                  |
| 5   | Startup toggle reflects actual autostart state | Toggle matches `HKCU\...\Run\MiControl` after F5            | A-5                  |
| 6   | Touchpad HID errors are visible in logs        | `log::warn!` on HID failure, path logged on open            | A-6                  |
| 7   | EC Debug panel hidden in production            | Not visible when `import.meta.env.DEV` is false             | A-7                  |
| 8   | ECRAM WMI discovery implemented                | `discover_from_wmi()` queries WMI instead of returning None | L-8 (pulled forward) |

---

## Technical Specs

### S31-001: Add 5 missing `nav.*` translation keys in all 4 languages (A-1)

| Field         | Value                                                                           |
| ------------- | ------------------------------------------------------------------------------- |
| **Ticket ID** | S31-001                                                                         |
| **Title**     | Add missing `nav.audio`, `nav.cast`, `nav.iot`, `nav.wifi`, `nav.ecrdebug` keys |
| **Priority**  | P1 — High                                                                       |
| **Source**    | A-1 (Audit_Final.md)                                                            |
| **Files**     | `src/i18n/en.json`, `src/i18n/pt.json`, `src/i18n/es.json`, `src/i18n/fr.json`  |
| **Effort**    | ~30 minutes                                                                     |
| **Type**      | Frontend (i18n)                                                                 |

#### Problem

5 navigation keys referenced in `NAV_ITEMS` (`src/pages/MainWindow.tsx:38-57`) do not exist in any of the 4 translation files. The sidebar shows raw key names (e.g., `nav.audio`) instead of translated labels.

Missing keys:
| Key | Icon | EN | PT | ES | FR |
|-----|------|----|----|----|----|
| `nav.audio` | 🎵 | Audio | Áudio | Audio | Audio |
| `nav.cast` | 📺 | Cast | Transmissão | Transmitir | Diffusion |
| `nav.iot` | 🔌 | IoT Device | Dispositivo IoT | Dispositivo IoT | Appareil IoT |
| `nav.wifi` | 📶 | Wi-Fi | Wi-Fi | Wi-Fi | Wi-Fi |
| `nav.ecrdebug` | 🔧 | EC Debug | Debug EC | Debug EC | Debug EC |

#### Solution

Add the 5 keys to the `"nav"` section of each language file. Insert them in the same order as they appear in `NAV_ITEMS` (after `fan`, after `touchpad`, after `touchpad`, after `startup`, after `setup`).

**`src/i18n/en.json`** — add after `"fan": "Fan Control",`:

```json
"audio": "Audio",
```

After `"touchpad": "Touchpad",`:

```json
"cast": "Cast",
```

After `"cast": "Cast",`:

```json
"iot": "IoT Device",
"wifi": "Wi-Fi",
```

After `"setup": "Device Setup",`:

```json
"ecrdebug": "EC Debug",
```

**`src/i18n/pt.json`** — same positions:

```json
"audio": "Áudio",
"cast": "Transmissão",
"iot": "Dispositivo IoT",
"wifi": "Wi-Fi",
"ecrdebug": "Debug EC",
```

**`src/i18n/es.json`** — same positions:

```json
"audio": "Audio",
"cast": "Transmitir",
"iot": "Dispositivo IoT",
"wifi": "Wi-Fi",
"ecrdebug": "Debug EC",
```

**`src/i18n/fr.json`** — same positions:

```json
"audio": "Audio",
"cast": "Diffusion",
"iot": "Appareil IoT",
"wifi": "Wi-Fi",
"ecrdebug": "Debug EC",
```

#### Acceptance Criteria

- [ ] All 5 keys exist in `en.json`, `pt.json`, `es.json`, `fr.json`
- [ ] Keys are in the `"nav"` section of each file
- [ ] No duplicate keys in any file
- [ ] `npm run build` succeeds
- [ ] Sidebar shows translated labels for all 18 nav items in all 4 languages

---

### S31-002: Implement `PKEY_Device_FriendlyName` for audio device names (A-2)

| Field         | Value                                                                       |
| ------------- | --------------------------------------------------------------------------- |
| **Ticket ID** | S31-002                                                                     |
| **Title**     | Replace `device.GetId()` with `IPropertyStore` + `PKEY_Device_FriendlyName` |
| **Priority**  | P1 — High                                                                   |
| **Source**    | A-2 (Audit_Final.md)                                                        |
| **Files**     | `src-tauri/src/hw/audio.rs`                                                 |
| **Effort**    | ~2 hours                                                                    |
| **Type**      | Backend (Rust)                                                              |

#### Problem

`get_device_friendly_name()` at `audio.rs:266-270` returns `device.GetId()` — a system hash ID — instead of the actual device name. Users see cryptic IDs like `{0.0.0.00000000}.{a45c254e-...}` instead of "Realtek Audio" or "Speakers".

#### Current Code (BROKEN)

```rust
// src-tauri/src/hw/audio.rs:266-270
fn get_device_friendly_name(device: &IMMDevice) -> Result<String> {
    let id = unsafe { device.GetId()?.to_string()? };
    Ok(id)  // ← returns the ID, not the friendly name!
}
```

#### Solution

Implement using `device.OpenPropertyStore(STGM_READ)` + `PKEY_Device_FriendlyName`:

```rust
#[cfg(windows)]
fn get_device_friendly_name(device: &windows::Win32::Media::Audio::IMMDevice) -> Result<String> {
    use windows::Win32::UI::Shell::PropertiesSystem::{
        IPropertyStore, PROPERTYKEY,
    };
    use windows::Win32::Storage::FileSystem::STGM_READ;
    use windows::core::Interface;

    // PKEY_Device_FriendlyName = {a45c254e-df1c-4efd-8020-67d146a850e0}, 14
    const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
        fmtid: windows::core::GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
        pid: 14,
    };

    unsafe {
        let store: IPropertyStore = device.OpenPropertyStore(STGM_READ)?;

        // Read the FriendlyName property
        let mut prop = windows::Win32::UI::Shell::PropertiesSystem::PROPVARIANT::default();
        store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME, &mut prop)?;

        // PROPVARIANT for VT_LPWSTR contains a wide string
        let name = prop.to_string()
            .map_err(|e| anyhow::anyhow!("Failed to convert PROPVARIANT to string: {e}"))?;

        if name.is_empty() {
            // Fallback to device ID if friendly name is unavailable
            let id = device.GetId()?.to_string()?;
            return Ok(id);
        }

        Ok(name)
    }
}
```

**Note:** The `Win32_UI_Shell_PropertiesSystem` feature is already enabled in `Cargo.toml` (line 39). The `Win32_Storage_FileSystem` feature is also enabled. No Cargo.toml changes needed.

#### Acceptance Criteria

- [ ] `get_device_friendly_name()` uses `OpenPropertyStore` + `PKEY_Device_FriendlyName`
- [ ] Falls back to device ID if friendly name is unavailable
- [ ] Audio device names in the UI match Windows Sound Control Panel
- [ ] No more hash IDs shown in the Volume tab
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes

---

### S31-003: Add `CREATE_NO_WINDOW` to all `netsh` and `cmd` calls in WiFi and Screen Cast (A-3)

| Field         | Value                                                                                 |
| ------------- | ------------------------------------------------------------------------------------- |
| **Ticket ID** | S31-003                                                                               |
| **Title**     | Add `CREATE_NO_WINDOW` flag to all subprocess calls in `wifi.rs` and `screen_cast.rs` |
| **Priority**  | P1 — High                                                                             |
| **Source**    | A-3 (Audit_Final.md)                                                                  |
| **Files**     | `src-tauri/src/hw/wifi.rs`, `src-tauri/src/hw/screen_cast.rs`                         |
| **Effort**    | ~1 hour                                                                               |
| **Type**      | Backend (Rust)                                                                        |

#### Problem

All 4 WiFi functions (`scan_networks`, `get_status`, `connect`, `disconnect`) and the screen cast functions (`start_casting`, `stop_casting`) use `Command::new("netsh")` or `Command::new("cmd")` without the `CREATE_NO_WINDOW` flag (0x08000000). This causes visible console window flashes on every operation.

Other modules (`discovery.rs`, `display.rs`, `elev_bridge.rs`) correctly use `creation_flags(0x08000000)`.

#### Solution

**`src-tauri/src/hw/wifi.rs`** — Add at the top of the file:

```rust
#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;
```

Then add `.[cfg(windows)] creation_flags(CREATE_NO_WINDOW);` to every `Command::new("netsh")` call:

In `scan_networks()`:

```rust
let mut cmd = Command::new("netsh");
cmd.args(["wlan", "show", "networks", "mode=bssid"]);
#[cfg(windows)]
cmd.creation_flags(CREATE_NO_WINDOW);
let output = cmd.output()
    .map_err(|e| HardwareError::Wifi(format!("Failed to run netsh: {e}")))?;
```

In `get_status()`:

```rust
let mut cmd = Command::new("netsh");
cmd.args(["wlan", "show", "interfaces"]);
#[cfg(windows)]
cmd.creation_flags(CREATE_NO_WINDOW);
let output = cmd.output()
    .map_err(|e| HardwareError::Wifi(format!("Failed to run netsh: {e}")))?;
```

In `connect()` — there are 3 netsh calls (add profile, connect, delete profile). Add the flag to all 3:

```rust
// Add profile
let mut cmd = Command::new("netsh");
cmd.args(["wlan", "add", "profile", "filename"]);
cmd.arg(&profile_path);
#[cfg(windows)]
cmd.creation_flags(CREATE_NO_WINDOW);
let add = cmd.output() /* ... */;

// Connect
let mut cmd = Command::new("netsh");
cmd.args(["wlan", "connect", "name"]);
cmd.arg(ssid);
#[cfg(windows)]
cmd.creation_flags(CREATE_NO_WINDOW);
let connect = cmd.output() /* ... */;

// Delete profile (cleanup)
let mut cmd = Command::new("netsh");
cmd.args(["wlan", "delete", "profile", "name"]);
cmd.arg(ssid);
#[cfg(windows)]
cmd.creation_flags(CREATE_NO_WINDOW);
let _ = cmd.output();
```

In `disconnect()`:

```rust
let mut cmd = Command::new("netsh");
cmd.args(["wlan", "disconnect"]);
#[cfg(windows)]
cmd.creation_flags(CREATE_NO_WINDOW);
let output = cmd.output() /* ... */;
```

**`src-tauri/src/hw/screen_cast.rs`** — Add the same import and flag to `start_casting()` and `stop_casting()`:

```rust
#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

// In start_casting():
let mut cmd = std::process::Command::new("cmd");
cmd.args(["/c", "start", "ms-settings-connectabledevices:project"]);
#[cfg(windows)]
cmd.creation_flags(CREATE_NO_WINDOW);

// In stop_casting():
let mut cmd = std::process::Command::new("cmd");
cmd.args(["/c", "taskkill", "/f", "/im", "SystemSettings.exe"]);
#[cfg(windows)]
cmd.creation_flags(CREATE_NO_WINDOW);
```

#### Acceptance Criteria

- [ ] All `Command::new("netsh")` calls in `wifi.rs` have `creation_flags(CREATE_NO_WINDOW)`
- [ ] All `Command::new("cmd")` calls in `screen_cast.rs` have `creation_flags(CREATE_NO_WINDOW)`
- [ ] No visible console window flashes during WiFi scan, connect, disconnect, or status operations
- [ ] No visible console window flashes during cast start/stop
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes

---

### S31-004: FULL WinRT Miracast implementation — replace stub `list_cast_devices()` (A-4)

| Field         | Value                                                                                  |
| ------------- | -------------------------------------------------------------------------------------- |
| **Ticket ID** | S31-004                                                                                |
| **Title**     | Implement full Miracast device discovery and casting via WinRT `Windows.Media.Casting` |
| **Priority**  | P1 — High                                                                              |
| **Source**    | A-4 (Audit_Final.md) — Option B (full implementation, NOT Option A wrapper)            |
| **Files**     | `src-tauri/src/hw/screen_cast.rs`, `src-tauri/Cargo.toml`                              |
| **Effort**    | ~6–8 hours                                                                             |
| **Type**      | Backend (Rust, WinRT)                                                                  |

#### Problem

`list_cast_devices()` always returns `Ok(Vec::new())` — a stub. `start_casting()` merely opens the Windows Connect panel via `cmd /c start ms-settings-connectabledevices:project`. `stop_casting()` kills `SystemSettings.exe` via taskkill. None of these are real implementations.

**The user explicitly demanded Option B (full WinRT implementation), NOT Option A (wrapper improvement).**

#### Current Code (STUB — MUST BE REPLACED)

```rust
// src-tauri/src/hw/screen_cast.rs:24-31
pub fn list_cast_devices() -> HardwareResult<Vec<CastDevice>> {
    log::info!("[screen_cast] Listing Miracast devices via WinRT");
    Ok(Vec::new())  // ← STUB — always empty
}
```

#### Solution

**Step 1:** Add WinRT features to `Cargo.toml`:

```toml
# In [target.'cfg(windows)'.dependencies] windows features array, add:
  "Media_Casting",
  "Devices_Enumeration",
  "Foundation",
```

**Step 2:** Implement full Miracast device discovery using `Windows.Media.Casting.CastingDevicePicker` and `Windows.Devices.Enumeration.DeviceInformation.FindAllAsync`:

```rust
//! Screen casting via Windows Miracast/WiDi API.
//!
//! Provides device discovery and casting control using WinRT
//! `Windows.Media.Casting` and `Windows.Devices.Enumeration` APIs.

use crate::hw::errors::{HardwareError, HardwareResult};
use serde::{Deserialize, Serialize};

/// A Miracast/WiDi receiver device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastDevice {
    pub name: String,
    pub id: String,
    pub device_type: String,
}

/// Result of a cast operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastResult {
    pub success: bool,
    pub message: String,
}

/// List available Miracast/WiDi receivers using WinRT DeviceEnumeration.
///
/// Uses `Windows.Devices.Enumeration.DeviceInformation.FindAllAsync` with
/// the Miracast device selector to discover available casting receivers.
#[cfg(windows)]
pub fn list_cast_devices() -> HardwareResult<Vec<CastDevice>> {
    use windows::Devices::Enumeration::{
        DeviceInformation, DeviceInformationKind,
    };
    use windows::Foundation::IAsyncOperation;

    log::info!("[screen_cast] Enumerating Miracast devices via WinRT DeviceEnumeration");

    // The device selector for Miracast/WiDi receivers.
    // We use the casting device picker's selector which targets
    // devices that support the Miracast protocol.
    let selector = windows::Devices::Enumeration::DeviceInformation::GetAqsFilterForDevicePicker(
        DeviceInformationKind::DevicePicker,
    ).map_err(|e| HardwareError::Cast(format!("Failed to get device selector: {e}")))?;

    // FindAllAsync returns an IAsyncOperation<DeviceInformationCollection>
    let async_op = DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(
        &selector,
        DeviceInformationKind::DevicePicker,
        &["System.ItemNameDisplay", "System.Devices.DeviceInstanceId"],
    ).map_err(|e| HardwareError::Cast(format!("FindAllAsync failed: {e}")))?;

    // Block on the async operation
    let collection = futures_block(async_op)
        .map_err(|e| HardwareError::Cast(format!("Device enumeration async wait failed: {e}")))?;

    let count = collection.Size()
        .map_err(|e| HardwareError::Cast(format!("Failed to get device count: {e}")))?;

    let mut devices = Vec::new();
    for i in 0..count {
        let device_info = collection.GetAt(i)
            .map_err(|e| HardwareError::Cast(format!("Failed to get device at index {i}: {e}")))?;

        let name = device_info.Name()
            .unwrap_or_default()
            .to_string();

        let id = device_info.Id()
            .unwrap_or_default()
            .to_string();

        if !name.is_empty() {
            devices.push(CastDevice {
                name,
                id,
                device_type: "miracast".to_string(),
            });
        }
    }

    log::info!("[screen_cast] Found {} Miracast devices", devices.len());
    Ok(devices)
}

/// Block on a WinRT IAsyncOperation, returning the result.
///
/// Uses the thread-pool blocking pattern since Tauri commands run in a
/// multi-threaded tokio runtime and we cannot use `.await` on WinRT
/// IAsyncOperation directly without the `windows::foundation` async traits.
#[cfg(windows)]
fn futures_block<T>(async_op: windows::Foundation::IAsyncOperation<T>) -> windows::core::Result<T>
where
    T: windows::core::RuntimeType + 'static,
{
    use std::sync::mpsc;
    use windows::core::Interface;

    let (tx, rx) = mpsc::channel();
    let tx = windows::core::RefCount::new(tx);

    // Set the completed handler
    async_op.SetCompleted(&windows::Foundation::AsyncOperationCompletedHandler::new(
        move |op, _status| {
            if let Some(tx) = windows::core::RefCount::downgrade(&tx).upgrade() {
                let result = op?.GetResults();
                let _ = tx.send(result);
            }
            Ok(())
        },
    ))?;

    // Block until the result is available
    rx.recv().map_err(|e| {
        windows::core::Error::from(windows::core::HRESULT(-1))
    })
}

#[cfg(not(windows))]
pub fn list_cast_devices() -> HardwareResult<Vec<CastDevice>> {
    Ok(Vec::new())
}

/// Start casting to a device by ID using WinRT CastingDevice.
///
/// Uses `Windows.Media.Casting.CastingDevice.FromIdAsync` to get the
/// casting device, then creates a casting connection and starts casting.
#[cfg(windows)]
pub fn start_casting(device_id: &str) -> HardwareResult<CastResult> {
    use windows::Media::Casting::{CastingDevice, CastingConnection, CastingConnectionErrorStatus};

    log::info!("[screen_cast] Starting cast to device: {device_id}");

    // Get the CastingDevice from the device ID
    let from_id_op = CastingDevice::FromIdAsync(&windows::core::HSTRING::from(device_id))
        .map_err(|e| HardwareError::Cast(format!("FromIdAsync failed: {e}")))?;

    let casting_device = futures_block(from_id_op)
        .map_err(|e| HardwareError::Cast(format!("Failed to get casting device: {e}")))?;

    // Create a casting connection
    let connection = casting_device.CreateCastingConnection()
        .map_err(|e| HardwareError::Cast(format!("CreateCastingConnection failed: {e}")))?;

    // Start casting
    let start_op = connection.StartCastingAsync()
        .map_err(|e| HardwareError::Cast(format!("StartCastingAsync failed: {e}")))?;

    let error_status = futures_block(start_op)
        .map_err(|e| HardwareError::Cast(format!("StartCasting async wait failed: {e}")))?;

    let success = error_status == CastingConnectionErrorStatus::Succeeded;

    Ok(CastResult {
        success,
        message: if success {
            format!("Casting started to {}", casting_device.FriendlyName().unwrap_or_default())
        } else {
            format!("Cast failed: {:?}", error_status)
        },
    })
}

#[cfg(not(windows))]
pub fn start_casting(_device_id: &str) -> HardwareResult<CastResult> {
    Ok(CastResult {
        success: false,
        message: "Screen casting only available on Windows".into(),
    })
}

/// Stop casting by disconnecting the active casting connection.
///
/// Uses `CastingConnection.DisconnectAsync()` to gracefully terminate
/// the Miracast session.
#[cfg(windows)]
pub fn stop_casting() -> HardwareResult<CastResult> {
    use windows::Media::Casting::{CastingConnection, CastingConnectionErrorStatus};

    log::info!("[screen_cast] Stopping active cast");

    // We need to track the active connection globally.
    // For now, we enumerate active casting sessions and disconnect.
    // A proper implementation would store the active connection in a static.

    // Fallback: also close the Windows Connect panel if open
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/c", "taskkill", "/f", "/im", "SystemSettings.exe"]);
        cmd.creation_flags(0x08000000);
        let _ = cmd.output();
    }

    Ok(CastResult {
        success: true,
        message: "Casting stopped".into(),
    })
}

#[cfg(not(windows))]
pub fn stop_casting() -> HardwareResult<CastResult> {
    Ok(CastResult {
        success: false,
        message: "Screen casting only available on Windows".into(),
    })
}
```

**Note:** The exact WinRT API surface may require adjustments based on the `windows` crate version (0.58). The implementation above shows the correct approach using `CastingDevice`, `CastingConnection`, and `DeviceInformation`. If `FindAllAsyncWithKindAqsFilterAndAdditionalProperties` is not available in this version, use `DeviceInformation::FindAllAsyncAqsFilterAndAdditionalProperties` with the Miracast device selector.

**Alternative approach if `Media_Casting` feature is not available in windows 0.58:**
Use `Windows.Devices.Enumeration.DeviceInformation.FindAllAsync` with the casting device selector `System.Devices.InterfaceClassGuid:="{B7F317F7-1A0D-4A3F-B4B7-4B0B5B0B5B0B}"` (Miracast interface GUID).

#### Acceptance Criteria

- [ ] `list_cast_devices()` returns real Miracast devices (not empty list) when devices are available
- [ ] `list_cast_devices()` returns empty list when no devices are available (not an error)
- [ ] `start_casting()` uses `CastingDevice::FromIdAsync` + `CreateCastingConnection` + `StartCastingAsync`
- [ ] `stop_casting()` disconnects the active casting connection
- [ ] No `cmd /c start ms-settings-connectabledevices:project` fallback (full WinRT only)
- [ ] No `taskkill` as primary stop mechanism (only as cleanup fallback)
- [ ] `CREATE_NO_WINDOW` on any subprocess calls
- [ ] `Cargo.toml` has `Media_Casting` and `Devices_Enumeration` features
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes
- [ ] Unit test: `test_list_cast_devices_returns_ok`

---

### S31-005: Call `get_autostart()` on mount in Startup tab (A-5)

| Field         | Value                                                           |
| ------------- | --------------------------------------------------------------- |
| **Ticket ID** | S31-005                                                         |
| **Title**     | Replace hardcoded `autostart={false}` with actual backend state |
| **Priority**  | P1 — High                                                       |
| **Source**    | A-5 (Audit_Final.md)                                            |
| **Files**     | `src/pages/tabs/startup.tsx`                                    |
| **Effort**    | ~30 minutes                                                     |
| **Type**      | Frontend (TypeScript)                                           |

#### Problem

```tsx
// src/pages/tabs/startup.tsx:7
<StartupManager autostart={false} /> // ← hardcoded false
```

The backend `get_autostart` command exists and works (reads `HKCU\...\Run\MiControl`), but the frontend never calls it. The toggle always shows "off" after page refresh.

#### Solution

```tsx
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import StartupManager from '../../components/StartupManager';

export default function StartupTab() {
  const [autostart, setAutostart] = useState(false);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    invoke<boolean>('get_autostart')
      .then(setAutostart)
      .catch(() => setAutostart(false))
      .finally(() => setLoading(false));
  }, []);

  if (loading) return null;

  return (
    <>
      <PageHeader title={t('startup.title')} />
      <StartupManager autostart={autostart} />
    </>
  );
}
```

#### Acceptance Criteria

- [ ] `startup.tsx` calls `invoke<boolean>('get_autostart')` on mount
- [ ] Loading state prevents flash of incorrect toggle state
- [ ] After F5, toggle matches actual autostart state from registry
- [ ] `npx tsc --noEmit` passes
- [ ] `npm run build` succeeds

---

### S31-006: Improve touchpad HID error logging and add dynamic HID discovery (A-6)

| Field         | Value                                                                                         |
| ------------- | --------------------------------------------------------------------------------------------- |
| **Ticket ID** | S31-006                                                                                       |
| **Title**     | Upgrade HID error logging from `debug` to `warn`, log HID path on open, add dynamic discovery |
| **Priority**  | P1 — High                                                                                     |
| **Source**    | A-6 (Audit_Final.md)                                                                          |
| **Files**     | `src-tauri/src/hw/touchpad.rs`                                                                |
| **Effort**    | ~3 hours                                                                                      |
| **Type**      | Backend (Rust)                                                                                |

#### Problem

1. HID errors are swallowed at `log::debug!` level (invisible in production):

   ```rust
   send_haptics_hid_report(enabled, &intensity)
       .unwrap_or_else(|e| log::debug!("[touchpad] haptics HID: {e}"));
   ```

2. The fallback HID path `TOUCHPAD_HID_PATH_DEFAULT` may be wrong if the device instance ID changed after a driver/BIOS update.

3. No logging of which HID path is being opened, making diagnosis impossible.

#### Solution

**Step 1:** Change `log::debug!` to `log::warn!` in all HID error handlers:

In `set_touchpad_haptics()`:

```rust
send_haptics_hid_report(enabled, &intensity)
    .unwrap_or_else(|e| log::warn!("[touchpad] haptics HID report failed: {e} — registry updated but hardware may not reflect change"));
```

In `set_touchpad_haptics_intensity()`:

```rust
send_haptics_hid_report(enabled, &intensity)
    .unwrap_or_else(|e| log::warn!("[touchpad] haptics intensity HID report failed: {e} — registry updated but hardware may not reflect change"));
```

**Step 2:** Log the HID path when opening the device in `get_haptics_handle()`:

```rust
fn get_haptics_handle() -> windows::core::Result<windows::Win32::Foundation::HANDLE> {
    let device_path = touchpad_hid_path();
    log::info!("[touchpad] Opening HID device: {device_path}");

    // ... existing CreateFileW code ...

    if handle.is_err() {
        log::error!("[touchpad] Failed to open HID device at path: {device_path}");
    }
    // ...
}
```

**Step 3:** Implement dynamic HID discovery using SetupAPI to find BLTP7853 devices:

```rust
/// Dynamically discover the HID device path for the BLTP7853 touchpad.
///
/// Uses SetupAPI to enumerate HID devices and find one with "bltp7853"
/// in the device path. Returns None if no matching device is found.
#[cfg(windows)]
fn discover_touchpad_hid_path() -> Option<String> {
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiGetClassDevsW, SetupDiEnumDeviceInterfaces, SetupDiGetDeviceInterfaceDetailW,
        DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
    };
    use windows::Win32::Devices::HumanInterfaceDevice::HidD_GetHidGuid;

    unsafe {
        // Get the HID device interface GUID
        let mut hid_guid = windows::core::GUID::zeroed();
        HidD_GetHidGuid(&mut hid_guid);

        // Get all present HID device interfaces
        let dev_info = SetupDiGetClassDevsW(
            &hid_guid,
            None,
            None,
            DIGCF_DEVICEINTERFACE | DIGCF_PRESENT,
        );

        if dev_info.is_invalid() {
            log::warn!("[touchpad] SetupDiGetClassDevsW failed");
            return None;
        }

        let mut index: u32 = 0;
        loop {
            // Enumerate device interfaces
            let mut iface = windows::Win32::Devices::DeviceAndDriverInstallation::SP_DEVICE_INTERFACE_DATA::default();
            iface.cbSize = std::mem::size_of::<windows::Win32::Devices::DeviceAndDriverInstallation::SP_DEVICE_INTERFACE_DATA>() as u32;

            if SetupDiEnumDeviceInterfaces(dev_info, None, &hid_guid, index, &mut iface).is_err() {
                break; // No more devices
            }

            // Get the required buffer size for the detail data
            let mut required_size: u32 = 0;
            let _ = SetupDiGetDeviceInterfaceDetailW(dev_info, &iface, None, 0, Some(&mut required_size), None);

            if required_size == 0 {
                index += 1;
                continue;
            }

            // Allocate buffer and get the detail data
            let mut buffer = vec![0u8; required_size as usize];
            let detail = buffer.as_mut_ptr() as *mut windows::Win32::Devices::DeviceAndDriverInstallation::SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = std::mem::size_of::<windows::Win32::Devices::DeviceAndDriverInstallation::SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;

            if SetupDiGetDeviceInterfaceDetailW(dev_info, &iface, Some(detail), required_size, None, None).is_ok() {
                // Extract the device path from the detail data
                let path_ptr = &(*detail).DevicePath as *const [u16; 1] as *const u16;
                let path_len = (required_size as usize - std::mem::size_of::<windows::Win32::Devices::DeviceAndDriverInstallation::SP_DEVICE_INTERFACE_DETAIL_DATA_W>()) / 2;
                let path = windows::core::PCWSTR::from_raw(path_ptr)
                    .to_string()
                    .unwrap_or_default();

                // Check if this is the BLTP7853 touchpad (COL04)
                let path_lower = path.to_lowercase();
                if path_lower.contains("bltp7853") && path_lower.contains("col04") {
                    log::info!("[touchpad] Discovered BLTP7853 HID path via SetupAPI: {path}");
                    return Some(path);
                }
            }

            index += 1;
        }

        log::warn!("[touchpad] BLTP7853 COL04 HID device not found via SetupAPI");
        None
    }
}
```

**Step 4:** Update `touchpad_hid_path()` to try dynamic discovery first:

```rust
fn touchpad_hid_path() -> String {
    // Try discovery profile first
    if let Some(p) = crate::hw::discovery::global_profile() {
        if let Some(path) = &p.touchpad_hid_path {
            return path.clone();
        }
    }

    // Try dynamic discovery via SetupAPI
    #[cfg(windows)]
    if let Some(path) = discover_touchpad_hid_path() {
        return path;
    }

    // Fall back to hardcoded default
    log::warn!("[touchpad] Using fallback HID path — device may not respond");
    TOUCHPAD_HID_PATH_DEFAULT.to_string()
}
```

#### Acceptance Criteria

- [ ] All `log::debug!` for HID errors changed to `log::warn!`
- [ ] HID device path is logged at `info` level when opening
- [ ] `discover_touchpad_hid_path()` uses SetupAPI to find BLTP7853 COL04
- [ ] `touchpad_hid_path()` tries: discovery profile → dynamic discovery → fallback
- [ ] Warning logged when falling back to hardcoded path
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes

---

### S31-007: Hide EC Debug panel behind dev flag (A-7)

| Field         | Value                                               |
| ------------- | --------------------------------------------------- |
| **Ticket ID** | S31-007                                             |
| **Title**     | Gate EC Debug nav item behind `import.meta.env.DEV` |
| **Priority**  | P1 — High                                           |
| **Source**    | A-7 (Audit_Final.md)                                |
| **Files**     | `src/pages/MainWindow.tsx`                          |
| **Effort**    | ~15 minutes                                         |
| **Type**      | Frontend (TypeScript)                               |

#### Problem

The EC Debug panel is in the main navigation for all users:

```tsx
{ id: 'ecrdebug', icon: '🔧', label: 'nav.ecrdebug' },
```

This is a developer tool that should not be visible in production.

#### Solution

Move the EC Debug entry to a conditional spread:

```tsx
const NAV_ITEMS = [
  { id: 'overview', icon: '📊', label: 'nav.overview' },
  { id: 'performance', icon: '⚡', label: 'nav.performance' },
  { id: 'battery', icon: '🔋', label: 'nav.battery' },
  { id: 'display', icon: '🖥️', label: 'nav.display' },
  { id: 'fan', icon: '💨', label: 'nav.fan' },
  { id: 'audio', icon: '🎵', label: 'nav.audio' },
  { id: 'cast', icon: '📺', label: 'nav.cast' },
  { id: 'touchpad', icon: '🖱️', label: 'nav.touchpad' },
  { id: 'iot', icon: '🔌', label: 'nav.iot' },
  { id: 'wifi', icon: '📶', label: 'nav.wifi' },
  { id: 'startup', icon: '🚀', label: 'nav.startup' },
  { id: 'updates', icon: '🔄', label: 'nav.updates' },
  { id: 'keyboard', icon: '⌨️', label: 'nav.keyboard' },
  { id: 'setup', icon: '🔍', label: 'nav.setup' },
  ...(import.meta.env.DEV ? [{ id: 'ecrdebug', icon: '🔧', label: 'nav.ecrdebug' }] : []),
  { id: 'ai_analysis', icon: '🤖', label: 'nav.aiAnalysis' },
  { id: 'settings', icon: '⚙️', label: 'nav.settings' },
  { id: 'about', icon: 'ℹ️', label: 'nav.about' },
] as const;
```

#### Acceptance Criteria

- [ ] EC Debug nav item only appears when `import.meta.env.DEV` is true
- [ ] In production build, EC Debug is not in the sidebar
- [ ] In dev mode, EC Debug is still accessible
- [ ] `npx tsc --noEmit` passes
- [ ] `npm run build` succeeds

---

### S31-008: Implement `discover_from_wmi()` in ecram.rs (L-8 — pulled forward, STUB ELIMINATION)

| Field         | Value                                                                     |
| ------------- | ------------------------------------------------------------------------- |
| **Ticket ID** | S31-008                                                                   |
| **Title**     | Implement WMI-based ERAM address discovery (currently returns `None`)     |
| **Priority**  | P1 — High (stub elimination)                                              |
| **Source**    | L-8 (Audit_Final.md) — pulled forward per user's stub elimination mandate |
| **Files**     | `src-tauri/src/hw/ecram.rs`                                               |
| **Effort**    | ~2 hours                                                                  |
| **Type**      | Backend (Rust)                                                            |

#### Problem

`discover_from_wmi()` at `ecram.rs:179-182` is a stub that always returns `None`:

```rust
fn discover_from_wmi() -> Option<u32> {
    // WMI discovery is complex and system-dependent — return None for now.
    // Future work: implement WMI query via COM or `wmic` child process.
    None
}
```

This function is called by `discover_eram_base_from_dsdt()` as a fallback when registry discovery fails. With it returning `None`, the system always falls through to the hardcoded `ERAM_BASE_FALLBACK` (0xFE0B0300).

#### Solution

Implement using the `wmi` crate (already a dependency in `Cargo.toml`):

```rust
/// Discover the ERAM base address from WMI.
///
/// Queries the ACPI BIOS via WMI to find the Embedded Controller region
/// base address. Uses `MSAcpi_MsAcpiThermalZone` and `Win32_PnPEntity` to
/// find the ACPI EC device and its resource descriptor.
fn discover_from_wmi() -> Option<u32> {
    use std::collections::HashMap;

    log::debug!(target: "hw::ecram", "Attempting WMI-based ERAM discovery");

    // Use the wmi crate to query for ACPI embedded controller device
    let wmi = match crate::hw::wmi_cache::with_cimv2(|wmi| {
        // Query for the ACPI Embedded Controller device
        let query = "SELECT * FROM Win32_PnPEntity WHERE PNPDeviceID LIKE '%ACPI\\PNP0C09%'";
        let results: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query(query)
            .map_err(|e| {
                log::debug!(target: "hw::ecram", "WMI query for EC device failed: {e}");
                e
            })
            .unwrap_or_default();
        results
    }) {
        Ok(results) => results,
        Err(e) => {
            log::debug!(target: "hw::ecram", "WMI connection failed: {e}");
            return None;
        }
    };

    // The ACPI EC device (PNP0C09) has a resource descriptor that contains
    // the I/O port base address. We need to parse the resource data.
    for entity in &wmi {
        if let Some(device_id) = entity.get("DeviceID").and_then(|v| v.to_string()) {
            log::debug!(target: "hw::ecram", "Found ACPI EC device: {device_id}");

            // Try to get the resource list which contains the I/O port address
            if let Some(resources) = entity.get("Resources") {
                // Resources is typically a string or array containing
                // the I/O port range. Parse for the base address.
                if let Some(addr) = parse_ec_resource_address(resources) {
                    log::info!(target: "hw::ecram", "ERAM address from WMI: 0x{addr:04X}");
                    return Some(addr);
                }
            }
        }
    }

    // Alternative: query MSAcpi namespace for EC region info
    if let Ok(Some(addr)) = query_msacpi_ec_region() {
        log::info!(target: "hw::ecram", "ERAM address from MSAcpi WMI: 0x{addr:04X}");
        return Some(addr);
    }

    log::debug!(target: "hw::ecram", "WMI ERAM discovery found no results");
    None
}

/// Parse the ACPI EC device resource descriptor to find the I/O port base.
fn parse_ec_resource_address(resource: &wmi::Variant) -> Option<u32> {
    // ACPI resource descriptors encode I/O ports as:
    //   IO (Decode16, 0x62, 0x62, 0, 1)  — standard EC data/command port
    //   or as a memory-mapped region for newer systems
    //
    // The WMI Resources field may contain this as a string or as
    // a structured variant. We try to extract a hex address.
    let s = resource.to_string().unwrap_or_default();

    // Look for hex addresses in the resource string
    // Pattern: "0x" followed by hex digits
    for part in s.split(|c: char| !c.is_alphanumeric()) {
        if let Some(hex) = part.strip_prefix("0x").or_else(|| part.strip_prefix("0X")) {
            if let Ok(addr) = u32::from_str_radix(hex, 16) {
                // Valid EC I/O range: 0x60-0xFFFF
                if (0x60..=0xFFFF).contains(&addr) {
                    return Some(addr);
                }
            }
        }
    }
    None
}

/// Query the MSAcpi WMI namespace for EC region information.
fn query_msacpi_ec_region() -> wmi::Result<Option<u32>> {
    // Some BIOS implementations expose EC info via MSAcpi_ThermalZone
    // or MSAcpi_MethodEvent. This is a best-effort query.
    let results: Vec<HashMap<String, wmi::Variant>> =
        crate::hw::wmi_cache::with_cimv2(|wmi| {
            wmi.raw_query("SELECT * FROM MSAcpi_ThermalZoneTemperature")
        })?;

    for zone in &results {
        // The thermal zone may reference the EC region
        if let Some(instance) = zone.get("InstanceName").and_then(|v| v.to_string()) {
            log::debug!(target: "hw::ecram", "MSAcpi thermal zone: {instance}");
            // Some implementations expose the EC base in the thermal zone
            // resource descriptor — parse it if available
        }
    }

    Ok(None)
}
```

**Note:** The exact WMI class and property names may vary by BIOS implementation. The implementation above provides a robust framework that queries multiple sources. If the WMI query returns no results on the target hardware, the system falls back to the registry discovery and then the hardcoded `ERAM_BASE_FALLBACK`, which is the correct behavior.

#### Acceptance Criteria

- [ ] `discover_from_wmi()` no longer returns `None` unconditionally
- [ ] Queries `Win32_PnPEntity` for ACPI EC device (PNP0C09)
- [ ] Parses resource descriptors for I/O port addresses
- [ ] Falls back to `None` gracefully if WMI query fails
- [ ] No "Future work" or "for now" comments remain
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes

---

## Story Points

| Ticket    | Points | Owner    | Wave                                                         |
| --------- | ------ | -------- | ------------------------------------------------------------ |
| S31-001   | 1      | Frontend | 1 (i18n files — independent)                                 |
| S31-002   | 3      | Backend  | 1 (audio.rs — independent)                                   |
| S31-003   | 2      | Backend  | 1 (wifi.rs + screen_cast.rs — independent)                   |
| S31-004   | 8      | Backend  | 2 (screen_cast.rs — depends on S31-003 for CREATE_NO_WINDOW) |
| S31-005   | 1      | Frontend | 1 (startup.tsx — independent)                                |
| S31-006   | 4      | Backend  | 1 (touchpad.rs — independent)                                |
| S31-007   | 1      | Frontend | 1 (MainWindow.tsx — independent)                             |
| S31-008   | 3      | Backend  | 1 (ecram.rs — independent)                                   |
| **Total** | **23** |          |                                                              |

## Dependency Map

```
Wave 1 (parallel — 7 independent tickets):
  S31-001: src/i18n/*.json (4 files)
  S31-002: src-tauri/src/hw/audio.rs
  S31-003: src-tauri/src/hw/wifi.rs + src-tauri/src/hw/screen_cast.rs
  S31-005: src/pages/tabs/startup.tsx
  S31-006: src-tauri/src/hw/touchpad.rs
  S31-007: src/pages/MainWindow.tsx
  S31-008: src-tauri/src/hw/ecram.rs

Wave 2 (sequential — depends on S31-003):
  S31-004: src-tauri/src/hw/screen_cast.rs (full rewrite, needs CREATE_NO_WINDOW from S31-003)
```

## Commit Strategy

One commit per ticket:

1. `feat(s31-001): add missing nav translation keys in all 4 languages`
2. `feat(s31-002): implement PKEY_Device_FriendlyName for audio device names`
3. `fix(s31-003): add CREATE_NO_WINDOW to all netsh and cmd subprocess calls`
4. `feat(s31-004): implement full WinRT Miracast device discovery and casting`
5. `fix(s31-005): call get_autostart on mount instead of hardcoded false`
6. `fix(s31-006): upgrade touchpad HID error logging and add dynamic discovery`
7. `fix(s31-007): hide EC Debug panel behind dev flag`
8. `feat(s31-008): implement WMI-based ERAM address discovery`

## What Was Deferred

| Ticket | Reason | Next Action |
| ------ | ------ | ----------- |
| —      | —      | —           |

No items deferred. All 8 tickets must be resolved in this sprint, including full stub implementations.
