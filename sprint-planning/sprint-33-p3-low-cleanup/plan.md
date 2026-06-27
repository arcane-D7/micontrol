# Sprint 33 — P3 LOW: Code Cleanup & Dead Code Removal

> **Date:** 2026-06-27
> **Sprint:** 33
> **Theme:** Fix 8 low-priority issues — logging upgrades, dead code removal, dependency cleanup, dev-gating
> **Duration:** ~1–2 days
> **Dependencies:** Sprint 32 (all P2 medium fixes)
> **Status:** 📌 Active
> **Audit Reference:** `C:\Users\mafsc\Documents\Audit_Final.md` (L-1 through L-8)

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

This sprint addresses 8 low-priority cleanup items from the audit. While none are user-facing bugs, they affect code quality, maintainability, and developer experience:

1. **L-1:** Touchpad HID errors logged at `debug` (invisible in production) — should be `warn`
2. **L-2:** IoTService `.ok()` swallows error context — should log before discarding
3. **L-3:** Charging pipe send is fire-and-forget — documented as intentional, no fix needed
4. **L-4:** `@tauri-apps/plugin-shell` dependency unused in frontend — remove
5. **L-5:** `test_perf` binary compiled in release builds — gate behind feature flag
6. **L-6:** Touchpad elevated dispatch entries are dead code — remove
7. **L-7:** `get_perf_debug` button always visible — hide behind dev flag
8. **L-8:** ~~`discover_from_wmi()` in ecram.rs is stub~~ — **PULLED FORWARD to S31-008** (already implemented in Sprint 31)

**Note:** L-8 was pulled forward to Sprint 31 per the user's mandate that all stubs must be implemented. This sprint has 7 tickets (L-1 through L-7, excluding L-8).

---

## Goals

| #   | Goal                                            | KPI                                        | Audit Reference |
| --- | ----------------------------------------------- | ------------------------------------------ | --------------- |
| 1   | Touchpad HID errors visible in production logs  | `log::warn!` on HID failure                | L-1             |
| 2   | IoTService errors logged before being swallowed | `log::warn!` before `.ok()`                | L-2             |
| 3   | No unused frontend dependencies                 | `@tauri-apps/plugin-shell` removed         | L-4             |
| 4   | `test_perf` not in release builds               | Gated behind `test-bin` feature            | L-5             |
| 5   | No dead code in elevated dispatch               | Unused touchpad entries removed            | L-6             |
| 6   | Channel Diagnostics button dev-only             | Hidden when `import.meta.env.DEV` is false | L-7             |

---

## Technical Specs

### S33-001: Upgrade touchpad HID error logging from `debug` to `warn` (L-1)

| Field         | Value                                                                      |
| ------------- | -------------------------------------------------------------------------- |
| **Ticket ID** | S33-001                                                                    |
| **Title**     | Change `log::debug!` to `log::warn!` for HID error handlers in touchpad.rs |
| **Priority**  | P3 — Low                                                                   |
| **Source**    | L-1 (Audit_Final.md)                                                       |
| **Files**     | `src-tauri/src/hw/touchpad.rs`                                             |
| **Effort**    | ~15 minutes                                                                |
| **Type**      | Backend (Rust)                                                             |

#### Problem

HID errors in `touchpad.rs` are logged at `debug` level, which is invisible in production builds. When haptics fail, there's no trace in the logs.

**Note:** S31-006 already changes these to `log::warn!` as part of the touchpad logging improvement. This ticket is a fallback in case S31-006 doesn't cover all instances. Verify all `log::debug!` calls related to HID are upgraded.

#### Solution

Search for all `log::debug!` calls in `touchpad.rs` that reference HID errors and change to `log::warn!`:

```rust
// All instances of:
send_haptics_hid_report(...).unwrap_or_else(|e| log::debug!("[touchpad] haptics HID: {e}"));
// Change to:
send_haptics_hid_report(...).unwrap_or_else(|e| log::warn!("[touchpad] haptics HID report failed: {e}"));
```

