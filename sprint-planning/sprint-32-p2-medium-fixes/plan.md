# Sprint 32 — P2 MEDIUM: Hardware Reliability & Security Hardening

> **Date:** 2026-06-27
> **Sprint:** 32
> **Theme:** Fix 7 medium-priority issues — adaptive brightness, device discovery, IoT elevation, battery cycles, diagnostics UX, IoT retry, credential security
> **Duration:** ~3–4 days
> **Dependencies:** Sprint 31 (all P1 high fixes)
> **Status:** 📌 Active
> **Audit Reference:** `C:\Users\mafsc\Documents\Audit_Final.md` (M-1 through M-7)

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **OBRIGATÓRIO: 100% dos tickets desta sprint devem ser concluídos. A sprint não será aceita como entregue se qualquer ticket permanecer incompleto.**

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

This sprint addresses 7 medium-priority issues that affect hardware reliability and security. While not blocking, these issues cause user frustration and represent incomplete implementations:

1. **M-1:** Auto-brightness doesn't work — the loop is spawned but sensor detection and error logging are insufficient
2. **M-2:** Device model shows "unknown" until manual scan — auto-discovery not triggered on mount
3. **M-3:** IoT module requires admin relaunch despite having an elevated bridge
4. **M-4:** Battery cycle count always zero — WMI cache poisoned on first failure
5. **M-5:** Channel Diagnostics shows confusing mismatch — resolved by S30-001, but needs UI note
6. **M-6:** IoT tab never loads — no retry, no diagnostic info
7. **M-7:** `set_secret`/`delete_secret` missing allowlist — security gap

---

## Goals

| #   | Goal                                                  | KPI                                                      | Audit Reference |
| --- | ----------------------------------------------------- | -------------------------------------------------------- | --------------- |
| 1   | Auto-brightness works when sensor is available        | Brightness adjusts based on ambient lux                  | M-1             |
| 2   | Device model shows on first visit without manual scan | Profile loaded on mount, auto-discovery if null          | M-2             |
| 3   | IoT module accessible without admin relaunch          | Uses elevated bridge, no lock screen                     | M-3             |
| 4   | Battery cycle count shows real value                  | Non-zero cycle count from WMI or powercfg fallback       | M-4             |
| 5   | Channel Diagnostics has explanatory note              | User understands overlay vs registry mismatch            | M-5             |
| 6   | IoT tab retries automatically and shows diagnostics   | Auto-retry every 5s, pipe path shown                     | M-6             |
| 7   | All credential commands have allowlist                | `set_secret`/`delete_secret` reject non-allowlisted keys | M-7             |

---

## Technical Specs

### S32-001: Fix adaptive brightness sensor detection and error logging (M-1)

| Field         | Value                                                                  |
| ------------- | ---------------------------------------------------------------------- |
| **Ticket ID** | S32-001                                                                |
| **Title**     | Add sensor-found logging and error logging to adaptive brightness loop |
| **Priority**  | P2 — Medium                                                            |
| **Source**    | M-1 (Audit_Final.md)                                                   |
| **Files**     | `src-tauri/src/hw/display.rs`                                          |
| **Effort**    | ~1.5 hours                                                             |
| **Type**      | Backend (Rust)                                                         |

#### Problem

The adaptive brightness loop is spawned in `lib.rs:427` and runs every 2s. However:

1. `get_ambient_lux()` doesn't log when the sensor IS found, only when it's not
2. `set_ai_brightness()` doesn't log errors when the registry write fails
3. The loop logs "no ambient light sensor found — loop idle" only once, but doesn't provide enough diagnostic info

#### Current Code

```rust
// display.rs:228-234 — get_ambient_lux (no success logging)
fn get_ambient_lux() -> Option<f32> {
    use windows::Devices::Sensors::LightSensor;
    let sensor = LightSensor::GetDefault().ok()?;
    let reading = sensor.GetCurrentReading().ok()?;
    reading.IlluminanceInLux().ok()
}

// display.rs:160-170 — set_ai_brightness (no error logging)
pub fn set_ai_brightness(enabled: bool) -> HardwareResult<()> {
    let mut cfg = get_ai_brightness_config();
    cfg.enabled = enabled;
    set_ai_brightness_config(cfg)?;
    if enabled {
        disable_windows_adaptive_brightness();
    }
    Ok(())
}
```

