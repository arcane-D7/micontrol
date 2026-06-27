# Sprint 30 — P0 CRITICAL: Audit Regression & UX Blockers

> **Date:** 2026-06-27
> **Sprint:** 30
> **Theme:** Fix 4 critical issues from `Audit_Final.md` — 2 regressions + 2 UX blockers
> **Duration:** ~2 days
> **Dependencies:** Sprint 29 (commit `bec0d42`)
> **Status:** 📌 Active
> **Audit Reference:** `C:\Users\mafsc\Documents\Audit_Final.md` (C-1 through C-4)

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **OBRIGATÓRIO: 100% dos tickets desta sprint devem ser concluídos. A sprint não será aceita como entregue se qualquer ticket permanecer incompleto.**
>
> **MANDATORY: 100% of the tickets in this sprint MUST be completed. The sprint will NOT be accepted as delivered if any ticket remains incomplete.**

Every ticket must pass its acceptance criteria AND the full health check suite before the sprint commit is made.

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

The audit (`Audit_Final.md`) identified **2 regressions** introduced during the refactoring from commit `0bb3041` to `HEAD`, plus **2 pre-existing UX blockers** that make core features unusable. These 4 issues are the highest priority — they directly cause the most visible user-facing bugs:

1. **C-1 (REGRESSION):** Performance mode and charging threshold are never read from the backend after the refactoring removed these calls from the polling loop. The UI shows stale defaults (`'balance'` and `80`) that reset on every page refresh.
2. **C-2 (REGRESSION):** `check_sentry_consent()` fails to parse the plain string `"granted"` format, preventing Sentry crash reporting from ever initializing.
3. **C-3 (RACE CONDITION):** The telemetry consent dialog reappears on every F5 refresh because the IPC `invoke` fails during the React/Tauri bridge reconnection window, and the `catch` swallows the error as `null`.
4. **C-4 (UX BLOCKER):** The sidebar has no vertical scroll, making Settings, About, and other tabs unreachable when the window is too short to display all 18 nav items.

---

## Goals

| #   | Goal                                                       | KPI                                                           | Audit Reference |
| --- | ---------------------------------------------------------- | ------------------------------------------------------------- | --------------- |
| 1   | Performance mode persists across page refreshes            | Mode shown matches backend registry after F5                  | C-1             |
| 2   | Charging threshold persists across page refreshes          | Threshold shown matches backend value after F5                | C-1             |
| 3   | Sentry crash reporting initializes when consent is granted | `check_sentry_consent()` returns `true` for plain `"granted"` | C-2             |
| 4   | Consent dialog does not reappear on F5                     | No consent dialog after F5 when consent already granted       | C-3             |
| 5   | All nav tabs are accessible regardless of window height    | Settings and About reachable at 600px window height           | C-4             |

---

## Technical Specs

### S30-001: Restore `get_performance_mode` and `get_charging_threshold` polling (C-1 REGRESSION)

| Field         | Value                                                                       |
| ------------- | --------------------------------------------------------------------------- |
| **Ticket ID** | S30-001                                                                     |
| **Title**     | Restore performance mode and charging threshold polling in `useHardware.ts` |
| **Priority**  | P0 — Critical                                                               |
| **Source**    | C-1 (Audit_Final.md)                                                        |
| **Files**     | `src/hooks/useHardware.ts`                                                  |
| **Effort**    | ~1 hour                                                                     |
| **Type**      | Frontend (TypeScript)                                                       |

#### Problem

In commit `0bb3041`, the `refresh()` function called `get_performance_mode` and `get_charging_threshold` as part of a `Promise.allSettled` every 2 seconds. During the refactoring, these calls were removed from `doInitialLoad`, `fastPoll`, and `slowPoll`. The state variables `performanceMode` (initialized to `'balance'`) and `chargingThreshold` (initialized to `80`) are **never updated from the backend**.