#### Acceptance Criteria

- [ ] Zero `log::debug!` calls for HID errors in `touchpad.rs`
- [ ] All HID error handlers use `log::warn!`
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes

---

### S33-002: Add `log::warn!` before `.ok()` in IoTService (L-2)

| Field         | Value                                                |
| ------------- | ---------------------------------------------------- |
| **Ticket ID** | S33-002                                              |
| **Title**     | Log IoTService errors before swallowing with `.ok()` |
| **Priority**  | P3 — Low                                             |
| **Source**    | L-2 (Audit_Final.md)                                 |
| **Files**     | `src-tauri/src/hw/iotservice.rs`                     |
| **Effort**    | ~30 minutes                                          |
| **Type**      | Backend (Rust)                                       |

#### Problem

In `iotservice.rs:1070-1090`, `get_device_info()` queries each field independently via `send_query()`, and `.ok()` swallows errors silently. If the pipe is unavailable, all fields return `None` with no log trace.

#### Solution

Add `log::warn!` before each `.ok()` in the `get_device_info()` function:

```rust
// For each field query, change from:
let model = send_query(...).ok();
// To:
let model = send_query(...).map_err(|e| {
    log::warn!("[iot] Failed to query model: {e}");
    e
}).ok();
```

Apply to all fields in `get_device_info()` (model, serial, firmware, etc.).

#### Acceptance Criteria

- [ ] All `.ok()` calls in `get_device_info()` preceded by error logging
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes

---

### S33-003: Remove unused `@tauri-apps/plugin-shell` dependency (L-4)

| Field         | Value                                                 |
| ------------- | ----------------------------------------------------- |
| **Ticket ID** | S33-003                                               |
| **Title**     | Remove `@tauri-apps/plugin-shell` from `package.json` |
| **Priority**  | P3 — Low                                              |
| **Source**    | L-4 (Audit_Final.md)                                  |
| **Files**     | `package.json`                                        |
| **Effort**    | ~10 minutes                                           |
| **Type**      | Frontend (npm)                                        |

#### Problem

`@tauri-apps/plugin-shell` is listed in `package.json:35` but is never imported or used in any frontend code.

#### Solution

1. Remove from `package.json` dependencies
2. Run `npm install` to update `package-lock.json`
3. Verify no imports reference it

```bash
# Verify no imports exist
grep -r "plugin-shell" src/
# Should return nothing
```

#### Acceptance Criteria

- [ ] `@tauri-apps/plugin-shell` removed from `package.json`
- [ ] `npm install` succeeds
- [ ] `npm run build` succeeds
- [ ] No broken imports

---

### S33-004: Gate `test_perf` binary behind feature flag (L-5)

| Field         | Value                                                    |
| ------------- | -------------------------------------------------------- |
| **Ticket ID** | S33-004                                                  |
| **Title**     | Add `#[cfg(feature = "test-bin")]` to `test_perf` binary |
| **Priority**  | P3 — Low                                                 |
| **Source**    | L-5 (Audit_Final.md)                                     |
| **Files**     | `src-tauri/Cargo.toml`                                   |
| **Effort**    | ~15 minutes                                              |
| **Type**      | Backend (Rust)                                           |

#### Problem

The `test_perf` binary is compiled in release builds, adding unnecessary build time and binary size.

#### Current Code

```toml
# Cargo.toml:20-22
[[bin]]
name = "test_perf"
path = "src/bin/test_perf.rs"
```

#### Solution

Add an optional feature and gate the binary:

```toml
[features]
default = []
test-bin = []

[[bin]]
name = "test_perf"
path = "src/bin/test_perf.rs"
required-features = ["test-bin"]
```

Then only build with `cargo build --features test-bin` when needed.

#### Acceptance Criteria

- [ ] `test_perf` binary has `required-features = ["test-bin"]`
- [ ] `test-bin` feature added to `[features]`
- [ ] `cargo build` (without features) does NOT compile `test_perf`
- [ ] `cargo build --features test-bin` DOES compile `test_perf`
- [ ] `cargo check` passes