#### Solution

**Step 1:** Add success logging to `get_ambient_lux()`:

```rust
#[cfg(windows)]
fn get_ambient_lux() -> Option<f32> {
    use windows::Devices::Sensors::LightSensor;
    let sensor = LightSensor::GetDefault().ok()?;
    log::debug!("[display] Ambient light sensor found");
    let reading = sensor.GetCurrentReading().ok()?;
    let lux = reading.IlluminanceInLux().ok()?;
    log::debug!("[display] Ambient lux: {lux}");
    Some(lux)
}
```

**Step 2:** Add error logging to `set_ai_brightness()`:

```rust
pub fn set_ai_brightness(enabled: bool) -> HardwareResult<()> {
    let mut cfg = get_ai_brightness_config();
    cfg.enabled = enabled;
    set_ai_brightness_config(cfg).map_err(|e| {
        log::error!("[display] set_ai_brightness_config failed: {e}");
        e
    })?;
    if enabled {
        disable_windows_adaptive_brightness();
    }
    log::info!("[display] AI brightness {}", if enabled { "enabled" } else { "disabled" });
    Ok(())
}
```

**Step 3:** Improve the loop's no-sensor warning to include more context:

```rust
// In adaptive_brightness_loop(), the no_sensor branch:
Ok(None) => {
    if !no_sensor_warned {
        log::warn!(
            "adaptive_brightness: no ambient light sensor found — loop idle. \
             LightSensor::GetDefault() returned None. \
             Check: (1) sensor driver installed, (2) sensor enabled in Device Manager, \
             (3) Devices_Sensors feature in Cargo.toml (confirmed present)."
        );
        no_sensor_warned = true;
    }
    continue;
}
```

#### Acceptance Criteria

- [ ] `get_ambient_lux()` logs at `debug` level when sensor is found and when lux is read
- [ ] `set_ai_brightness()` logs at `error` level when config write fails
- [ ] `set_ai_brightness()` logs at `info` level when enabled/disabled
- [ ] No-sensor warning includes diagnostic hints
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes

---

### S32-002: Auto-discover hardware profile on mount if null (M-2)

| Field         | Value                                                             |
| ------------- | ----------------------------------------------------------------- |
| **Ticket ID** | S32-002                                                           |
| **Title**     | Trigger auto-discovery when `refreshHardwareProfile` returns null |
| **Priority**  | P2 — Medium                                                       |
| **Source**    | M-2 (Audit_Final.md)                                              |
| **Files**     | `src/hooks/useHardware.ts`                                        |
| **Effort**    | ~1 hour                                                           |
| **Type**      | Frontend (TypeScript)                                             |

#### Problem

`refreshHardwareProfile` is called on mount (`useHardware.ts:712`), but if the backend hasn't discovered the hardware yet (no cached profile), it returns `null`. The UI shows "unknown" until the user manually clicks "Re-scan".

#### Solution

Add auto-discovery fallback to `doInitialLoad`:

```typescript
// In doInitialLoad, after the existing Promise.all:
const [
  fanResult,
  systemResult,
  batteryResult,
  displayResult,
  touchpadResult,
  perfMode,
  chargeThreshold,
] = await Promise.all([
  /* ... existing ... */
]);

// ... set all states ...

// Load hardware profile, auto-discover if not cached
try {
  const profile = await invoke<HardwareProfile | null>('get_hardware_profile');
  if (profile) {
    setHardwareProfile(profile);
  } else {
    // Profile not cached — trigger discovery automatically
    try {
      const discovered = await invoke<HardwareProfile>('run_hardware_discovery');
      setHardwareProfile(discovered);
    } catch (e) {
      console.warn('[hardware] Auto-discovery failed:', e);
    }
  }
} catch (e) {
  console.warn('[hardware] Failed to load profile:', e);
}
```

#### Acceptance Criteria

- [ ] `doInitialLoad` calls `get_hardware_profile` after the main `Promise.all`
- [ ] If profile is null, calls `run_hardware_discovery` automatically
- [ ] Device model shows on first visit without manual scan
- [ ] `npx tsc --noEmit` passes
- [ ] `npm run build` succeeds

---

### S32-003: Remove IoT module elevation gate, use bridge (M-3)