The backend command `get_hardware_state_batch` exists in `system.rs:361` but is never invoked by the frontend (confirmed: only reference is a stale comment at `useHardware.ts:668`).

**Symptoms reported by user:**

- "Se eu atualizar o perfil de performance aparece como aplicado, mas então se eu atualizar a pagina volta para o perfil Balanced"
- "O Charging control não respeita a minha seleção"

#### Current Code (BROKEN)

```typescript
// src/hooks/useHardware.ts — slowPoll (lines ~117-135)
const slowPoll = useCallback(async () => {
  try {
    const [batteryResult, displayResult, touchpadResult] = await Promise.all([
      invoke<BatteryInfo>('get_battery_info'),
      invoke<DisplayInfo>('get_display_info'),
      invoke<TouchpadInfo>('get_touchpad_info'),
    ]);
    // ... sets battery, display, touchpad ...
    // ← MISSING: get_performance_mode, get_charging_threshold
  } catch (e) {
    /* ... */
  }
}, []);

// doInitialLoad (lines ~155-180)
const [fanResult, systemResult, batteryResult, displayResult, touchpadResult] = await Promise.all([
  invoke<FanInfo>('get_fan_info'),
  invoke<SystemInfo>('get_system_info'),
  invoke<BatteryInfo>('get_battery_info'),
  invoke<DisplayInfo>('get_display_info'),
  invoke<TouchpadInfo>('get_touchpad_info'),
]);
// ← MISSING: get_performance_mode, get_charging_threshold
```

#### Solution

**Step 1:** Add `get_performance_mode` and `get_charging_threshold` to `slowPoll`:

```typescript
const slowPoll = useCallback(async () => {
  try {
    const [batteryResult, displayResult, touchpadResult, perfMode, chargeThreshold] =
      await Promise.all([
        invoke<BatteryInfo>('get_battery_info'),
        invoke<DisplayInfo>('get_display_info'),
        invoke<TouchpadInfo>('get_touchpad_info'),
        invoke<PerformanceMode>('get_performance_mode'),
        invoke<number>('get_charging_threshold'),
      ]);
    if (batteryResult !== null) setBattery(batteryResult);
    if (displayResult !== null) setDisplay(displayResult);
    if (touchpadResult !== null && Date.now() >= touchpadDirtyUntil.current) {
      setTouchpad(touchpadResult);
    }
    if (perfMode) setPerformanceModeState(perfMode);
    setChargingThresholdState(chargeThreshold);
    setError(null);
  } catch (e) {
    console.error('Slow poll failed:', e);
    setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
  }
}, []);
```

**Step 2:** Add the same calls to `doInitialLoad`:

```typescript
const [
  fanResult,
  systemResult,
  batteryResult,
  displayResult,
  touchpadResult,
  perfMode,
  chargeThreshold,
] = await Promise.all([
  invoke<FanInfo>('get_fan_info'),
  invoke<SystemInfo>('get_system_info'),
  invoke<BatteryInfo>('get_battery_info'),
  invoke<DisplayInfo>('get_display_info'),
  invoke<TouchpadInfo>('get_touchpad_info'),
  invoke<PerformanceMode>('get_performance_mode'),
  invoke<number>('get_charging_threshold'),
]);
setFan(fanResult);
setSystemInfo(systemResult);
if (batteryResult !== null) setBattery(batteryResult);
if (displayResult !== null) setDisplay(displayResult);
if (touchpadResult !== null) setTouchpad(touchpadResult);
if (perfMode) setPerformanceModeState(perfMode);
setChargingThresholdState(chargeThreshold);
```

#### Acceptance Criteria

- [ ] `slowPoll` includes `get_performance_mode` and `get_charging_threshold` in its `Promise.all`
- [ ] `doInitialLoad` includes `get_performance_mode` and `get_charging_threshold` in its `Promise.all`
- [ ] After F5 refresh, the performance mode shown in the UI matches the backend registry value
- [ ] After F5 refresh, the charging threshold shown in the UI matches the backend value
- [ ] `npx tsc --noEmit` passes with no errors
- [ ] `npm run build` succeeds