---

### S33-005: Remove dead touchpad dispatch entries from `elevated.rs` (L-6)

| Field         | Value                                            |
| ------------- | ------------------------------------------------ |
| **Ticket ID** | S33-005                                          |
| **Title**     | Remove unused touchpad elevated dispatch entries |
| **Priority**  | P3 — Low                                         |
| **Source**    | L-6 (Audit_Final.md)                             |
| **Files**     | `src-tauri/src/elevated.rs`                      |
| **Effort**    | ~30 minutes                                      |
| **Type**      | Backend (Rust)                                   |

#### Problem

The `elevated.rs` dispatch table (lines 335-400) contains entries for `set_touchpad_sensitivity`, `set_touchpad_haptics`, `set_touchpad_haptics_intensity`, `set_touchpad_gesture_screenshot`, `set_touchpad_repress`, and `set_touchpad_edge_slide`. These touchpad operations write to `HKCU` (not `HKLM`) and don't need elevation — they are handled directly by the non-elevated Tauri commands.

#### Solution

**Step 1:** Verify these entries are never called via the elevated bridge. Search for `elev_bridge::run_elevated` calls with touchpad commands in `commands/hardware.rs`:

```bash
grep -n "set_touchpad" src-tauri/src/commands/hardware.rs
```

If the touchpad commands are called directly (not through the bridge), the elevated dispatch entries are dead code.

**Step 2:** Remove the dead entries from `elevated.rs`:

```rust
// Remove these match arms from the dispatch function:
"set_touchpad_sensitivity" => { ... }
"set_touchpad_haptics" => { ... }
"set_touchpad_haptics_intensity" => { ... }
"set_touchpad_gesture_screenshot" => { ... }
"set_touchpad_repress" => { ... }
"set_touchpad_edge_slide" => { ... }
```

**Note:** Only remove if confirmed they are not called via the bridge. If any ARE called via the bridge, keep those and remove only the confirmed dead ones.

#### Acceptance Criteria

- [ ] Verified touchpad commands are called directly, not via bridge
- [ ] Dead dispatch entries removed from `elevated.rs`
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes

---

### S33-006: Dev-gate the Channel Diagnostics button (L-7)

| Field         | Value                                                        |
| ------------- | ------------------------------------------------------------ |
| **Ticket ID** | S33-006                                                      |
| **Title**     | Hide Channel Diagnostics button behind `import.meta.env.DEV` |
| **Priority**  | P3 — Low                                                     |
| **Source**    | L-7 (Audit_Final.md)                                         |
| **Files**     | `src/pages/tabs/performance.tsx`                             |
| **Effort**    | ~15 minutes                                                  |
| **Type**      | Frontend (TypeScript)                                        |

#### Problem

The Channel Diagnostics button (`get_perf_debug`) is always visible in the Performance tab. This is a developer diagnostic tool that shouldn't be in production.

#### Solution

Wrap the Channel Diagnostics section in a dev check:

```tsx
{import.meta.env.DEV && (
  <div className="card">
    {/* Performance channel diagnostics */}
    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
      <div>
        <div className="card-title" style={{ marginBottom: 2 }}>
          {t('performance.channels.title')}
        </div>
        <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>
          {t('performance.channels.subtitle')}
        </div>
      </div>
      <button
        className="btn-secondary"
        style={{ fontSize: 12 }}
        onClick={() => void runPerfDebug()}
        disabled={loadingDebug}
      >
        {loadingDebug ? t('performance.channels.checking') : t('performance.channels.checkNow')}
      </button>
    </div>
    {debugInfo && (/* ... existing debug info rendering ... */)}
  </div>
)}
```

#### Acceptance Criteria

- [ ] Channel Diagnostics section only renders when `import.meta.env.DEV` is true
- [ ] In production build, the section is not visible
- [ ] In dev mode, the section is still accessible
- [ ] `npx tsc --noEmit` passes
- [ ] `npm run build` succeeds