| Field         | Value                                                               |
| ------------- | ------------------------------------------------------------------- |
| **Ticket ID** | S32-003                                                             |
| **Title**     | Remove `isElevated()` gate from IotModulePanel, use elevated bridge |
| **Priority**  | P2 — Medium                                                         |
| **Source**    | M-3 (Audit_Final.md)                                                |
| **Files**     | `src/pages/tabs/setup.tsx`                                          |
| **Effort**    | ~1 hour                                                             |
| **Type**      | Frontend (TypeScript)                                               |

#### Problem

The `IotModulePanel` in `setup.tsx:88-260` checks `hw.isElevated()` and shows a lock screen with "Re-launch as Administrator" if not elevated. But the elevated bridge exists for exactly this purpose — EC RAM operations can go through the bridge instead of requiring the entire process to be elevated.

#### Current Code (BROKEN)

```tsx
// setup.tsx — elevation gate
if (elevated === false) {
  return (
    <div className="card">// ... lock screen with "Re-launch as Administrator" button ...</div>
  );
}
```

#### Solution

Remove the elevation gate entirely. The EC RAM operations (`get_ecram_map`, `write_iot_hex`, `read_ecram_raw`) already go through the elevated bridge when the process is not elevated.

```tsx
// Remove the entire `if (elevated === false) { ... }` block
// Remove the `checkElevation` callback and `elevated` state
// Remove the `handleRelaunch` callback and `relaunching` state

// The component should directly render the EC RAM content:
export default function IotModulePanel({ hw }: { hw: Hardware }) {
  // ... existing state for ecramMap, regions, etc. ...
  // ... refreshIot callback ...

  useEffect(() => {
    void refreshIot();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="card" style={{ marginTop: 14 }}>
      {/* ... existing EC RAM content without the elevation check ... */}
    </div>
  );
}
```

**Note:** Verify that `get_ecram_map`, `write_iot_hex`, and `read_ecram_raw` commands in `commands/hardware.rs` use `elev_bridge::run_elevated` when the process is not elevated. If they don't, add the bridge call.

#### Acceptance Criteria

- [ ] No `isElevated()` check in IotModulePanel
- [ ] No "Re-launch as Administrator" lock screen
- [ ] EC RAM operations work without process elevation (via bridge)
- [ ] `npx tsc --noEmit` passes
- [ ] `npm run build` succeeds

---

### S32-004: Fix battery cycle count with powercfg fallback (M-4)

| Field         | Value                                                                     |
| ------------- | ------------------------------------------------------------------------- |
| **Ticket ID** | S32-004                                                                   |
| **Title**     | Add `powercfg /batteryreport` fallback for cycle count when WMI returns 0 |
| **Priority**  | P2 — Medium                                                               |
| **Source**    | M-4 (Audit_Final.md)                                                      |
| **Files**     | `src-tauri/src/hw/battery.rs`                                             |
| **Effort**    | ~2 hours                                                                  |
| **Type**      | Backend (Rust)                                                            |

#### Problem

The cycle count is read from WMI `BatteryStaticData.CycleCount` and cached in a `OnceLock`. If the first WMI query fails (e.g., WMI service not ready at boot), the cache is poisoned with `cycle_count: 0` forever.

#### Solution

**Step 1:** Don't cache cycle_count permanently — read it fresh or use powercfg fallback:

```rust
pub fn get_battery_info() -> HardwareResult<BatteryInfo> {
    // ... existing code for other fields ...

    // Try WMI first (cached for static data like designed capacity)
    let battery_static = BATTERY_STATIC_DATA.get_or_init(|| {
        // ... existing WMI query ...
    });

    // If cycle_count is 0, try powercfg as fallback (not cached)
    let cycle_count = if battery_static.cycle_count > 0 {
        battery_static.cycle_count
    } else {
        get_cycle_count_powercfg().unwrap_or(0)
    };

    // ... rest of function using cycle_count ...
}
```

**Step 2:** Implement `get_cycle_count_powercfg()`:

```rust
/// Read battery cycle count from `powercfg /batteryreport /xml` output.
///
/// This is a fallback when WMI `BatteryStaticData.CycleCount` returns 0
/// or is unavailable. The XML output contains a `<CycleCount>` element
/// under `<Battery>` → `<BatteryInformation>`.
#[cfg(windows)]
fn get_cycle_count_powercfg() -> Option<u32> {
    use std::process::Command;
    use std::os::windows::process::CommandExt;

    let mut cmd = Command::new("powercfg");
    cmd.args(["/batteryreport", "/xml", "/output", "-"]);
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

    let output = cmd.output().ok()?;
    let xml = String::from_utf8_lossy(&output.stdout);

    // Parse <CycleCount> element from XML
    // The battery report XML has this structure:
    // <BatteryReport>
    //   <Batteries>
    //     <Battery>
    //       <CycleCount>42</CycleCount>
    //     </Battery>
    //   </Batteries>
    // </BatteryReport>

    // Simple XML parsing (avoid pulling in a full XML parser dependency)
    if let Some(start) = xml.find("<CycleCount>") {
        if let Some(end) = xml[start..].find("</CycleCount>") {
            let value_str = &xml[start + 12..start + end];
            return value_str.trim().parse::<u32>().ok();
        }
    }

    None
}

#[cfg(not(windows))]
fn get_cycle_count_powercfg() -> Option<u32> {
    None
}
```

#### Acceptance Criteria

- [ ] `get_cycle_count_powercfg()` implemented with `CREATE_NO_WINDOW`
- [ ] `get_battery_info()` uses powercfg fallback when WMI cycle_count is 0
- [ ] Cycle count shows a non-zero value when the battery has been cycled
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes

---

### S32-005: Add explanatory note to Channel Diagnostics (M-5)

| Field         | Value                                                           |
| ------------- | --------------------------------------------------------------- |
| **Ticket ID** | S32-005                                                         |
| **Title**     | Add UI note explaining overlay vs registry mismatch             |
| **Priority**  | P2 — Medium                                                     |
| **Source**    | M-5 (Audit_Final.md)                                            |
| **Files**     | `src/pages/tabs/performance.tsx`, `src/i18n/{en,pt,es,fr}.json` |
| **Effort**    | ~45 minutes                                                     |
| **Type**      | Frontend (TypeScript + i18n)                                    |

#### Problem

The Channel Diagnostics panel shows the MI registry mode and the Windows overlay mode independently. The overlay has only 3 GUIDs (Efficiency/Balanced/Performance) while the registry has 12+ modes. This causes apparent "mismatches" that are actually expected behavior.

**Note:** The core issue (stale mode display) is resolved by S30-001. This ticket adds the explanatory note.

#### Solution

Add a note below the Channel Diagnostics panel:

```tsx
// In performance.tsx, after the debugInfo section:
{
  debugInfo && (
    <div style={{ fontSize: 11, color: 'var(--text-dim)', marginTop: 8, lineHeight: 1.5 }}>
      {t('performance.channels.note')}
    </div>
  );
}
```

Add the translation key in each language file:

**`en.json`** (in `performance.channels`):

```json
"note": "MI Registry stores the exact performance mode. The Windows Power Overlay has only 3 levels (Efficiency, Balanced, Performance), so multiple MI modes share the same overlay GUID. This is expected behavior."
```

**`pt.json`**:

```json
"note": "O registo MI armazena o modo de desempenho exato. O Windows Power Overlay tem apenas 3 níveis (Eficiência, Equilibrado, Desempenho), pelo que vários modos MI partilham o mesmo GUID de overlay. Este comportamento é esperado."
```

**`es.json`**:

```json
"note": "El registro MI almacena el modo de rendimiento exacto. El Windows Power Overlay tiene solo 3 niveles (Eficiencia, Equilibrado, Rendimiento), por lo que múltiples modos MI comparten el mismo GUID de overlay. Este comportamiento es esperado."
```

**`fr.json`**:

```json
"note": "Le registre MI stocke le mode de performance exact. Le Windows Power Overlay n'a que 3 niveaux (Efficacité, Équilibré, Performance), donc plusieurs modes MI partagent le même GUID d'overlay. Ce comportement est attendu."
```

#### Acceptance Criteria

- [ ] Explanatory note appears below Channel Diagnostics when `debugInfo` is present
- [ ] Note is translated in all 4 languages
- [ ] `npx tsc --noEmit` passes
- [ ] `npm run build` succeeds

---

### S32-006: Add auto-retry and diagnostics to IoT tab (M-6)

