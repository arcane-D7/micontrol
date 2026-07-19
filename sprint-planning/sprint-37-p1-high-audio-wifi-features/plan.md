# Sprint 37 — P1 HIGH: Audio Device Switching & WiFi WlanAPI

> **Date:** 2026-07-19
> **Sprint:** 37
> **Theme:** Implement audio device switching via IPolicyConfig COM + replace netsh WiFi with native WlanAPI
> **Duration:** ~5–10 days
> **Dependencies:** Sprint 34 (Auth Bridge fixes), Sprint 35 (temperature/volume fixes)
> **Status:** 📌 Active
> **Audit Reference:** `C:\Users\mafsc\Documents\Audit_Report_miControl.md` (Bug 5: 5A–5E; Bug 6: 6A)

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **OBRIGATÓRIO: 100% dos tickets desta sprint devem ser concluídos. A sprint não será aceita como entregue se qualquer ticket permanecer incompleto.**
>
> **MANDATORY: 100% of the tickets in this sprint MUST be completed. The sprint will NOT be accepted as delivered if any ticket remains incomplete.**

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

This sprint implements two major features that were identified as missing or broken in the audit: audio device switching (never implemented) and WiFi scanning via native WlanAPI (replacing locale-dependent `netsh` parsing). These are the most complex tickets in the audit remediation, requiring COM FFI and Windows API integration.

1. **S37-001:** Implement `set_default_endpoint()` in `audio.rs` using `IPolicyConfig` COM interface
2. **S37-002:** Register `set_audio_default_endpoint` Tauri command and wire up frontend
3. **S37-003:** Replace `netsh wlan scan` with native WlanAPI (`wlanapi.dll`) for locale-independent WiFi scanning
4. **S37-004:** Replace `netsh wlan show interfaces` with WlanAPI for locale-independent connection status
5. **S37-005:** Add i18n strings for audio device switching UI

---

## Goals

| #   | Goal                                                         | KPI                                      | Audit Reference |
| --- | ------------------------------------------------------------ | ---------------------------------------- | --------------- |
| 1   | User can switch default audio playback device from miControl | `set_default_endpoint()` works via COM   | Bug 5 (5A–5E)   |
| 2   | WiFi scanning works on all Windows locales                   | WlanAPI replaces `netsh` parsing         | Bug 6 (6A)      |
| 3   | WiFi connection status works on all Windows locales          | WlanAPI replaces `netsh show interfaces` | Bug 6 (6A)      |
| 4   | Audio device UI is accessible and localized                  | Clickable device list with i18n labels   | Bug 5 (5D)      |

---

## Technical Specs

### S37-001: Implement `set_default_endpoint()` using IPolicyConfig COM (Bug 5A)

| Field         | Value                                                                    |
| ------------- | ------------------------------------------------------------------------ |
| **Ticket ID** | S37-001                                                                  |
| **Title**     | Implement `IPolicyConfig::SetDefaultEndpoint` via COM FFI in `audio.rs`  |
| **Priority**  | P1 — High                                                                |
| **Source**    | Bug 5A (`Audit_Report_miControl.md`)                                     |
| **Files**     | `src-tauri/src/hw/audio.rs` (add after `set_playback_mute()`, ~line 215) |
| **Effort**    | ~4–6 hours                                                               |
| **Type**      | Backend (Rust, COM FFI)                                                  |

#### Problem

miControl can list audio devices and control volume, but **cannot switch the default playback device**. The `IPolicyConfig::SetDefaultEndpoint` COM interface is required but not implemented.

**Confirmed:** `audio.rs` has only 4 public functions: `list_audio_devices()`, `get_playback_volume()`, `set_playback_volume()`, `set_playback_mute()`. No `set_default_endpoint` exists anywhere in the codebase.

#### Solution

Implement `IPolicyConfig::SetDefaultEndpoint` via COM FFI:

**Constants:**

- CLSID: `{870af99c-171d-4f9e-af0d-e63df40c2bc9}` (`PolicyConfigClient`)
- IID: `{f8679f50-850a-41cf-9c72-430f290290c8}` (`IPolicyConfig`, Windows 8.1+)
- Method: `SetDefaultEndpoint(pwcsDeviceId, eRole)` where `eConsole = 0`

