# Sprint 34 — P0 CRITICAL: Auth Bridge UAC Regression & Quick Wins

> **Date:** 2026-07-19
> **Sprint:** 34
> **Theme:** Fix Auth Bridge UAC regression (4 root causes) + 3 quick-win UX fixes
> **Duration:** ~2–3 days
> **Dependencies:** Sprint 30–33 (should be completed or merged into this sprint)
> **Status:** ✅ Complete
> **Commit:** `309bfa3` — `fix(s34): fix Auth Bridge UAC regression and quick wins`
> **Audit Reference:** `C:\Users\mafsc\Documents\Audit_Report_miControl.md` (Bug 1: 1A, 1B, 1C, 1D; Bug 3: 3A, 3C; Bug 4: 4A)

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

This sprint addresses the **most critical bug** in miControl: the Auth Bridge UAC regression that prompts for administrator permission on every configuration change. The root cause is a combination of 4 defects in the elevated bridge system. Additionally, 3 quick-win fixes for tray popup rendering and volume slider sync are included to maximize user impact in a single sprint.

1. **S34-001:** XML encoding mismatch (`UTF-16` declared, UTF-8 written) — prevents scheduled task registration
2. **S34-002:** `MultipleInstancesPolicy=StopExisting` kills in-flight elevated helpers
3. **S34-003:** `ensure_task_correct_path()` tri-state return bug — treats "already correct" as "healing failed"
4. **S34-004:** Per-command timeout map — 15s global timeout too short for hardware discovery and driver install
5. **S34-005:** Remove `color-mix(in oklch, ...)` from tray popup CSS — causes invisible rendering on transparent WebView2
6. **S34-006:** Remove `is_visible()` guard from `resize_tray_popup` — race condition prevents first resize
7. **S34-007:** Re-add `get_audio_volume` to `fastPoll` — audio state never polled since S5-003/S12-007 regression

---

## Goals

| #   | Goal                                                         | KPI                                              | Audit Reference |
| --- | ------------------------------------------------------------ | ------------------------------------------------ | --------------- |
| 1   | Zero UAC prompts on config changes (when task is registered) | No `ShellExecuteExW` "runas" in production logs  | Bug 1 (1A–1D)   |
| 2   | Scheduled task registers successfully on fresh install       | `schtasks /query /tn MiControlElevated` succeeds | Bug 1 (1B)      |
| 3   | Concurrent elevated calls don't kill each other              | `IgnoreNew` policy in task XML                   | Bug 1 (1C)      |
| 4   | Slow commands don't timeout                                  | 90s timeout for `run_hardware_discovery`         | Bug 1 (1D)      |
| 5   | Tray popup renders correctly on all GPUs                     | No `color-mix` in tray CSS                       | Bug 3 (3A)      |
| 6   | Tray popup resizes on first show                             | `resize_tray_popup` works even when hidden       | Bug 3 (3C)      |
| 7   | Volume slider syncs with system volume                       | `audioState` polled every 2s                     | Bug 4 (4A)      |

---

## Technical Specs

### S34-001: Fix XML encoding mismatch in scheduled task registration (Bug 1B)

| Field         | Value                                                                                             |
| ------------- | ------------------------------------------------------------------------------------------------- |
| **Ticket ID** | S34-001                                                                                           |
| **Title**     | Change `encoding="UTF-16"` to `encoding="UTF-8"` in task XML (NSIS installer + self-healing code) |
| **Priority**  | P0 — Critical                                                                                     |
| **Source**    | Bug 1B (`Audit_Report_miControl.md`)                                                              |
| **Files**     | `src-tauri/nsis/installer-hooks.nsi` (line 103), `src-tauri/src/elev_bridge.rs` (lines 521–524)   |
| **Effort**    | ~15 minutes                                                                                       |
| **Type**      | Backend (Rust + NSIS)                                                                             |

#### Problem

Both the NSIS installer and the self-healing code in `elev_bridge.rs` declare `encoding="UTF-16"` in the task XML but write the bytes as UTF-8. NSIS `WriteFile` writes raw bytes (does not transcode). Rust `std::fs::write` writes `&str` as UTF-8. MSXML (used by `schtasks /create /xml`) honors the `encoding=` declaration — when it says UTF-16 but the byte stream is UTF-8 with no BOM, the parse fails with `SCHED_E_MALFORMEDXML`.