---

### S30-002: Fix `check_sentry_consent()` to accept plain string format (C-2 REGRESSION)

| Field         | Value                                                                  |
| ------------- | ---------------------------------------------------------------------- |
| **Ticket ID** | S30-002                                                                |
| **Title**     | Fix `check_sentry_consent()` to accept plain string `"granted"` format |
| **Priority**  | P0 — Critical                                                          |
| **Source**    | C-2 (Audit_Final.md)                                                   |
| **Files**     | `src-tauri/src/util/consent_audit.rs`                                  |
| **Effort**    | ~30 minutes                                                            |
| **Type**      | Backend (Rust)                                                         |

#### Problem

The function `check_sentry_consent()` at `consent_audit.rs:195-213` tries to `serde_json::from_str` the keyring value. When the value is the plain string `"granted"` (which is what `setTelemetryConsent('granted')` stores), the JSON parse fails because `granted` without quotes is not valid JSON. The function returns `false`, preventing Sentry from ever initializing.

The companion function `get_telemetry_consent()` in `ai.rs:295-310` was already fixed to handle both formats, but `check_sentry_consent()` was not updated.

#### Current Code (BROKEN)

```rust
// src-tauri/src/util/consent_audit.rs:195-213
pub fn check_sentry_consent() -> bool {
    let entry = match Entry::new(KEYRING_SERVICE, TELEMETRY_CONSENT_KEY) {
        Ok(e) => e,
        Err(_) => return false,
    };
    match entry.get_password() {
        Ok(val) => {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&val) {
                parsed["value"].as_str() == Some("granted")
            } else {
                false  // ← plain "granted" falls here
            }
        }
        Err(_) => false,
    }
}
```

#### Solution

Replace the `match entry.get_password()` block to check for plain string format first, then fall back to JSON:

```rust
pub fn check_sentry_consent() -> bool {
    let entry = match Entry::new(KEYRING_SERVICE, TELEMETRY_CONSENT_KEY) {
        Ok(e) => e,
        Err(_) => return false,
    };
    match entry.get_password() {
        Ok(val) => {
            // Handle plain string format (current — stored by setTelemetryConsent)
            if val == "granted" {
                return true;
            }
            if val == "denied" {
                return false;
            }
            // Handle legacy JSON format (backwards compatibility)
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&val) {
                return parsed["value"].as_str() == Some("granted");
            }
            false
        }
        Err(_) => false,
    }
}
```

#### Acceptance Criteria

- [ ] `check_sentry_consent()` returns `true` when keyring contains plain string `"granted"`
- [ ] `check_sentry_consent()` returns `false` when keyring contains plain string `"denied"`
- [ ] `check_sentry_consent()` returns `true` when keyring contains legacy JSON `{"value":"granted"}`
- [ ] `check_sentry_consent()` returns `false` when keyring is empty or key doesn't exist
- [ ] Unit test added: `test_check_sentry_consent_plain_string`
- [ ] Unit test added: `test_check_sentry_consent_legacy_json`
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes

---

### S30-003: Fix consent dialog race condition on F5 refresh (C-3)

| Field         | Value                                                                         |
| ------------- | ----------------------------------------------------------------------------- |
| **Ticket ID** | S30-003                                                                       |
| **Title**     | Add retry logic to `getTelemetryConsent()` and memoize `useSettings()` return |
| **Priority**  | P0 — Critical                                                                 |
| **Source**    | C-3 (Audit_Final.md)                                                          |
| **Files**     | `src/hooks/useTelemetryConsent.ts`, `src/hooks/useSettings.ts`                |
| **Effort**    | ~1.5 hours                                                                    |
| **Type**      | Frontend (TypeScript)                                                         |

#### Problem