---

### S33-007: Document charging pipe fire-and-forget as intentional (L-3)

| Field         | Value                                                               |
| ------------- | ------------------------------------------------------------------- |
| **Ticket ID** | S33-007                                                             |
| **Title**     | Add documentation comment for charging pipe fire-and-forget pattern |
| **Priority**  | P3 — Low                                                            |
| **Source**    | L-3 (Audit_Final.md)                                                |
| **Files**     | `src-tauri/src/hw/charging.rs`                                      |
| **Effort**    | ~10 minutes                                                         |
| **Type**      | Backend (Rust, documentation)                                       |

#### Problem

The charging pipe send is fire-and-forget (no response read). This is documented as intentional in the audit, but the code doesn't have a comment explaining why.

#### Solution

Add a documentation comment:

```rust
/// Send a command to the IoTService charging pipe.
///
/// This is intentionally fire-and-forget: the IoTService pipe protocol
/// does not return a response for charging threshold commands. The
/// command is validated before sending, and the registry is updated
/// separately. If the pipe send fails, the registry value still
/// reflects the user's intent and will be applied on next IoTService
/// restart.
fn send_charging_command(/* ... */) {
    // ... existing code ...
}
```

#### Acceptance Criteria

- [ ] Documentation comment added explaining the fire-and-forget pattern
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes

---

## Story Points

| Ticket    | Points | Owner    | Wave                              |
| --------- | ------ | -------- | --------------------------------- |
| S33-001   | 1      | Backend  | 1 (touchpad.rs — independent)     |
| S33-002   | 1      | Backend  | 1 (iotservice.rs — independent)   |
| S33-003   | 1      | Frontend | 1 (package.json — independent)    |
| S33-004   | 1      | Backend  | 1 (Cargo.toml — independent)      |
| S33-005   | 1      | Backend  | 1 (elevated.rs — independent)     |
| S33-006   | 1      | Frontend | 1 (performance.tsx — independent) |
| S33-007   | 1      | Backend  | 1 (charging.rs — independent)     |
| **Total** | **7**  |          |                                   |

## Dependency Map

```
Wave 1 (all parallel — 7 independent tickets):
  S33-001: src-tauri/src/hw/touchpad.rs
  S33-002: src-tauri/src/hw/iotservice.rs
  S33-003: package.json
  S33-004: src-tauri/Cargo.toml
  S33-005: src-tauri/src/elevated.rs
  S33-006: src/pages/tabs/performance.tsx
  S33-007: src-tauri/src/hw/charging.rs
```

All 7 tickets modify different files and have no logical dependencies.

## Commit Strategy

One commit per ticket:

1. `fix(s33-001): upgrade touchpad HID error logging to warn level`
2. `fix(s33-002): log IoTService errors before swallowing with ok()`
3. `chore(s33-003): remove unused @tauri-apps/plugin-shell dependency`
4. `chore(s33-004): gate test_perf binary behind test-bin feature flag`
5. `refactor(s33-005): remove dead touchpad dispatch entries from elevated.rs`
6. `fix(s33-006): dev-gate channel diagnostics button`
7. `docs(s33-007): document charging pipe fire-and-forget as intentional`

## What Was Deferred

| Ticket                  | Reason                    | Next Action                      |
| ----------------------- | ------------------------- | -------------------------------- |
| L-8 (discover_from_wmi) | Pulled forward to S31-008 | Already implemented in Sprint 31 |

---

## Sprint Completion Checklist

After all tickets are committed:

- [ ] All 7 tickets have passing health checks (9/9)
- [ ] All commits pushed to `master`
- [ ] `sprint-overview.md` updated with Sprint 33 status
- [ ] No remaining stubs in the codebase (verified by grep for `stub`, `todo!`, `unimplemented!`, `for now`, `Future work`)
- [ ] Final verification: `grep -rn "stub\|todo!\|unimplemented!\|for now\|Future work" src-tauri/src/` returns only `#[cfg(not(windows))]` platform stubs (acceptable)