**Result:** The scheduled task is never registered at install time → self-healing attempts → if it fails (non-admin), every subsequent `run_elevated()` call falls back to UAC.

#### Current Code

**`installer-hooks.nsi` line 103:**

```nsis
WriteFile "$TEMP\MCElev.xml" '<?xml version="1.0" encoding="UTF-16"?><Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">...'
```

**`elev_bridge.rs` lines 521–524:**

```rust
let xml = format!(
    r#"<?xml version="1.0" encoding="UTF-16"?><Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">..."#
);
std::fs::write(&xml_path, &xml)
```

#### Solution

Change `encoding="UTF-16"` to `encoding="UTF-8"` in both locations. The XML contains only ASCII characters, so UTF-8 is correct and matches the actual byte stream.

**`installer-hooks.nsi` line 103:**

```nsis
WriteFile "$TEMP\MCElev.xml" '<?xml version="1.0" encoding="UTF-8"?><Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">...'
```

**`elev_bridge.rs` lines 521–524:**

```rust
let xml = format!(
    r#"<?xml version="1.0" encoding="UTF-8"?><Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">..."#
);
```

#### Acceptance Criteria

- [ ] Both `installer-hooks.nsi` and `elev_bridge.rs` declare `encoding="UTF-8"`
- [ ] `schtasks /create /xml <file>` succeeds on Windows 10 22H2 and Windows 11 24H2
- [ ] After fresh install, `schtasks /query /tn MiControlElevated /xml` returns valid XML
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes

---

### S34-002: Change `MultipleInstancesPolicy` from `StopExisting` to `IgnoreNew` (Bug 1C)

| Field         | Value                                                                                      |
| ------------- | ------------------------------------------------------------------------------------------ |
| **Ticket ID** | S34-002                                                                                    |
| **Title**     | Replace `StopExisting` with `IgnoreNew` in task XML (NSIS installer + self-healing code)   |
| **Priority**  | P0 — Critical                                                                              |
| **Source**    | Bug 1C (`Audit_Report_miControl.md`)                                                       |
| **Files**     | `src-tauri/nsis/installer-hooks.nsi` (line 103), `src-tauri/src/elev_bridge.rs` (line 525) |
| **Effort**    | ~10 minutes                                                                                |
| **Type**      | Backend (Rust + NSIS)                                                                      |

#### Problem

`StopExisting` terminates the running instance when a new instance starts. If the first elevated helper is mid-way through writing `elev_result_<id>.json`, it gets killed, the result file is never written, and the first caller times out → UAC fallback.

The `ELEV_REQUEST_LOCK` in `elev_bridge.rs:31` only serializes calls within one process. The tray popup and main window are separate processes — both can call `run_elevated` concurrently.

#### Current Code

**`installer-hooks.nsi` line 103 (within the XML string):**

```xml
<MultipleInstancesPolicy>StopExisting</MultipleInstancesPolicy>
```

**`elev_bridge.rs` line 525 (within the format! string):**

```rust
<MultipleInstancesPolicy>StopExisting</MultipleInstancesPolicy>
```

#### Solution

Replace `StopExisting` with `IgnoreNew` in both locations.

**Why `IgnoreNew` and not `Queue`:**

- `IgnoreNew`: A new trigger while the task is running is dropped. The in-flight helper completes and writes its result. The dropped caller's `run_schtasks_run()` returns success but no new instance starts — the dropped caller times out and falls back to UAC. This is the correct trade-off: the in-flight call succeeds, only the concurrent caller pays the UAC cost.
- `Queue`: New instances queue and run sequentially. The scheduler's queue is unbounded and the queued instance will run later with no `--request-id`, potentially picking up a stale command file. Combined with the timeout, `Queue` creates a thundering herd of queued helpers. Avoid `Queue`.

**`installer-hooks.nsi`:**

```xml
<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
```

**`elev_bridge.rs`:**

```rust
<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
```

#### Acceptance Criteria

