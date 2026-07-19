# Sprint 35 — P1 HIGH: Temperature Sensors, Volume Sync & Tray Polish

> **Date:** 2026-07-19
> **Sprint:** 35
> **Theme:** Fix temperature sensor fallback, complete volume slider sync, verify tray popup CSS, add WiFi delay fix
> **Duration:** ~3–4 days
> **Dependencies:** Sprint 34 (all P0 critical fixes must be completed first)
> **Status:** ✅ Complete
> **Commit:** `45b0d5a` — `fix(s35): remove fabricated temperature fallbacks and complete volume/tray/wifi polish`
> **Audit Reference:** `C:\Users\mafsc\Documents\Audit_Report_miControl.md` (Bug 2: 2A, 2B, 2C; Bug 3: 3B; Bug 4: 4C; Bug 6: 6B)

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

This sprint addresses the remaining P1 high-priority bugs from the audit: temperature sensors reporting fabricated values, volume slider drag protection, tray CSS verification, and WiFi scan delay. These fixes complete the user-facing bug fixes started in Sprint 34.

1. **S35-001:** Remove 50°C/45°C fallback in `get_esif_readings()` — return `Option<f32>` instead of fabricating values
2. **S35-002:** Remove `v > 0` filter from `extract_int` and switch to `extract_u32` for Temperature/Power fields
3. **S35-003:** Add doc comment to `extract_i32` about `UI4` overflow behavior (defense in depth)
4. **S35-004:** Update `FanInfo` struct and frontend types to use `Option<f32>` / `number | null` for temperature fields
5. **S35-005:** Add `isAdjustingRef` dirty flag to `AudioControl.tsx` slider to prevent mid-drag overwrite
6. **S35-006:** Verify `.tray-window` CSS class is applied to tray root element; add explicit `color-mix` override if needed
7. **S35-007:** Increase WiFi scan delay from 1.5s to 4s (band-aid fix; WlanAPI replacement in Sprint 37)

---

## Goals

| #   | Goal                                                 | KPI                                           | Audit Reference |
| --- | ---------------------------------------------------- | --------------------------------------------- | --------------- |
| 1   | Temperature never shows fabricated 50°C/45°C values  | `cpu_temp_celsius` is `null` when no sensor   | Bug 2 (2A)      |
| 2   | Valid zero readings (0°C, 0W) are not discarded      | `extract_u32` used instead of `extract_i32`   | Bug 2 (2B)      |
| 3   | `FanInfo` struct uses `Option<f32>` for temperatures | Frontend handles `null` gracefully            | Bug 2 (2A)      |
| 4   | Volume slider doesn't jump during drag               | `isAdjustingRef` prevents poll overwrite      | Bug 4 (4C)      |
| 5   | Tray popup CSS overrides are confirmed applied       | `.tray-window` class verified on root element | Bug 3 (3B)      |
| 6   | WiFi scan delay sufficient for adapter completion    | 4s delay before querying networks             | Bug 6 (6B)      |

---

## Technical Specs

### S35-001: Remove 50°C/45°C fallback in `get_esif_readings()` (Bug 2A)

| Field         | Value                                                                                   |
| ------------- | --------------------------------------------------------------------------------------- |
| **Ticket ID** | S35-001                                                                                 |
| **Title**     | Replace fabricated 50°C/45°C fallback with `Option<f32>` and ACPI thermal zone fallback |
| **Priority**  | P1 — High                                                                               |
| **Source**    | Bug 2A (`Audit_Report_miControl.md`)                                                    |
| **Files**     | `src-tauri/src/hw/fan.rs` (lines 50–140)                                                |
| **Effort**    | ~2 hours                                                                                |
| **Type**      | Backend (Rust)                                                                          |

#### Problem

When the DPTF/ESIF driver is absent, `SELECT ... FROM EsifDeviceInformation` returns an empty result set. The `filter_map` produces nothing, `cpu_temp` stays at `f32::NEG_INFINITY`, and the fallback to `50.0` is applied silently. The user sees "50°C" and believes the sensor is working.