**Vtable layout (Windows 8.1+ IID `f8679f50-...`):**

- Slots 0–2: IUnknown (QueryInterface, AddRef, Release)
- Slots 3–14: Other IPolicyConfig methods (unused, must be present in vtable)
- Slot 15: `SetDefaultEndpoint`

**⚠️ CRITICAL:** The exact vtable slot must be validated empirically on the target Windows version before shipping. An alternative is the `IPolicyConfigX` (Windows 10 1703+) IID `6db61774-b7be-4a0f-a3b7-4b18e9b4a3df` which has a different slot layout.

**Implementation:**

```rust
/// COM interface for setting the default audio endpoint.
/// IPolicyConfig is undocumented but stable since Windows 8.1.
/// IID: {f8679f50-850a-41cf-9c72-430f290290c8}
/// CLSID: {870af99c-171d-4f9e-af0d-e63df40c2bc9}
#[cfg(windows)]
#[repr(C)]
struct IPolicyConfigVtbl {
    query_interface: unsafe extern "system" fn(*mut std::ffi::c_void, *const windows::core::GUID, *mut *mut std::ffi::c_void) -> windows::core::HRESULT,
    add_ref: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
    release: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
    // Slots 3-14: other IPolicyConfig methods (unused, but must be present)
    _slots: [*mut std::ffi::c_void; 12],
    // Slot 15: SetDefaultEndpoint
    set_default_endpoint: unsafe extern "system" fn(*mut std::ffi::c_void, windows::core::PCWSTR, u32) -> windows::core::HRESULT,
}

#[cfg(windows)]
const CLSID_POLICY_CONFIG: windows::core::GUID = windows::core::GUID::from_u128(
    0x870af99c_171d_4f9e_af0d_e63df40c2bc9,
);
#[cfg(windows)]
const IID_IPOLICY_CONFIG: windows::core::GUID = windows::core::GUID::from_u128(
    0xf8679f50_850a_41cf_9c72_430f290290c8,
);

/// ERole for SetDefaultEndpoint
const E_CONSOLE: u32 = 0;

/// Set the default audio playback endpoint by device ID.
#[cfg(windows)]
pub fn set_default_endpoint(device_id: &str) -> HardwareResult<()> {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };
    use windows::core::Interface;

    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let result = (|| -> anyhow::Result<()> {
        unsafe {
            let unknown: windows::core::IUnknown =
                CoCreateInstance(&CLSID_POLICY_CONFIG, None, CLSCTX_ALL)?;
            let policy: windows::core::RawPtr = std::ptr::null_mut();
            let hr = (unknown.vtable().query_interface)(
                unknown.as_raw(),
                &IID_IPOLICY_CONFIG,
                &mut policy as *mut _ as *mut *mut _,
            );
            if hr.is_err() || policy.is_null() {
                anyhow::bail!("QueryInterface for IPolicyConfig failed: {hr:?}");
            }

            let vtbl = &*(policy as *const *const IPolicyConfigVtbl).read();

            let device_w: Vec<u16> = std::ffi::OsStr::new(device_id)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let hr = (vtbl.set_default_endpoint)(
                policy as *mut _,
                windows::core::PCWSTR(device_w.as_ptr()),
                E_CONSOLE,
            );
            // Release the interface
            (vtbl.release)(policy as *mut _);

            if hr.is_err() {
                anyhow::bail!("SetDefaultEndpoint failed: {hr:?}");
            }
            Ok(())
        }
    })();

    unsafe { CoUninitialize(); }
    result.map_err(Into::into)
}

#[cfg(not(windows))]
pub fn set_default_endpoint(_device_id: &str) -> HardwareResult<()> {
    Err(crate::hw::errors::HardwareError::NotSupported(
        "Audio device switching is Windows-only".into(),
    ))
}
```

#### Acceptance Criteria

- [ ] `set_default_endpoint()` function implemented in `audio.rs`
- [ ] Function takes `device_id: &str` and returns `HardwareResult<()>`
- [ ] COM initialized and uninitialized correctly
- [ ] `SetDefaultEndpoint` called with `eConsole` role
- [ ] **Vtable slot validated empirically on Windows 11 target** (log HRESULT, compare with `S_OK`)
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] Manual test: calling `set_default_endpoint("device_id")` switches the default device (verify in Windows Sound Settings)