- [ ] Both `installer-hooks.nsi` and `elev_bridge.rs` use `IgnoreNew`
- [ ] Concurrent `run_elevated` calls from tray + main window don't kill each other
- [ ] First caller succeeds without UAC; second caller either succeeds or falls back to UAC cleanly
- [ ] `cargo check` passes

---

### S34-003: Fix tri-state return bug in `ensure_task_correct_path()` (Bug 1A)

| Field         | Value                                                                               |
| ------------- | ----------------------------------------------------------------------------------- |
| **Ticket ID** | S34-003                                                                             |
| **Title**     | Replace `bool` return with `enum TaskHealResult { AlreadyCorrect, Healed, Failed }` |
| **Priority**  | P0 — Critical                                                                       |
| **Source**    | Bug 1A (`Audit_Report_miControl.md`)                                                |
| **Files**     | `src-tauri/src/elev_bridge.rs` (lines 460–540 function, lines 115–131 caller)       |
| **Effort**    | ~1–2 hours                                                                          |
| **Type**      | Backend (Rust)                                                                      |

#### Problem

`ensure_task_correct_path()` returns `bool` but has three terminal states:

1. **Already correct** (line 483): `return false;` — the task already points to the current exe
2. **Healed** (line 537): `return true;` — the task was re-registered successfully
3. **Failed** (line 539): `return false;` — healing failed (not admin or UAC declined)

The caller (lines 115–131) treats `false` as "healing failed" → UAC fallback. When the task is already correct (case 1, the common case), the caller falls back to UAC, prompting the user for administrator permission.

#### Current Code

**`elev_bridge.rs` lines 460–540 (function):**

```rust
fn ensure_task_correct_path() -> bool {
    // ...
    if !need_reregister {
        return false;  // ← AMBIGUOUS: "already correct" treated as "failed"
    }
    // ... healing logic ...
    success  // true = healed, false = failed
}
```

**`elev_bridge.rs` lines 115–131 (caller):**

```rust
let healed = tokio::task::spawn_blocking(ensure_task_correct_path).await...?;
if healed {
    let retry_ok = run_schtasks_run().await;
    if retry_ok { /* poll */ } else { launch_uac_fallback(...).await?; }
} else {
    // Healing failed — fall back to UAC
    launch_uac_fallback(&request_id, &cmd_path).await?;
}
```

#### Solution

**Step 1:** Define the enum near the top of `elev_bridge.rs` (after the constants block, ~line 30):

```rust
/// Outcome of a task-path self-healing attempt.
enum TaskHealResult {
    /// Task already exists and points to the current exe — no action needed.
    AlreadyCorrect,
    /// Task was missing or mis-pointed and has been re-registered successfully.
    Healed,
    /// Healing was attempted but failed (not admin, UAC declined, etc.).
    Failed,
}
```

**Step 2:** Change `ensure_task_correct_path()` to return `TaskHealResult`:

```rust
#[cfg(windows)]
fn ensure_task_correct_path() -> TaskHealResult {
    // ... existing setup ...
    if !need_reregister {
        return TaskHealResult::AlreadyCorrect;  // was: return false;
    }
    // ... existing healing logic ...
    if success {
        log::info!("Scheduled task re-registered successfully with correct path");
        TaskHealResult::Healed  // was: true
    } else {
        log::warn!("Failed to re-register scheduled task (UAC may have been declined)");
        TaskHealResult::Failed  // was: false
    }
}

#[cfg(not(windows))]
fn ensure_task_correct_path() -> TaskHealResult {
    TaskHealResult::Failed
}
```

**Step 3:** Update the caller (lines 115–131):

```rust
let healed = tokio::task::spawn_blocking(ensure_task_correct_path)
    .await
    .map_err(|e| format!("task heal task panicked: {e}"))?;

match healed {
    TaskHealResult::AlreadyCorrect => {
        // Task was fine — the original /run failure was transient.
        // Retry once before falling back to UAC.
        let retry_ok = run_schtasks_run().await;
        if !retry_ok {
            log::warn!("Scheduled task still failing after AlreadyCorrect, falling back to UAC");
            launch_uac_fallback(&request_id, &cmd_path).await?;
        }
    }
    TaskHealResult::Healed => {
        let retry_ok = run_schtasks_run().await;
        if !retry_ok {
            log::warn!("Scheduled task still failed after self-healing, falling back to UAC");
            launch_uac_fallback(&request_id, &cmd_path).await?;
        }
    }
    TaskHealResult::Failed => {
        launch_uac_fallback(&request_id, &cmd_path).await?;
    }
}
```