| Field         | Value                                               |
| ------------- | --------------------------------------------------- |
| **Ticket ID** | S32-006                                             |
| **Title**     | Add auto-retry and diagnostic info to IotDeviceCard |
| **Priority**  | P2 — Medium                                         |
| **Source**    | M-6 (Audit_Final.md)                                |
| **Files**     | `src/components/IotDeviceCard.tsx`                  |
| **Effort**    | ~1.5 hours                                          |
| **Type**      | Frontend (TypeScript)                               |

#### Problem

When the IoTService pipe is unavailable, `IotDeviceCard` shows "IoT Service not available" with no retry and no diagnostic info. The user has no way to know what's wrong or when it might become available.

#### Solution

```tsx
import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { IotDeviceInfo } from '../types/hardware';

export default function IotDeviceCard() {
  const [info, setInfo] = useState<IotDeviceInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [retryCount, setRetryCount] = useState(0);

  const loadInfo = useCallback(async () => {
    try {
      const data = await invoke<IotDeviceInfo>('get_iot_device_info');
      setInfo(data);
    } catch (e) {
      console.error('Failed to load IoT device info:', e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadInfo();
  }, [loadInfo]);

  // Auto-retry every 5 seconds when pipe is not available
  useEffect(() => {
    if (info?.pipe_available === false) {
      const timer = setTimeout(() => {
        setRetryCount((c) => c + 1);
        void loadInfo();
      }, 5000);
      return () => clearTimeout(timer);
    }
  }, [info?.pipe_available, retryCount, loadInfo]);

  if (loading) {
    return (
      <div className="card">
        <div className="card-title">🔌 IoT Device</div>
        <p className="page-subtitle">Loading device information...</p>
      </div>
    );
  }

  if (!info?.pipe_available) {
    return (
      <div className="card">
        <div className="card-title">🔌 IoT Device</div>
        <p className="page-subtitle" style={{ color: 'var(--text-dim)' }}>
          IoT Service not available. The Xiaomi IoT chip was not detected on this system.
        </p>
        <div style={{ marginTop: 12, fontSize: 12, color: 'var(--text-muted)' }}>
          <div>
            Expected pipe: <code>{'\\\\.\\pipe\\LOCAL\\IoTService_IPC_Broker'}</code>
          </div>
          <div style={{ marginTop: 4 }}>
            Status: Not found
            {retryCount > 0 && ` (retry ${retryCount}...)`}
          </div>
          <div style={{ marginTop: 8, lineHeight: 1.5 }}>
            Ensure Xiaomi PC Manager is installed and IoTService is running. The system will
            automatically retry every 5 seconds.
          </div>
        </div>
        <button
          className="btn-secondary"
          style={{ marginTop: 12, fontSize: 12 }}
          onClick={() => void loadInfo()}
        >
          Refresh now
        </button>
      </div>
    );
  }

  // ... existing rendering for when pipe IS available ...
}
```

#### Acceptance Criteria

- [ ] Auto-retry every 5 seconds when `pipe_available` is false
- [ ] Retry count displayed
- [ ] Expected pipe path shown in diagnostic info
- [ ] "Refresh now" button for manual retry
- [ ] Auto-retry stops when pipe becomes available
- [ ] `npx tsc --noEmit` passes
- [ ] `npm run build` succeeds

---

### S32-007: Add allowlist to `set_secret` and `delete_secret` (M-7)

| Field         | Value                                                               |
| ------------- | ------------------------------------------------------------------- |
| **Ticket ID** | S32-007                                                             |
| **Title**     | Add `ALLOWED_SECRET_KEYS` check to `set_secret` and `delete_secret` |
| **Priority**  | P2 — Medium                                                         |
| **Source**    | M-7 (Audit_Final.md)                                                |
| **Files**     | `src-tauri/src/commands/credentials.rs`                             |
| **Effort**    | ~30 minutes                                                         |
| **Type**      | Backend (Rust)                                                      |

#### Problem

`get_secret` has an allowlist (`ALLOWED_SECRET_KEYS`), but `set_secret` and `delete_secret` do not. Any frontend code could write or delete arbitrary keys in the credential store.

#### Current Code