---

### S37-002: Register Tauri command and wire up frontend for audio device switching (Bug 5B–5E)

| Field         | Value                                                                                                                       |
| ------------- | --------------------------------------------------------------------------------------------------------------------------- |
| **Ticket ID** | S37-002                                                                                                                     |
| **Title**     | Add `set_audio_default_endpoint` command + frontend click handler                                                           |
| **Priority**  | P1 — High                                                                                                                   |
| **Source**    | Bug 5B–5E (`Audit_Report_miControl.md`)                                                                                     |
| **Files**     | `src-tauri/src/commands/hardware.rs`, `src-tauri/src/lib.rs`, `src/components/AudioControl.tsx`, `src/hooks/useHardware.ts` |
| **Effort**    | ~2–3 hours                                                                                                                  |
| **Type**      | Full-stack (Rust + TypeScript)                                                                                              |

#### Problem

Even after implementing `set_default_endpoint()` in `audio.rs`, the functionality is not exposed to the user. The Tauri command is not registered, and the frontend device list is display-only (`<div>` without `onClick`).

#### Current Code

**`AudioControl.tsx` lines 130–142 — display-only device list:**

```tsx
{devices.playback.slice(0, 5).map((d) => (
    <div key={d.id} className="stat-row" style={{...}}>
      <span style={{ flex: 1, fontSize: 13 }}>{d.name}</span>
      <span style={{ fontSize: 11, color: 'var(--text-dim)' }}>
        {d.is_default ? `✓ ${t('audio.defaultDevice')}` : ''}
      </span>
    </div>
))}
```

No `onClick` handler. Devices are `<div>` elements, not buttons.

#### Solution

**Step 1:** Add Tauri command in `commands/hardware.rs`:

```rust
/// Set the default audio playback device by device ID.
#[tauri::command]
pub async fn set_audio_default_endpoint(device_id: String) -> Result<(), ErrorResponse> {
    run_blocking(move || crate::hw::audio::set_default_endpoint(&device_id))
        .await
        .map_err(ErrorResponse::from)
}
```

**Step 2:** Register command in `lib.rs` `invoke_handler!`:

```rust
.invoke_handler(tauri::generate_handler![
    // ... existing commands ...
    set_audio_default_endpoint,
    // ...
])
```

**Step 3:** Add `setDefaultAudioDevice` to `useHardware.ts`:

```typescript
const setDefaultAudioDevice = useCallback(async (deviceId: string) => {
  try {
    await invoke('set_audio_default_endpoint', { deviceId });
    // Refresh device list to update is_default flags
    const list = await invoke<AudioDeviceList>('get_audio_devices');
    // Update local state...
    setError(null);
  } catch (e) {
    console.error('[audio] set_default_endpoint failed:', e);
    setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
    throw e;
  }
}, []);
```

**Step 4:** Update `AudioControl.tsx` — change `<div>` to `<button>`:

```tsx
interface AudioControlProps {
  audioState: AudioVolumeResult | null;
  loading: boolean;
  onVolumeChange: (volumeFraction: number) => Promise<void>;
  onMuteToggle: (muted: boolean) => Promise<void>;
  onSetDefaultDevice?: (deviceId: string) => Promise<void>; // NEW
}

// In the device map:
{
  devices.playback.slice(0, 5).map((d) => (
    <button
      key={d.id}
      className="stat-row"
      disabled={d.is_default}
      onClick={() => onSetDefaultDevice?.(d.id)}
      style={{
        padding: '6px 8px',
        borderRadius: 'var(--r-xs)',
        background: d.is_default ? 'var(--bg-hover)' : 'transparent',
        marginBottom: 4,
        cursor: d.is_default ? 'default' : 'pointer',
        width: '100%',
        textAlign: 'left',
        border: 'none',
      }}
    >
      <span style={{ flex: 1, fontSize: 13 }}>{d.name}</span>
      <span style={{ fontSize: 11, color: 'var(--text-dim)' }}>
        {d.is_default ? `✓ ${t('audio.defaultDevice')}` : t('audio.setAsDefault')}
      </span>
    </button>
  ));
}
```