#### Acceptance Criteria

- [ ] `TaskHealResult` enum defined with 3 variants
- [ ] `ensure_task_correct_path()` returns `TaskHealResult` (both `#[cfg(windows)]` and `#[cfg(not(windows))]`)
- [ ] Caller handles all 3 variants correctly
- [ ] `AlreadyCorrect` retries `run_schtasks_run()` instead of immediately falling back to UAC
- [ ] No UAC prompt when the task is already correctly registered
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes

---

### S34-004: Add per-command timeout map for elevated operations (Bug 1D)

| Field         | Value                                                                                                                  |
| ------------- | ---------------------------------------------------------------------------------------------------------------------- |
| **Ticket ID** | S34-004                                                                                                                |
| **Title**     | Replace global 15s timeout with per-command timeout map                                                                |
| **Priority**  | P0 — Critical                                                                                                          |
| **Source**    | Bug 1D (`Audit_Report_miControl.md`)                                                                                   |
| **Files**     | `src-tauri/src/elev_bridge.rs` (line 30, lines 208–209), `installer-hooks.nsi` (line 103), `elev_bridge.rs` (line 525) |
| **Effort**    | ~1 hour                                                                                                                |
| **Type**      | Backend (Rust)                                                                                                         |

#### Problem

`ELEV_TIMEOUT_SECS = 15` (line 30) is used for ALL commands. But `run_hardware_discovery` (WMI + IOCTL probes) and `install_driver` (`pnputil /add-driver`) can take 30–60s on cold start. The `ExecutionTimeLimit=PT30S` in the task XML means the task scheduler kills the helper at 30s — so the bridge timeout should be longer than that to avoid racing the scheduler's kill.

#### Current Code

**`elev_bridge.rs` line 30:**

```rust
const ELEV_TIMEOUT_SECS: u64 = 15;
```

**`elev_bridge.rs` lines 208–209:**

```rust
let timeout = Duration::from_secs(ELEV_TIMEOUT_SECS);
```

**Task XML (both `installer-hooks.nsi:103` and `elev_bridge.rs:525`):**

```xml
<ExecutionTimeLimit>PT30S</ExecutionTimeLimit>
```

#### Solution

**Step 1:** Add per-command timeout constants and function after line 30:

```rust
/// Default timeout for elevated operations (fast commands).
const ELEV_TIMEOUT_SECS: u64 = 15;
/// Timeout for slow commands that do WMI/IOCTL probes or driver installs.
const ELEV_TIMEOUT_SLOW_SECS: u64 = 90;
/// Timeout for medium commands (WMI queries on cold start).
const ELEV_TIMEOUT_MEDIUM_SECS: u64 = 45;

/// Returns the timeout for a given elevated command.
///
/// Slow commands (hardware discovery, driver install) do WMI + IOCTL probes
/// or run `pnputil`, which can take 30–60 s on a cold system. The task
/// scheduler's `ExecutionTimeLimit` is PT120S, so the bridge timeout must be
/// shorter than that to avoid waiting for a killed helper.
fn timeout_for_cmd(cmd: &str) -> Duration {
    match cmd {
        "run_hardware_discovery" | "install_driver" => {
            Duration::from_secs(ELEV_TIMEOUT_SLOW_SECS)
        }
        "wmi_ec_read_sensor_data"
        | "wmi_ec_read_battery_health"
        | "wmi_ec_read_adapter_power"
        | "wmi_ec_get_performance_mode"
        | "diag_wmi_query" => Duration::from_secs(ELEV_TIMEOUT_MEDIUM_SECS),
        _ => Duration::from_secs(ELEV_TIMEOUT_SECS),
    }
}
```

**Step 2:** Update the poll loop at line 208–209:

```rust
// Poll for the result file (check every 150 ms, timeout per-command)
let timeout = timeout_for_cmd(cmd);
let start = Instant::now();
```