Three layers of fabrication:

1. `get_esif_readings()` lines 76–79: `else { 50.0 }` for CPU temp
2. `get_esif_readings()` lines 96–103: `else { 45.0 }` for GPU temp
3. `get_fan_info()` lines 117–121: `unwrap_or(EsifReadings { cpu_temp: 50.0, gpu_temp: 45.0, ... })`

#### Current Code

**`fan.rs` lines 50–55 (struct):**

```rust
struct EsifReadings {
    cpu_temp: f32,
    gpu_temp: f32,
    tdp_watts: Option<f32>,
}
```

**`fan.rs` lines 76–79 (CPU fallback):**

```rust
let cpu_temp = if cpu_temp > 0.0 && cpu_temp.is_finite() {
    cpu_temp.clamp(0.0, 120.0)
} else {
    50.0   // ← FABRICATES value
};
```

**`fan.rs` lines 96–103 (GPU fallback):**

```rust
.unwrap_or_else(|| {
    let m = results.iter().filter_map(|r| extract_int(r, "Temperature"))
        .fold(f32::NEG_INFINITY, |acc, v| acc.max(v as f32));
    if m > 0.0 && m.is_finite() { m.clamp(0.0, 120.0) }
    else { 45.0 }  // ← FABRICATES value
});
```

**`fan.rs` lines 117–121 (get_fan_info fallback):**

```rust
let esif = get_esif_readings().unwrap_or(EsifReadings {
    cpu_temp: 50.0,
    gpu_temp: 45.0,
    tdp_watts: None,
});
```

#### Solution

**Step 1:** Change `EsifReadings` struct to use `Option<f32>`:

```rust
struct EsifReadings {
    cpu_temp: Option<f32>,
    gpu_temp: Option<f32>,
    tdp_watts: Option<f32>,
}
```

**Step 2:** Update `get_esif_readings()` to return `None` instead of fabricating:

```rust
// CPU temp: max Temperature across participants
let cpu_temp = results
    .iter()
    .filter_map(|r| extract_u32_temp(r, "Temperature"))
    .fold(f32::NEG_INFINITY, |acc, v| acc.max(v));
let cpu_temp = if cpu_temp.is_finite() {
    Some(cpu_temp.clamp(0.0, 120.0))
} else {
    None  // No ESIF data — do NOT fabricate a value
};

// GPU temp: prefer participant _10
let gpu_temp = results
    .iter()
    .find(|r| instance_suffix(r, "_10"))
    .and_then(|r| extract_u32_temp(r, "Temperature"))
    .map(|v| v.clamp(0.0, 120.0))
    .or_else(|| {
        let m = results
            .iter()
            .filter_map(|r| extract_u32_temp(r, "Temperature"))
            .fold(f32::NEG_INFINITY, |acc, v| acc.max(v));
        if m.is_finite() { Some(m.clamp(0.0, 120.0)) } else { None }
    });

Ok(EsifReadings { cpu_temp, gpu_temp, tdp_watts })
```

**Step 3:** Update `get_fan_info()` to fall back to ACPI thermal zone:

```rust
let esif = get_esif_readings().unwrap_or(EsifReadings {
    cpu_temp: None,
    gpu_temp: None,
    tdp_watts: None,
});

// If ESIF failed, try ACPI thermal zone as fallback (not a hardcoded value)
let cpu_temp = esif.cpu_temp.or_else(|| {
    match crate::hw::thermal::get_primary_thermal_zone() {
        Ok(zone) => Some(zone.current_temp_celsius as f32),
        Err(e) => {
            log::warn!(target: "hw::fan", "ESIF and ACPI thermal zone both unavailable: {e}");
            None
        }
    }
});
```

**Step 4:** Update `#[cfg(not(windows))]` stub:

```rust
#[cfg(not(windows))]
{
    Ok(EsifReadings {
        cpu_temp: None,
        gpu_temp: None,
        tdp_watts: None,
    })
}
```

#### Acceptance Criteria