#### Acceptance Criteria

- [ ] `set_audio_default_endpoint` command registered in `lib.rs`
- [ ] `setDefaultAudioDevice` function added to `useHardware.ts`
- [ ] `AudioControl.tsx` device list uses `<button>` with `onClick`
- [ ] Non-default devices show "Set as Default" label
- [ ] Default device button is disabled
- [ ] Clicking a device switches the default (verify in Windows Sound Settings)
- [ ] Device list refreshes after switching
- [ ] `cargo check` passes
- [ ] `npx tsc --noEmit` passes
- [ ] `npm run build` succeeds

---

### S37-003: Replace `netsh wlan scan` with native WlanAPI (Bug 6A)

| Field         | Value                                                                                       |
| ------------- | ------------------------------------------------------------------------------------------- |
| **Ticket ID** | S37-003                                                                                     |
| **Title**     | Replace `netsh wlan scan` + `netsh wlan show networks` with WlanAPI FFI                     |
| **Priority**  | P1 — High                                                                                   |
| **Source**    | Bug 6A (`Audit_Report_miControl.md`)                                                        |
| **Files**     | `src-tauri/src/hw/wifi.rs` (lines 35–60 `scan_networks`, lines 150–260 `parse_scan_output`) |
| **Effort**    | ~6–8 hours                                                                                  |
| **Type**      | Backend (Rust, FFI)                                                                         |

#### Problem

`scan_networks()` uses `netsh wlan scan` (triggers async scan) + `netsh wlan show networks mode=bssid` (lists results). The output is parsed using English-only keywords (`"Signal"`, `"Authentication"`). On Portuguese Windows, these keywords are `"Sinal"`, `"Autenticação"` — all parsing fails, returning an empty network list.

The `windows` crate already includes `WlanOpenHandle`, `WlanEnumInterfaces`, `WlanScan`, `WlanGetNetworkBssList` under `windows::Win32::NetworkManagement::WiFi`. These return structured data (not text), making them locale-independent.

#### Current Code

**`wifi.rs` lines 35–60:**

```rust
pub fn scan_networks() -> HardwareResult<Vec<WifiNetwork>> {
    #[cfg(windows)]
    {
        let mut scan_cmd = Command::new("netsh");
        scan_cmd.args(["wlan", "scan"]);
        scan_cmd.creation_flags(CREATE_NO_WINDOW);
        let _ = scan_cmd.output();
        std::thread::sleep(std::time::Duration::from_millis(1500));  // S35-007 changes to 4000
    }
    // ... parse netsh output ...
}
```

**`wifi.rs` lines 150–260:** `parse_scan_output()` matches English keywords only.

#### Solution

Replace the entire `scan_networks()` function with WlanAPI calls:

```rust
/// Scan for available WiFi networks using the native WlanAPI.
///
/// Uses wlanapi.dll instead of shelling out to `netsh wlan`. This is:
/// - Locale-independent (structured data, not parsed text)
/// - Faster (no arbitrary sleep — scan completes via notification)
/// - More accurate (raw RSSI, not parsed percentage)
#[cfg(windows)]
pub fn scan_networks() -> HardwareResult<Vec<WifiNetwork>> {
    use windows::Win32::NetworkManagement::WiFi::{
        WlanCloseHandle, WlanEnumInterfaces, WlanGetAvailableNetworkList,
        WlanOpenHandle, WlanScan, WLAN_API_VERSION_2_0,
        WLAN_AVAILABLE_NETWORK_LIST, WLAN_INTERFACE_INFO_LIST,
    };
    use windows::Win32::Foundation::HANDLE;

    unsafe {
        // 1. Open WLAN handle
        let mut handle = HANDLE::default();
        let mut negotiated_version = 0u32;
        WlanOpenHandle(WLAN_API_VERSION_2_0, None, &mut negotiated_version, &mut handle)
            .map_err(|e| HardwareError::Wifi(format!("WlanOpenHandle: {e}")))?;

        // 2. Enumerate interfaces
        let iface_list_ptr = WlanEnumInterfaces(handle, None)
            .map_err(|e| HardwareError::Wifi(format!("WlanEnumInterfaces: {e}")))?;

        // 3. Pick first interface
        let iface_list = &*iface_list_ptr;
        if iface_list.dwNumberOfItems == 0 {
            WlanCloseHandle(handle, None);
            return Ok(vec![]);
        }
        let guid = iface_list.InterfaceInfo[0].InterfaceGuid;

        // 4. Trigger scan (async, returns immediately)
        WlanScan(handle, &guid, None, None)
            .map_err(|e| HardwareError::Wifi(format!("WlanScan: {e}")))?;

        // 5. Wait for scan completion (4s pragmatic — proper fix uses WlanRegisterNotification)
        std::thread::sleep(std::time::Duration::from_millis(4000));

        // 6. Get available network list (structured, locale-independent!)
        let network_list_ptr = WlanGetAvailableNetworkList(handle, &guid, 0, None, None)
            .map_err(|e| HardwareError::Wifi(format!("WlanGetAvailableNetworkList: {e}")))?;

        // 7. Parse structured data into WifiNetwork
        let network_list = &*network_list_ptr;
        let mut networks = Vec::new();
        for i in 0..network_list.dwNumberOfItems {
            let net = &network_list.Network[i];
            // dot11Ssid: 4-byte length + 32-byte SSID buffer
            let ssid_len = net.dot11Ssid.SSIDLength as usize;
            let ssid_bytes = &net.dot11Ssid.SSID[..ssid_len];
            let ssid = String::from_utf8_lossy(ssid_bytes).to_string();

            // wlanSignalQuality: 0-100 percentage
            let signal = net.wlanSignalQuality;

            // bSecurityEnabled: bool
            let security = if net.bSecurityEnabled.as_bool() {
                "WPA2-Personal"  // Simplified — actual auth is in dot11DefaultAuthAlgorithm
            } else {
                "Open"
            };

            networks.push(WifiNetwork {
                ssid,
                signal,
                security: security.to_string(),
                connected: false,  // Determined by get_status()
            });
        }

        WlanCloseHandle(handle, None);
        Ok(networks)
    }
}
```

**Note:** The `WlanGetAvailableNetworkList` returns `WLAN_AVAILABLE_NETWORK` entries with:

- `dot11Ssid` — binary SSID (no text parsing needed)
- `wlanSignalQuality` — 0-100% signal (no `%` suffix parsing)
- `bSecurityEnabled` — bool (no "Authentication:" keyword matching)
- `dot11BssType` — enum (not locale-dependent string)

Remove the `parse_scan_output()` function entirely — it's no longer needed.

#### Acceptance Criteria