**Step 3:** Update `ExecutionTimeLimit` from `PT30S` to `PT120S` in both `installer-hooks.nsi:103` and `elev_bridge.rs:525`:

```xml
<ExecutionTimeLimit>PT120S</ExecutionTimeLimit>
```

**Step 4:** Update error messages at lines 243 and 289 to use `timeout.as_secs()` instead of hardcoded "15 s":

```rust
return Err(format!(
    "Elevated process timed out after {} s and UAC fallback failed: {}. Reinstall MiControl to fix the scheduled task.",
    timeout.as_secs(), e
));
```

#### Acceptance Criteria

- [ ] `timeout_for_cmd()` function defined with 3 timeout tiers
- [ ] `run_hardware_discovery` and `install_driver` use 90s timeout
- [ ] WMI queries use 45s timeout
- [ ] Default commands use 15s timeout
- [ ] `ExecutionTimeLimit` changed to `PT120S` in both NSIS and self-healing XML
- [ ] Error messages use dynamic timeout value
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes

---

### S34-005: Remove `color-mix(in oklch, ...)` from tray popup CSS (Bug 3A)

| Field         | Value                                                                            |
| ------------- | -------------------------------------------------------------------------------- |
| **Ticket ID** | S34-005                                                                          |
| **Title**     | Replace `color-mix` gradient with solid `var(--surface-2)` in `.tray-quick-card` |
| **Priority**  | P0 — Critical                                                                    |
| **Source**    | Bug 3A (`Audit_Report_miControl.md`)                                             |
| **Files**     | `src/styles/globals.css` (lines 1088–1098)                                       |
| **Effort**    | ~10 minutes                                                                      |
| **Type**      | Frontend (CSS)                                                                   |

#### Problem

The `.tray-quick-card` CSS uses `color-mix(in oklch, var(--surface-2) 72%, var(--accent-soft))` in a linear-gradient background. The tray window is created with `.transparent(true)`. On GPUs with Intel Arc and certain Nvidia/AMD drivers, `color-mix(in oklch, ...)` inside a transparent WebView2 window causes elements to render as completely invisible or corrupted.

The `.tray-window` override (lines 290–306) sets `--blur: none` and `--surface: var(--surface-solid)`, but does NOT override `color-mix`.

#### Current Code

**`globals.css` lines 1088–1098:**

```css
.tray-quick-card {
  padding: 10px;
  border-radius: var(--r-sm);
  border: 1px solid var(--border);
  background: linear-gradient(
    130deg,
    var(--surface-2),
    color-mix(in oklch, var(--surface-2) 72%, var(--accent-soft))
  );
}
```

#### Solution

Replace the `color-mix` gradient with a solid background:

```css
.tray-quick-card {
  padding: 10px;
  border-radius: var(--r-sm);
  border: 1px solid var(--border);
  background: var(--surface-2);
}
```

#### Acceptance Criteria

- [ ] No `color-mix` in `.tray-quick-card` CSS rule
- [ ] Tray quick cards render visibly on all GPU configurations
- [ ] `npm run build` succeeds
- [ ] Visual verification: tray popup cards are visible (not invisible/transparent)

---

### S34-006: Remove `is_visible()` guard from `resize_tray_popup` (Bug 3C)

| Field         | Value                                                                 |
| ------------- | --------------------------------------------------------------------- |
| **Ticket ID** | S34-006                                                               |
| **Title**     | Remove early return when window is not visible in `resize_tray_popup` |
| **Priority**  | P0 — Critical                                                         |
| **Source**    | Bug 3C (`Audit_Report_miControl.md`)                                  |
| **Files**     | `src-tauri/src/lib.rs` (lines 581–617)                                |
| **Effort**    | ~10 minutes                                                           |
| **Type**      | Backend (Rust)                                                        |

#### Problem

`resize_tray_popup` checks `window.is_visible()` and returns early if false. The `ResizeObserver` in `TrayPopup.tsx` fires `resize_tray_popup` immediately on mount. If the window hasn't been shown yet (race condition with `popup.show()`), the resize is ignored and the window appears with the default 300×460 size instead of the content size.

#### Current Code

**`lib.rs` lines 581–617:**