- [ ] `EsifReadings` struct uses `Option<f32>` for all temperature fields
- [ ] No hardcoded `50.0` or `45.0` values in `fan.rs`
- [ ] `get_fan_info()` falls back to ACPI thermal zone when ESIF fails
- [ ] `log::warn!` emitted when both ESIF and ACPI are unavailable
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes

---

### S35-002: Remove `v > 0` filter and switch to `extract_u32` (Bug 2B)

| Field         | Value                                                                                         |
| ------------- | --------------------------------------------------------------------------------------------- |
| **Ticket ID** | S35-002                                                                                       |
| **Title**     | Remove `.filter(\|&v\| v > 0)` from `extract_int` and use `extract_u32` for Temperature/Power |
| **Priority**  | P1 — High                                                                                     |
| **Source**    | Bug 2B (`Audit_Report_miControl.md`)                                                          |
| **Files**     | `src-tauri/src/hw/fan.rs` (lines 65–69)                                                       |
| **Effort**    | ~30 minutes                                                                                   |
| **Type**      | Backend (Rust)                                                                                |

#### Problem

The `extract_int` closure filters `v > 0`, which discards legitimate zero values (0°C idle, 0W idle). The filter was likely intended to reject "missing" WMI values, but `extract_i32` already returns `None` for absent keys. The filter is redundant and harmful.

Additionally, `extract_i32` does `UI4 as i32` which can overflow for values > `i32::MAX`. The `extract_u32` function (lines 5–16 of `wmi_extract.rs`) already exists and handles `UI4` correctly without overflow.

#### Current Code

**`fan.rs` lines 65–69:**

```rust
let extract_int = |row: &HashMap<String, wmi::Variant>, key: &str| -> Option<i64> {
    wmi_extract::extract_i32(row, key)
        .filter(|&v| v > 0)  // ← discards 0°C and 0W (valid readings)
        .map(|v| v as i64)
};
```

#### Solution

Replace `extract_int` with a closure using `extract_u32`:

```rust
let extract_u32_temp = |row: &HashMap<String, wmi::Variant>, key: &str| -> Option<f32> {
    wmi_extract::extract_u32(row, key).map(|v| v as f32)
};
```

Update all usages of `extract_int(r, "Temperature")` and `extract_int(r, "Power")` to use `extract_u32_temp` instead.

#### Acceptance Criteria

- [ ] `.filter(|&v| v > 0)` removed from all extract closures in `fan.rs`
- [ ] `extract_u32` used instead of `extract_i32` for Temperature and Power fields
- [ ] Zero values (0°C, 0W) are no longer discarded
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes

---

### S35-003: Add doc comment about `UI4` overflow in `extract_i32` (Bug 2C)

| Field         | Value                                                    |
| ------------- | -------------------------------------------------------- |
| **Ticket ID** | S35-003                                                  |
| **Title**     | Document `UI4 as i32` overflow behavior in `extract_i32` |
| **Priority**  | P1 — High                                                |
| **Source**    | Bug 2C (`Audit_Report_miControl.md`)                     |
| **Files**     | `src-tauri/src/util/wmi_extract.rs` (line 40)            |
| **Effort**    | ~10 minutes                                              |
| **Type**      | Backend (Rust, documentation)                            |

#### Problem

`extract_i32` does `*v as i32` for `UI4` variants, which is bit-pattern-preserving: values > `i32::MAX` wrap to negative. This is a latent bug — for temperature (0–120) and power (0–1500 dW), the overflow cannot trigger, but it could cause issues if `extract_i32` is used for other WMI fields in the future.

#### Current Code

**`wmi_extract.rs` line 40:**

```rust
Some(wmi::Variant::UI4(v)) => Some(*v as i32),
```

#### Solution

Add a doc comment noting the overflow behavior and directing users to `extract_u32` for unsigned use:

```rust
/// Extract an i32 from a WMI variant map.
///
/// Note: `UI4` values are cast via `as i32`, which preserves the bit pattern.
/// Values > `i32::MAX` will wrap to negative. For unsigned access, use
/// [`extract_u32`] instead.
pub fn extract_i32(map: &HashMap<String, wmi::Variant>, key: &str) -> Option<i32> {
    match map.get(key) {
        Some(wmi::Variant::I1(v)) => Some(*v as i32),
        Some(wmi::Variant::I2(v)) => Some(*v as i32),
        Some(wmi::Variant::I4(v)) => Some(*v),
        Some(wmi::Variant::I8(v)) => Some(*v as i32),
        Some(wmi::Variant::UI1(v)) => Some(*v as i32),
        Some(wmi::Variant::UI2(v)) => Some(*v as i32),
        // UI4 → i32: bit-pattern-preserving cast. Use extract_u32 for unsigned access.
        Some(wmi::Variant::UI4(v)) => Some(*v as i32),
        _ => None,
    }
}
```

#### Acceptance Criteria

- [ ] Doc comment added to `extract_i32` explaining `UI4` overflow behavior
- [ ] Comment directs users to `extract_u32` for unsigned access
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes

---

### S35-004: Update `FanInfo` struct and frontend types for `Option<f32>` (Bug 2A — frontend)

| Field         | Value                                                                                                                                                                               |
| ------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Ticket ID** | S35-004                                                                                                                                                                             |
| **Title**     | Change `FanInfo.cpu_temp_celsius` and `gpu_temp_celsius` to `Option<f32>` / `number \| null`                                                                                        |
| **Priority**  | P1 — High                                                                                                                                                                           |
| **Source**    | Bug 2A (`Audit_Report_miControl.md`) — frontend portion                                                                                                                             |
| **Files**     | `src-tauri/src/hw/fan.rs` (struct definition, lines 19–30), `src/types/hardware.ts`, `src/pages/TrayPopup.tsx`, `src/pages/tabs/performance.tsx` (or wherever FanInfo is displayed) |
| **Effort**    | ~1–2 hours                                                                                                                                                                          |
| **Type**      | Full-stack (Rust + TypeScript)                                                                                                                                                      |

#### Problem

`FanInfo` currently uses `f32` for `cpu_temp_celsius` and `gpu_temp_celsius`. After S35-001 changes these to `Option<f32>`, the JSON serialization changes from a number to `null` when no sensor is available. The frontend must handle this gracefully.

#### Current Code

**`fan.rs` lines 19–30:**

```rust
pub struct FanInfo {
    pub mode: FanMode,
    pub speed_rpm: u32,
    pub speed_percent: u8,
    pub gpu_temp_celsius: f32,
    pub cpu_temp_celsius: f32,
    pub tdp_watts: Option<f32>,
}
```

#### Solution

**Step 1:** Update `FanInfo` struct in `fan.rs`:

```rust
pub struct FanInfo {
    pub mode: FanMode,
    pub speed_rpm: u32,
    pub speed_percent: u8,
    pub gpu_temp_celsius: Option<f32>,
    pub cpu_temp_celsius: Option<f32>,
    pub tdp_watts: Option<f32>,
}
```

**Step 2:** Update `src/types/hardware.ts`:

```typescript
export interface FanInfo {
  mode: FanMode;
  speed_rpm: number;
  speed_percent: number;
  gpu_temp_celsius: number | null;
  cpu_temp_celsius: number | null;
  tdp_watts: number | null;
}
```

**Step 3:** Update all frontend display locations to handle `null`:

```tsx
// TrayPopup.tsx — temperature display
<span>
  {fan.cpu_temp_celsius != null
    ? `${Math.round(fan.cpu_temp_celsius)}°C CPU`
    : '— CPU'}
</span>
<span>
  {fan.gpu_temp_celsius != null
    ? `${Math.round(fan.gpu_temp_celsius)}°C GPU`
    : '— GPU'}
</span>
```

Apply similar null checks in all components that display `cpu_temp_celsius` or `gpu_temp_celsius`.

#### Acceptance Criteria