- [ ] `scan_networks()` uses WlanAPI instead of `netsh`
- [ ] `parse_scan_output()` function removed (dead code)
- [ ] WiFi networks appear on Portuguese Windows (currently they don't)
- [ ] WiFi networks appear on English Windows (regression check)
- [ ] Signal strength matches `netsh` output (within ±5%)
- [ ] Scan completes in <6s
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes

---

### S37-004: Replace `netsh wlan show interfaces` with WlanAPI for connection status (Bug 6A)

| Field         | Value                                                                                      |
| ------------- | ------------------------------------------------------------------------------------------ |
| **Ticket ID** | S37-004                                                                                    |
| **Title**     | Replace `netsh wlan show interfaces` with `WlanQueryInterface` for status                  |
| **Priority**  | P1 — High                                                                                  |
| **Source**    | Bug 6A (`Audit_Report_miControl.md`) — `parse_interface_output()`                          |
| **Files**     | `src-tauri/src/hw/wifi.rs` (lines 200–260 `parse_interface_output`, `get_status` function) |
| **Effort**    | ~3–4 hours                                                                                 |
| **Type**      | Backend (Rust, FFI)                                                                        |

#### Problem

`get_status()` uses `netsh wlan show interfaces` and parses the output with English-only keywords (`"Name"`, `"SSID"`, `"State"`, `"Signal"`). On Portuguese Windows:

- `"State"` → `"Estado"`
- `"connected"` → `"conectado"`

The `connected` check `state.as_deref() == Some("connected")` fails, returning `connected: false` even when connected.

#### Solution

Replace `parse_interface_output()` with `WlanQueryInterface` using:

- `wlan_intf_opcode_interface_state` → returns enum `wlan_interface_state_connected` (not locale-dependent string)
- `wlan_intf_opcode_current_connection` → returns `WLAN_CONNECTION_ATTRIBUTES` with SSID and signal

```rust
#[cfg(windows)]
pub fn get_status() -> HardwareResult<WifiStatus> {
    use windows::Win32::NetworkManagement::WiFi::{
        WlanCloseHandle, WlanEnumInterfaces, WlanOpenHandle, WlanQueryInterface,
        WLAN_API_VERSION_2_0, WLAN_INTF_OPCODE_CURRENT_CONNECTION,
        WLAN_INTF_OPCODE_INTERFACE_STATE,
    };
    use windows::Win32::Foundation::HANDLE;

    unsafe {
        let mut handle = HANDLE::default();
        let mut negotiated = 0u32;
        WlanOpenHandle(WLAN_API_VERSION_2_0, None, &mut negotiated, &mut handle)
            .map_err(|e| HardwareError::Wifi(format!("WlanOpenHandle: {e}")))?;

        let iface_list_ptr = WlanEnumInterfaces(handle, None)
            .map_err(|e| HardwareError::Wifi(format!("WlanEnumInterfaces: {e}")))?;
        let iface_list = &*iface_list_ptr;
        if iface_list.dwNumberOfItems == 0 {
            WlanCloseHandle(handle, None);
            return Ok(WifiStatus {
                connected: false,
                ssid: None,
                signal: None,
                interface: None,
            });
        }
        let guid = iface_list.InterfaceInfo[0].InterfaceGuid;
        let interface_name = String::from_utf16_lossy(
            &iface_list.InterfaceInfo[0].strInterfaceDescription
        ).trim_end_matches('\0').to_string();

        // Query interface state (enum, not locale-dependent string)
        let mut state_ptr = std::ptr::null_mut::<u32>();
        let mut data_size = 0u32;
        WlanQueryInterface(handle, &guid, WLAN_INTF_OPCODE_INTERFACE_STATE, None, &mut data_size, &mut state_ptr as *mut _ as *mut *mut std::ffi::c_void, None)
            .map_err(|e| HardwareError::Wifi(format!("WlanQueryInterface(state): {e}")))?;

        let state = *state_ptr;
        // WLAN_INTERFACE_STATE: 0=not ready, 1=connected, 2=ad_hoc_network_formed,
        // 3=disconnecting, 4=disconnected, 5=associating, 6=discovering, 7=authenticating
        let connected = state == 1;  // wlan_interface_state_connected

        let mut ssid = None;
        let mut signal = None;

        if connected {
            // Query current connection for SSID and signal
            let mut conn_ptr = std::ptr::null_mut::<u8>();
            let mut conn_size = 0u32;
            if WlanQueryInterface(handle, &guid, WLAN_INTF_OPCODE_CURRENT_CONNECTION, None, &mut conn_size, &mut conn_ptr as *mut _ as *mut *mut std::ffi::c_void, None).is_ok() {
                // Parse WLAN_CONNECTION_ATTRIBUTES for SSID and signal
                // ... (extract SSID from wlanAssociationAttributes.dot11Ssid,
                //      signal from wlanAssociationAttributes.wlanSignalQuality)
            }
        }

        WlanCloseHandle(handle, None);

        Ok(WifiStatus {
            connected,
            ssid,
            signal,
            interface: Some(interface_name),
        })
    }
}
```

Remove `parse_interface_output()` function entirely.

#### Acceptance Criteria

- [ ] `get_status()` uses WlanAPI instead of `netsh`
- [ ] `parse_interface_output()` function removed
- [ ] `connected` returns `true` on Portuguese Windows when connected (currently returns `false`)
- [ ] `connected` returns `true` on English Windows (regression check)
- [ ] SSID and signal are populated when connected
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes

---

### S37-005: Add i18n strings for audio device switching UI

| Field         | Value                                                                     |
| ------------- | ------------------------------------------------------------------------- |
| **Ticket ID** | S37-005                                                                   |
| **Title**     | Add translation keys for "Set as Default" and related audio device labels |
| **Priority**  | P1 — High                                                                 |
| **Source**    | Bug 5D (`Audit_Report_miControl.md`) — frontend i18n                      |
| **Files**     | `src/i18n/locales/pt.json`, `src/i18n/locales/en.json` (or equivalent)    |
| **Effort**    | ~15 minutes                                                               |
| **Type**      | Frontend (JSON i18n)                                                      |

#### Problem

The new "Set as Default" button label needs translation keys. The existing `audio.defaultDevice` key exists, but `audio.setAsDefault` does not.

#### Solution

Add the following keys to all locale files:

**`pt.json`:**

```json
{
  "audio": {
    "defaultDevice": "Dispositivo padrão",
    "setAsDefault": "Definir como padrão"
  }
}
```

**`en.json`:**

```json
{
  "audio": {
    "defaultDevice": "Default device",
    "setAsDefault": "Set as default"
  }
}
```

#### Acceptance Criteria

- [ ] `audio.setAsDefault` key added to all locale files
- [ ] Label displays correctly in Portuguese and English
- [ ] `npx tsc --noEmit` passes
- [ ] `npm run build` succeeds

---

## Story Points

| Ticket    | Points | Owner      | Wave                                           |
| --------- | ------ | ---------- | ---------------------------------------------- |
| S37-001   | 5      | Backend    | 1 (audio.rs — COM FFI, independent)            |
| S37-002   | 3      | Full-stack | 2 (commands + frontend — depends on 001)       |
| S37-003   | 5      | Backend    | 1 (wifi.rs — WlanAPI, independent)             |
| S37-004   | 3      | Backend    | 2 (wifi.rs — depends on 003 for WlanAPI setup) |
| S37-005   | 1      | Frontend   | 2 (i18n — depends on 002 for key names)        |
| **Total** | **17** |            |                                                |

## Dependency Map

```
Wave 1 (2 parallel — independent backend work):
  S37-001: src-tauri/src/hw/audio.rs (IPolicyConfig COM)
  S37-003: src-tauri/src/hw/wifi.rs (WlanAPI scan_networks)

Wave 2 (after Wave 1):
  S37-002: commands/hardware.rs + lib.rs + AudioControl.tsx + useHardware.ts (depends on 001)
  S37-004: src-tauri/src/hw/wifi.rs (WlanAPI get_status — depends on 003 for WlanAPI patterns)
  S37-005: src/i18n/locales/*.json (depends on 002 for key names)
```

## Commit Strategy

One commit per ticket:

1. `feat(s37-001): implement IPolicyConfig SetDefaultEndpoint for audio device switching`
2. `feat(s37-002): register set_audio_default_endpoint command and wire up frontend`
3. `refactor(s37-003): replace netsh wlan scan with native WlanAPI for locale-independent WiFi scanning`
4. `refactor(s37-004): replace netsh wlan show interfaces with WlanQueryInterface for connection status`
5. `i18n(s37-005): add setAsDefault translation keys for audio device switching`

## What Was Deferred

| Ticket                                               | Reason                                             | Next Action   |
| ---------------------------------------------------- | -------------------------------------------------- | ------------- |
| WlanRegisterNotification (async scan)                | More complex, requires callback                    | Future sprint |
| IAudioEndpointVolumeCallback (real-time volume)      | COM callback for volume change notifications       | Future sprint |
| WlanSetProfile + WlanConnect (replace netsh connect) | Larger change, connect error parsing less critical | Future sprint |

---

## Sprint Completion Checklist

After all tickets are committed:

- [ ] All 5 tickets have passing health checks (9/9)
- [ ] All commits pushed to `main`
- [ ] `sprint-overview.md` updated with Sprint 37 status
- [ ] Manual test: Click a non-default audio device → it becomes default (verify in Windows Sound Settings)
- [ ] Manual test: Click the already-default device → no-op (button disabled)
- [ ] Manual test: WiFi networks appear on Portuguese Windows (currently they don't)
- [ ] Manual test: WiFi networks appear on English Windows (regression check)
- [ ] Manual test: WiFi connection status is correct on Portuguese Windows
- [ ] **CRITICAL:** IPolicyConfig vtable slot validated on Windows 11 target
- [ ] No `netsh` calls remain in `wifi.rs` (verified via grep)