```rust
async fn resize_tray_popup(app: tauri::AppHandle, height: f64) -> Result<(), String> {
    const MIN_H: f64 = 200.0;
    const MAX_H: f64 = 780.0;
    let height = height.clamp(MIN_H, MAX_H);
    if let Some(window) = app.get_webview_window("tray") {
        if !window.is_visible().unwrap_or(false) {
            return Ok(());  // ← BUG: skips resize on first show
        }
        // ... resize logic ...
    }
    Ok(())
}
```

#### Solution

Remove the `is_visible()` guard. The resize should always be applied when the window exists, regardless of visibility:

```rust
async fn resize_tray_popup(app: tauri::AppHandle, height: f64) -> Result<(), String> {
    const MIN_H: f64 = 200.0;
    const MAX_H: f64 = 780.0;
    let height = height.clamp(MIN_H, MAX_H);
    if let Some(window) = app.get_webview_window("tray") {
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
        let pos = window.outer_position().map_err(|e| e.to_string())?;
        let cur = window.inner_size().map_err(|e| e.to_string())?;
        let bottom_phys = pos.y + cur.height as i32;
        let new_h_phys = (height * scale).round() as u32;
        let new_y = (bottom_phys - new_h_phys as i32).max(0);
        window
            .set_size(tauri::PhysicalSize::new(cur.width, new_h_phys))
            .map_err(|e| e.to_string())?;
        window
            .set_position(tauri::PhysicalPosition::new(pos.x, new_y))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
```

#### Acceptance Criteria

- [ ] `is_visible()` guard removed from `resize_tray_popup`
- [ ] Tray popup resizes correctly on first show
- [ ] No visual glitch (window appears at correct size immediately)
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes

---

### S34-007: Re-add `get_audio_volume` to `fastPoll` (Bug 4A)

| Field         | Value                                                                   |
| ------------- | ----------------------------------------------------------------------- |
| **Ticket ID** | S34-007                                                                 |
| **Title**     | Add `invoke<AudioVolumeResult>('get_audio_volume')` to the 2s fast poll |
| **Priority**  | P0 — Critical                                                           |
| **Source**    | Bug 4A (`Audit_Report_miControl.md`)                                    |
| **Files**     | `src/hooks/useHardware.ts` (lines 106–120)                              |
| **Effort**    | ~15 minutes                                                             |
| **Type**      | Frontend (TypeScript)                                                   |

#### Problem

`audioState` is never polled. The `fastPoll` (2s) fetches `get_fan_info` + `get_system_info` only. The `slowPoll` (15s) fetches battery/display/touchpad/perf/charge. The `getAudioState` function (lines 711–719) exists but is never called in any `useEffect` or `setInterval`.

The comment on line 708 says: "Audio state is now polled as part of the batched get_hardware_state_batch." But `get_hardware_state_batch` is dead code — never invoked from the frontend.

**Consequence:** `setMasterVolume` does an optimistic update, but if an external app (Spotify, system mixer, keyboard media key) changes the volume, the slider never reflects the real state.

#### Current Code

**`useHardware.ts` lines 106–120:**

```typescript
const fastPoll = useCallback(async () => {
  try {
    const [fanResult, systemResult] = await Promise.all([
      invoke<FanInfo>('get_fan_info'),
      invoke<SystemInfo>('get_system_info'),
    ]);
    setFan(fanResult);
    if (systemResult) {
      setSystemInfo(systemResult);
    }
    setError(null);
  } catch (e) {
    console.error('Fast poll failed:', e);
    setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
  }
}, []);
```

#### Solution

Add `get_audio_volume` to the `Promise.all` array in `fastPoll`:

```typescript
const fastPoll = useCallback(async () => {
  try {
    const [fanResult, systemResult, audioResult] = await Promise.all([
      invoke<FanInfo>('get_fan_info'),
      invoke<SystemInfo>('get_system_info'),
      invoke<AudioVolumeResult>('get_audio_volume'),
    ]);
    setFan(fanResult);
    if (systemResult) {
      setSystemInfo(systemResult);
    }
    setAudioState(audioResult);
    setError(null);
  } catch (e) {
    console.error('Fast poll failed:', e);
    setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
  }
}, []);
```