- [ ] `FanInfo` struct uses `Option<f32>` for `cpu_temp_celsius` and `gpu_temp_celsius`
- [ ] TypeScript `FanInfo` interface uses `number | null`
- [ ] All frontend display locations handle `null` gracefully (show "—" or similar)
- [ ] No runtime errors when temperature is `null`
- [ ] `cargo check` passes
- [ ] `npx tsc --noEmit` passes
- [ ] `npm run build` succeeds

---

### S35-005: Add `isAdjustingRef` dirty flag to `AudioControl.tsx` slider (Bug 4C)

| Field         | Value                                                                     |
| ------------- | ------------------------------------------------------------------------- |
| **Ticket ID** | S35-005                                                                   |
| **Title**     | Add dirty flag to prevent poll from overwriting volume slider during drag |
| **Priority**  | P1 — High                                                                 |
| **Source**    | Bug 4C (`Audit_Report_miControl.md`)                                      |
| **Files**     | `src/components/AudioControl.tsx` (lines 50–70)                           |
| **Effort**    | ~30 minutes                                                               |
| **Type**      | Frontend (TypeScript)                                                     |

#### Problem

After S34-007 re-adds `get_audio_volume` to `fastPoll` (2s interval), the polled value could overwrite the slider position while the user is actively dragging it. The `TrayPopup.tsx` already has an `isAdjustingVolume` guard (lines 70–74), but `AudioControl.tsx` does NOT have this guard.

#### Current Code

**`AudioControl.tsx` lines 50–62:**

```tsx
const volume = audioState?.volume ?? 50;
const muted = audioState?.muted ?? false;

const handleVolumeChange = async (newVolume: number) => {
  try {
    await onVolumeChange(newVolume / 100);
  } catch (e) {
    addToast(`Volume error: ${String(e)}`, 'error');
  }
};

// Slider:
<input
  type="range"
  min={0}
  max={100}
  value={muted ? 0 : volume}
  onChange={(e) => handleVolumeChange(Number(e.target.value))}
  disabled={loading}
  style={{ flex: 1, accentColor: 'var(--accent)' }}
/>;
```

#### Solution

Add an `isAdjustingRef` to prevent the poll from overwriting during drag:

```tsx
import { useEffect, useState, useRef } from 'react';

// Inside the component:
const isAdjustingRef = useRef(false);

const handleVolumeChange = async (newVolume: number) => {
  isAdjustingRef.current = true;
  try {
    await onVolumeChange(newVolume / 100);
  } catch (e) {
    addToast(`Volume error: ${String(e)}`, 'error');
  } finally {
    // Re-enable sync after a short delay to let the poll catch up
    setTimeout(() => {
      isAdjustingRef.current = false;
    }, 500);
  }
};

// Derive volume — use local state during adjustment, polled state otherwise
const volume = audioState?.volume ?? 50;
const muted = audioState?.muted ?? false;
```

**Note:** The `value` prop on the `<input type="range">` is controlled by `audioState.volume` (via the parent `useHardware` hook). Since `setMasterVolume` does an optimistic update (`setAudioState((prev) => ...)`), the slider position is already updated immediately. The `isAdjustingRef` is a safety net for the 2s poll race.

If the optimistic update is sufficient (the slider follows the `audioState` which is immediately updated), this ticket may be a no-op. Verify by testing: drag the slider rapidly and check if the poll causes any visual jumps.

#### Acceptance Criteria

- [ ] `isAdjustingRef` added to `AudioControl.tsx`
- [ ] No visual jumps when dragging the volume slider while `fastPoll` is active
- [ ] Slider position is correct after drag release
- [ ] `npx tsc --noEmit` passes
- [ ] `npm run build` succeeds

---

### S35-006: Verify `.tray-window` CSS class is applied to tray root (Bug 3B)