The `getTelemetryConsent()` function in `useTelemetryConsent.ts:31-33` has a `catch` that returns `null` on any IPC error. On F5 (hard reload), React's `useEffect` fires before the Tauri IPC bridge is fully re-established. If `invoke('get_secret')` fails during this window, the catch returns `null` → `consent === null` → consent dialog appears.

Additionally, `useSettings()` returns a new object on every render (no `useMemo`), causing unnecessary `useEffect` re-executions.

#### Current Code (BROKEN)

```typescript
// src/hooks/useTelemetryConsent.ts:20-35
async function getTelemetryConsent(): Promise<TelemetryConsentValue> {
  try {
    const result = await invoke<string | null>('get_secret', { key: TELEMETRY_CONSENT_KEY });
    if (!result) return null;
    if (result === 'granted') return 'granted';
    if (result === 'denied') return 'denied';
    try {
      const parsed = JSON.parse(result);
      if (parsed?.value === 'granted') return 'granted';
      if (parsed?.value === 'denied') return 'denied';
    } catch {}
    return null;
  } catch {
    return null; // ← swallows IPC failure as "no consent"
  }
}
```

```typescript
// src/hooks/useSettings.ts:178-190 — returns new object every render
return {
  settings,
  saveSettings,
  updateKey,
  setOnboardingCompleted,
  analyzeSystem: ai.analyzeSystem,
  // ... no useMemo
};
```

#### Solution

**Step 1:** Add retry with 500ms delay in `getTelemetryConsent()`:

```typescript
async function getTelemetryConsent(): Promise<TelemetryConsentValue> {
  const tryFetch = async (): Promise<TelemetryConsentValue> => {
    try {
      const result = await invoke<string | null>('get_secret', { key: TELEMETRY_CONSENT_KEY });
      if (!result) return null;
      if (result === 'granted') return 'granted';
      if (result === 'denied') return 'denied';
      try {
        const parsed = JSON.parse(result);
        if (parsed?.value === 'granted') return 'granted';
        if (parsed?.value === 'denied') return 'denied';
      } catch {
        // Not JSON — treat as unknown
      }
      return null;
    } catch {
      return null;
    }
  };

  // First attempt
  let result = await tryFetch();
  // If null (could be race condition on F5), retry after 500ms
  if (result === null) {
    await new Promise((resolve) => setTimeout(resolve, 500));
    result = await tryFetch();
  }
  return result;
}
```

**Step 2:** Memoize `useSettings()` return value:

```typescript
// src/hooks/useSettings.ts — add useMemo import
import { useState, useEffect, useMemo } from 'react';

// Replace the return statement at the end of useSettings():
return useMemo(
  () => ({
    settings,
    saveSettings,
    updateKey,
    setOnboardingCompleted,
    analyzeSystem: ai.analyzeSystem,
    analyzeWithLogs: ai.analyzeWithLogs,
    testConnection: ai.testConnection,
    isConfigured: Boolean(settings.openai_api_key.trim()),
    getTelemetryConsent: telemetry.getTelemetryConsent,
    setTelemetryConsent: telemetry.setTelemetryConsent,
    revokeTelemetryConsent: telemetry.revokeTelemetryConsent,
    checkTelemetryConsent: telemetry.checkTelemetryConsent,
  }),
  [settings, ai, telemetry],
);
```

#### Acceptance Criteria

- [ ] `getTelemetryConsent()` retries once after 500ms if the first attempt returns `null`
- [ ] `useSettings()` return value is wrapped in `useMemo` with correct dependencies
- [ ] After F5 refresh with consent already granted, the consent dialog does NOT appear
- [ ] After F5 refresh with consent denied, the consent dialog does NOT appear
- [ ] After F5 refresh with no consent set, the consent dialog DOES appear (correct behavior)
- [ ] `npx tsc --noEmit` passes
- [ ] `npm run lint` passes
- [ ] `npm run build` succeeds

---

### S30-004: Add vertical scroll to sidebar (C-4)