**Note:** The `TrayPopup.tsx` (lines 70–74) already has an `isAdjustingVolume` guard that prevents overwriting `localVolume` during active dragging. The `AudioControl.tsx` slider does NOT have this guard — it should be added in Sprint 35 (S35-005) to prevent mid-drag overwrite.

#### Acceptance Criteria

- [ ] `get_audio_volume` added to `fastPoll` `Promise.all`
- [ ] `setAudioState(audioResult)` called on each poll
- [ ] Volume slider updates when system volume changes externally (within 2s)
- [ ] Mute state updates when keyboard mute key is pressed (within 2s)
- [ ] `npx tsc --noEmit` passes
- [ ] `npm run build` succeeds

---

## Story Points

| Ticket    | Points | Owner    | Wave                                    |
| --------- | ------ | -------- | --------------------------------------- |
| S34-001   | 1      | Backend  | 1 (NSIS + elev_bridge.rs — independent) |
| S34-002   | 1      | Backend  | 1 (NSIS + elev_bridge.rs — independent) |
| S34-003   | 3      | Backend  | 2 (elev_bridge.rs — depends on 001/002) |
| S34-004   | 2      | Backend  | 2 (elev_bridge.rs — depends on 001/002) |
| S34-005   | 1      | Frontend | 1 (globals.css — independent)           |
| S34-006   | 1      | Backend  | 1 (lib.rs — independent)                |
| S34-007   | 1      | Frontend | 1 (useHardware.ts — independent)        |
| **Total** | **10** |          |                                         |

## Dependency Map

```
Wave 1 (all parallel — 5 independent tickets):
  S34-001: installer-hooks.nsi + elev_bridge.rs (XML encoding)
  S34-002: installer-hooks.nsi + elev_bridge.rs (MultipleInstancesPolicy)
  S34-005: src/styles/globals.css (color-mix removal)
  S34-006: src-tauri/src/lib.rs (is_visible guard removal)
  S34-007: src/hooks/useHardware.ts (audio poll re-add)

Wave 2 (after Wave 1 — 2 tickets modifying elev_bridge.rs):
  S34-003: src-tauri/src/elev_bridge.rs (tri-state enum — depends on 001/002 being committed)
  S34-004: src-tauri/src/elev_bridge.rs (per-command timeout — depends on 001/002 being committed)
```

**Note:** S34-001 and S34-002 both modify `installer-hooks.nsi` and `elev_bridge.rs`. They should be committed sequentially (001 first, then 002) to avoid merge conflicts. S34-003 and S34-004 also modify `elev_bridge.rs` and should be committed after 001 and 002.

## Commit Strategy

One commit per ticket:

1. `fix(s34-001): fix XML encoding mismatch in scheduled task registration`
2. `fix(s34-002): change MultipleInstancesPolicy from StopExisting to IgnoreNew`
3. `fix(s34-003): fix tri-state return bug in ensure_task_correct_path`
4. `fix(s34-004): add per-command timeout map for elevated operations`
5. `fix(s34-005): remove color-mix from tray popup CSS to fix invisible rendering`
6. `fix(s34-006): remove is_visible guard from resize_tray_popup`
7. `fix(s34-007): re-add get_audio_volume to fastPoll for volume slider sync`

## What Was Deferred

| Ticket                                 | Reason                                             | Next Action |
| -------------------------------------- | -------------------------------------------------- | ----------- |
| S35-005 (AudioControl dirty flag)      | Not critical for sync, TrayPopup already has guard | Sprint 35   |
| S35-006 (backdrop-filter verification) | Override already exists, needs verification only   | Sprint 35   |

---

## Sprint Completion Checklist

After all tickets are committed:

- [ ] All 7 tickets have passing health checks (9/9)
- [ ] All commits pushed to `main`
- [ ] `sprint-overview.md` updated with Sprint 34 status
- [ ] Manual test: No UAC prompt when changing brightness/performance mode/fan mode
- [ ] Manual test: Tray popup renders correctly (cards visible, correct size)
- [ ] Manual test: Volume slider updates when system volume changes externally
- [ ] Manual test: `schtasks /query /tn MiControlElevated /xml` returns valid XML after fresh install