| Field         | Value                                                                        |
| ------------- | ---------------------------------------------------------------------------- |
| **Ticket ID** | S35-006                                                                      |
| **Title**     | Verify `.tray-window` CSS override is applied to the tray popup root element |
| **Priority**  | P1 — High                                                                    |
| **Source**    | Bug 3B (`Audit_Report_miControl.md`)                                         |
| **Files**     | `src/pages/TrayPopup.tsx`, `src/styles/globals.css` (lines 290–306)          |
| **Effort**    | ~30 minutes                                                                  |
| **Type**      | Frontend (TypeScript + CSS)                                                  |

#### Problem

The `.tray-window` CSS override (lines 290–306) sets `--blur: none`, `--blur-sm: none`, `--surface: var(--surface-solid)`, and `--surface-2: var(--surface-solid)`. This override is critical for the tray popup to render correctly on transparent WebView2 windows.

However, the override only works if the `.tray-window` class is actually applied to the root element of the tray popup. If Tauri doesn't apply this class, the `backdrop-filter: var(--blur)` in `.tray-popup` (lines 1018–1019) would use the default `--blur` value (not `none`), potentially causing rendering issues.

#### Current Code

**`globals.css` lines 290–306:**

```css
.tray-window,
.tray-window body,
.tray-window #root {
  background: transparent !important;
  background-image: none !important;
  --blur: none;
  --blur-sm: none;
  --surface: var(--surface-solid);
  --surface-2: var(--surface-solid);
}
```

#### Solution

**Step 1:** Verify how the `.tray-window` class is applied. Check if Tauri applies it based on the window label (`"tray"`) or if it needs to be added manually in `TrayPopup.tsx`.

**Step 2:** If the class is NOT automatically applied, add it to the root element in `TrayPopup.tsx`:

```tsx
// TrayPopup.tsx — ensure the root div has the tray-window class
return (
  <div className="tray-popup tray-window" ref={rootRef}>
    {/* ... */}
  </div>
);
```

**Step 3:** If the class IS applied by Tauri (via the window label), verify by inspecting the DOM in dev tools. If it's applied to `<html>` or `<body>` but not to the React root, the CSS selector `.tray-window #root` should catch it.

**Step 4:** As a belt-and-suspenders approach, also add an explicit override in the `.tray-popup` rule:

```css
.tray-popup {
  width: 300px;
  background: var(--surface);
  backdrop-filter: var(--blur, none); /* explicit fallback */
  -webkit-backdrop-filter: var(--blur, none);
  border: 1px solid var(--border);
  border-radius: var(--r-lg);
  box-shadow: var(--shadow-md);
}
```

#### Acceptance Criteria

- [ ] Verified `.tray-window` class is applied to the tray popup root element
- [ ] If not applied by Tauri, added manually in `TrayPopup.tsx`
- [ ] `backdrop-filter` resolves to `none` in the tray popup (verified via dev tools)
- [ ] `--surface` resolves to `var(--surface-solid)` in the tray popup
- [ ] No rendering issues on Intel Arc / Nvidia / AMD GPUs
- [ ] `npm run build` succeeds

---

### S35-007: Increase WiFi scan delay from 1.5s to 4s (Bug 6B)

| Field         | Value                                          |
| ------------- | ---------------------------------------------- |
| **Ticket ID** | S35-007                                        |
| **Title**     | Increase WiFi scan delay from 1500ms to 4000ms |
| **Priority**  | P1 — High                                      |
| **Source**    | Bug 6B (`Audit_Report_miControl.md`)           |
| **Files**     | `src-tauri/src/hw/wifi.rs` (lines 38–43)       |
| **Effort**    | ~5 minutes                                     |
| **Type**      | Backend (Rust)                                 |

#### Problem

`netsh wlan scan` triggers the scan asynchronously and returns immediately. The 1.5s sleep is too short — on many adapters, the scan takes 3–6 seconds. If the scan hasn't completed when `netsh wlan show networks` runs, the results are stale or empty — only the connected network appears (because it's already cached).

#### Current Code

**`wifi.rs` lines 38–43:**

```rust
let _ = scan_cmd.output();
std::thread::sleep(std::time::Duration::from_millis(1500));
```

#### Solution

Increase the delay to 4 seconds:

```rust
let _ = scan_cmd.output();
// WiFi scan is async — most adapters need 3-6 seconds to complete.
// 4s is a pragmatic delay; the proper fix (WlanAPI with
// WlanRegisterNotification) is tracked in Sprint 37.
std::thread::sleep(std::time::Duration::from_millis(4000));
```

**Note:** This is a band-aid fix. The definitive fix (replacing `netsh` with WlanAPI) is tracked in Sprint 37 (S37-002). The locale-dependent parsing issue (Bug 6A) is also addressed in Sprint 37.

#### Acceptance Criteria

- [ ] WiFi scan delay increased from 1500ms to 4000ms
- [ ] Comment added explaining the delay and referencing Sprint 37 for the proper fix
- [ ] WiFi networks appear more reliably after scan (manual test)
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes

---

## Story Points

| Ticket    | Points | Owner      | Wave                                              |
| --------- | ------ | ---------- | ------------------------------------------------- |
| S35-001   | 3      | Backend    | 1 (fan.rs — independent)                          |
| S35-002   | 1      | Backend    | 1 (fan.rs — independent, but coordinate with 001) |
| S35-003   | 1      | Backend    | 1 (wmi_extract.rs — independent)                  |
| S35-004   | 2      | Full-stack | 2 (fan.rs + frontend — depends on 001)            |
| S35-005   | 1      | Frontend   | 1 (AudioControl.tsx — independent)                |
| S35-006   | 1      | Frontend   | 1 (TrayPopup.tsx + globals.css — independent)     |
| S35-007   | 1      | Backend    | 1 (wifi.rs — independent)                         |
| **Total** | **10** |            |                                                   |

## Dependency Map

```
Wave 1 (all parallel — 6 independent tickets):
  S35-001: src-tauri/src/hw/fan.rs (remove fallback)
  S35-002: src-tauri/src/hw/fan.rs (remove v>0 filter)  ← coordinate with 001
  S35-003: src-tauri/src/util/wmi_extract.rs (doc comment)
  S35-005: src/components/AudioControl.tsx (dirty flag)
  S35-006: src/pages/TrayPopup.tsx + globals.css (CSS verification)
  S35-007: src-tauri/src/hw/wifi.rs (scan delay)

Wave 2 (after S35-001):
  S35-004: fan.rs struct + frontend types (depends on 001 being committed)
```

**Note:** S35-001 and S35-002 both modify `fan.rs`. They should be committed sequentially (001 first, then 002) or combined into a single commit.

## Commit Strategy

One commit per ticket:

1. `fix(s35-001): remove fabricated 50C/45C fallback in get_esif_readings`
2. `fix(s35-002): remove v>0 filter and switch to extract_u32 for temperature/power`
3. `docs(s35-003): document UI4 overflow behavior in extract_i32`
4. `fix(s35-004): update FanInfo struct to use Option<f32> for temperatures`
5. `fix(s35-005): add isAdjustingRef dirty flag to AudioControl volume slider`
6. `fix(s35-006): verify and enforce tray-window CSS class on tray popup root`
7. `fix(s35-007): increase WiFi scan delay from 1.5s to 4s`

## What Was Deferred

| Ticket                        | Reason                                        | Next Action |
| ----------------------------- | --------------------------------------------- | ----------- |
| S37-002 (WlanAPI replacement) | Full replacement of netsh with native WlanAPI | Sprint 37   |
| S37-001 (WiFi locale parsing) | Will be fixed by WlanAPI replacement          | Sprint 37   |

---

## Sprint Completion Checklist

After all tickets are committed:

- [ ] All 7 tickets have passing health checks (9/9)
- [ ] All commits pushed to `main`
- [ ] `sprint-overview.md` updated with Sprint 35 status
- [ ] Manual test: Temperature shows real value or "—" (never 50°C when sensor is unavailable)
- [ ] Manual test: Volume slider doesn't jump during drag
- [ ] Manual test: Tray popup renders correctly (no invisible elements)
- [ ] Manual test: WiFi scan shows more networks (not just connected one)