| Field         | Value                                    |
| ------------- | ---------------------------------------- |
| **Ticket ID** | S30-004                                  |
| **Title**     | Add `overflow-y: auto` to `.sidebar` CSS |
| **Priority**  | P0 — Critical                            |
| **Source**    | C-4 (Audit_Final.md)                     |
| **Files**     | `src/styles/globals.css`                 |
| **Effort**    | ~15 minutes                              |
| **Type**      | Frontend (CSS)                           |

#### Problem

The `.app-layout` has `overflow: hidden` and `.sidebar` has no `overflow-y` property. With 18 nav items + footer, the content overflows the viewport in shorter windows. Settings, About, and other tabs at the bottom of the sidebar are unreachable.

#### Current Code (BROKEN)

```css
/* src/styles/globals.css:337-360 */
.app-layout {
  display: flex;
  height: 100vh;
  overflow: hidden; /* ← cuts everything that exceeds */
}

.sidebar {
  width: 192px;
  min-width: 192px;
  display: flex;
  flex-direction: column;
  padding: 14px 10px 12px;
  gap: 2px;
  background: var(--surface);
  backdrop-filter: var(--blur);
  -webkit-backdrop-filter: var(--blur);
  border-right: 1px solid var(--border);
  box-shadow: var(--shadow-glass);
  transition:
    background var(--t-slow) var(--ease),
    border-color var(--t-slow) var(--ease);
  /* ← NO overflow-y: auto */
}
```

#### Solution

Add `overflow-y: auto`, `overflow-x: hidden`, and `scrollbar-width: thin` to `.sidebar`:

```css
.sidebar {
  width: 192px;
  min-width: 192px;
  display: flex;
  flex-direction: column;
  padding: 14px 10px 12px;
  gap: 2px;
  background: var(--surface);
  backdrop-filter: var(--blur);
  -webkit-backdrop-filter: var(--blur);
  border-right: 1px solid var(--border);
  box-shadow: var(--shadow-glass);
  overflow-y: auto;
  overflow-x: hidden;
  scrollbar-width: thin;
  transition:
    background var(--t-slow) var(--ease),
    border-color var(--t-slow) var(--ease);
}
```

#### Acceptance Criteria

- [ ] `.sidebar` has `overflow-y: auto`
- [ ] `.sidebar` has `overflow-x: hidden`
- [ ] `.sidebar` has `scrollbar-width: thin` (Firefox)
- [ ] At 600px window height, all 18 nav items are scrollable and accessible
- [ ] Settings and About tabs are reachable by scrolling
- [ ] Horizontal scrollbar does not appear
- [ ] `npm run build` succeeds

---

## Story Points

| Ticket    | Points | Owner    | Wave            |
| --------- | ------ | -------- | --------------- |
| S30-001   | 2      | Frontend | 1 (independent) |
| S30-002   | 1      | Backend  | 1 (independent) |
| S30-003   | 3      | Frontend | 1 (independent) |
| S30-004   | 1      | Frontend | 1 (independent) |
| **Total** | **7**  |          |                 |

## Dependency Map

```
Wave 1 (all parallel — independent files):
  S30-001: src/hooks/useHardware.ts
  S30-002: src-tauri/src/util/consent_audit.rs
  S30-003: src/hooks/useTelemetryConsent.ts + src/hooks/useSettings.ts
  S30-004: src/styles/globals.css
```

All 4 tickets modify different files and have no logical dependencies. They can all be executed in parallel.

## Commit Strategy

One commit per ticket:

1. `fix(s30-001): restore performance mode and charging threshold polling`
2. `fix(s30-002): accept plain string format in check_sentry_consent`
3. `fix(s30-003): add retry to consent fetch and memoize useSettings`
4. `fix(s30-004): add vertical scroll to sidebar for nav accessibility`

## What Was Deferred

| Ticket | Reason | Next Action |
| ------ | ------ | ----------- |
| —      | —      | —           |

No items deferred. All 4 critical issues must be resolved in this sprint.