```rust
// credentials.rs — set_secret has NO allowlist
#[tauri::command]
pub fn set_secret(key: String, value: String) -> Result<(), String> {
    let entry = Entry::new(SERVICE_NAME, &key).map_err(|e| e.to_string())?;
    entry.set_password(&value).map_err(|e| e.to_string())?;
    // ... audit log ...
    Ok(())
}

// delete_secret has NO allowlist
#[tauri::command]
pub fn delete_secret(key: String) -> Result<(), String> {
    let entry = Entry::new(SERVICE_NAME, &key).map_err(|e| e.to_string())?;
    // ...
}
```

#### Solution

Add the allowlist check to both functions:

```rust
#[tauri::command]
pub fn set_secret(key: String, value: String) -> Result<(), String> {
    if !ALLOWED_SECRET_KEYS.contains(&key.as_str()) {
        return Err(format!(
            "Access denied: key '{key}' is not in the allowlist"
        ));
    }
    let entry = Entry::new(SERVICE_NAME, &key).map_err(|e| e.to_string())?;
    entry.set_password(&value).map_err(|e| e.to_string())?;

    // Audit log for telemetry consent grant/revoke
    if key == "telemetry_consent" && (value == "granted" || value.contains("\"granted\"")) {
        crate::util::consent_audit::log_consent_granted(crate::util::consent_audit::POLICY_VERSION);
    }

    Ok(())
}

#[tauri::command]
pub fn delete_secret(key: String) -> Result<(), String> {
    if !ALLOWED_SECRET_KEYS.contains(&key.as_str()) {
        return Err(format!(
            "Access denied: key '{key}' is not in the allowlist"
        ));
    }
    let entry = Entry::new(SERVICE_NAME, &key).map_err(|e| e.to_string())?;
    match entry.delete_credential() {
        Ok(_) => {
            if key == "telemetry_consent" {
                crate::util::consent_audit::log_consent_revoked(
                    crate::util::consent_audit::POLICY_VERSION,
                );
            }
            Ok(())
        }
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
```

#### Acceptance Criteria

- [ ] `set_secret` rejects keys not in `ALLOWED_SECRET_KEYS`
- [ ] `delete_secret` rejects keys not in `ALLOWED_SECRET_KEYS`
- [ ] Error message includes the rejected key name
- [ ] Existing functionality for `openai_api_key` and `telemetry_consent` still works
- [ ] Unit test: `test_set_secret_rejects_unknown_key`
- [ ] Unit test: `test_delete_secret_rejects_unknown_key`
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes

---

## Story Points

| Ticket    | Points | Owner    | Wave                                     |
| --------- | ------ | -------- | ---------------------------------------- |
| S32-001   | 2      | Backend  | 1 (display.rs — independent)             |
| S32-002   | 2      | Frontend | 1 (useHardware.ts — independent)         |
| S32-003   | 2      | Frontend | 1 (setup.tsx — independent)              |
| S32-004   | 3      | Backend  | 1 (battery.rs — independent)             |
| S32-005   | 1      | Frontend | 1 (performance.tsx + i18n — independent) |
| S32-006   | 2      | Frontend | 1 (IotDeviceCard.tsx — independent)      |
| S32-007   | 1      | Backend  | 1 (credentials.rs — independent)         |
| **Total** | **13** |          |                                          |

## Dependency Map

```
Wave 1 (all parallel — 7 independent tickets):
  S32-001: src-tauri/src/hw/display.rs
  S32-002: src/hooks/useHardware.ts
  S32-003: src/pages/tabs/setup.tsx
  S32-004: src-tauri/src/hw/battery.rs
  S32-005: src/pages/tabs/performance.tsx + src/i18n/*.json
  S32-006: src/components/IotDeviceCard.tsx
  S32-007: src-tauri/src/commands/credentials.rs
```

All 7 tickets modify different files and have no logical dependencies.

## Commit Strategy

One commit per ticket:

1. `fix(s32-001): add sensor detection logging to adaptive brightness loop`
2. `feat(s32-002): auto-discover hardware profile on mount when null`
3. `fix(s32-003): remove IoT elevation gate, use elevated bridge`
4. `fix(s32-004): add powercfg fallback for battery cycle count`
5. `feat(s32-005): add explanatory note to channel diagnostics`
6. `fix(s32-006): add auto-retry and diagnostics to IoT device card`
7. `fix(s32-007): add allowlist to set_secret and delete_secret`

## What Was Deferred

| Ticket | Reason | Next Action |
| ------ | ------ | ----------- |
| —      | —      | —           |

No items deferred.
