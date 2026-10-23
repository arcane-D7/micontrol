# Sprint 24 — P2 MEDIUM: Architecture, UI, Performance, AI & DevOps (Post-Audit v2)

## Sprint Metadata

| Field                 | Value                                                                                           |
| --------------------- | ----------------------------------------------------------------------------------------------- |
| **Sprint Name**       | Architecture, UI, Performance, AI & DevOps                                                      |
| **Sprint Goal**       | Batch-fix all MEDIUM findings across security, architecture, UI/UX, performance, AI, and DevOps |
| **Duration Estimate** | ~5 days                                                                                         |
| **Priority**          | P2 — Medium                                                                                     |
| **Sprint Type**       | Multi-domain (Backend, Frontend, AI, DevOps)                                                    |
| **Primary Owner**     | Full-stack engineer                                                                             |
| **Source**            | `docs/STABILITY_REPORT_v2.md` — All MEDIUM findings                                             |
| **Depends On**        | Sprint 23                                                                                       |

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **OBRIGATÓRIO: 100% dos tickets desta sprint devem ser concluídos. A sprint não será aceita como entregue se qualquer ticket permanecer incompleto.**
>
> **MANDATORY: 100% of the tickets in this sprint MUST be completed. The sprint will NOT be accepted as delivered if any ticket remains incomplete.**

Every ticket must pass its acceptance criteria AND the full health check suite (9/9) before the sprint commit is made.

---

## Sprint Goal Statement

The post-sprint-21 stability audit (v2) identified 19 MEDIUM findings across 6 domains. This sprint batches them into 3 parallel execution groups:

- **Batch A (Rust Backend):** Security nonce persistence, key rotation, poison recovery, battery init, error code standardization
- **Batch B (Frontend):** Sentry reporting, accessibility, i18n error messages, per-tab error boundaries, missing `run_blocking`, re-render optimization
- **Batch C (AI & DevOps):** URL validation, usage persistence, code signing enforcement, Dependabot, SAST

---

## Health Check Commands (must pass 9/9 before commit)

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

## Batch A — Rust Backend (S24-001 through S24-008)

### S24-001 — Call `flush_nonces()` before `exit(0)` in elevated helper

| Field                | Value                                                                     |
| -------------------- | ------------------------------------------------------------------------- |
| **Ticket ID**        | S24-001                                                                   |
| **Title**            | Call `flush_nonces()` before `std::process::exit(0)` in `elevated::run()` |
| **Priority**         | P2                                                                        |
| **Type**             | Security / Data Persistence                                               |
| **Estimated Effort** | XS                                                                        |
| **Source Finding**   | S-M01 (MEDIUM)                                                            |

#### Context

In `src-tauri/src/elevated.rs`, the `run()` function processes one command and then exits via `std::process::exit(0)`. Nonces are persisted every 3rd insertion (`if map.len().is_multiple_of(3) { save_nonces(map); }`), but since the process exits after a single command, 2 of every 3 nonces are lost. A `flush_nonces()` function exists at line 157 but is never called in the exit path.

#### Acceptance Criteria

- [x] `flush_nonces()` is called before every `std::process::exit(0)` in `elevated.rs`
- [x] The call is placed after the result file is written but before exit
- [x] `flush_nonces()` is also called before the error-path exit (line ~38, where `select_pending_command` fails)
- [x] Unit test verifies nonces are persisted after a single command (existing `test_flush_nonces_persists_to_disk` may need updating)
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S24-002 — Wire `key_needs_rotation()` into startup or remove dead code

| Field                | Value                                                                         |
| -------------------- | ----------------------------------------------------------------------------- |
| **Ticket ID**        | S24-002                                                                       |
| **Title**            | Call `key_needs_rotation()` in app startup and log warning if rotation needed |
| **Priority**         | P2                                                                            |
| **Type**             | Security / Code Quality                                                       |
| **Estimated Effort** | S                                                                             |
| **Source Finding**   | S-M02 (MEDIUM)                                                                |

#### Context

In `src-tauri/src/util/auth.rs:383-460`, the functions `key_needs_rotation()`, `rotate_key()`, and `read_old_key()` are defined but never called from anywhere. The key rotation mechanism is dead code. This means if the HMAC key is compromised, there is no way to rotate it.

#### Acceptance Criteria

- [x] `key_needs_rotation()` is called during app startup in `lib.rs` setup (or in `elevated.rs::run()`)
- [x] If rotation is needed, log a `warn!` message: "HMAC key rotation needed — run with --rotate-key to rotate"
- [x] Do NOT auto-rotate (requires elevated process and careful key migration)
- [x] `read_old_key()` is wired into `verify_payload` to support grace period (see S24-003), OR both S24-002 and S24-003 are resolved by removing the dead code entirely
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S24-003 — Try `read_old_key()` in `verify_payload` or remove grace period

| Field                | Value                                                                |
| -------------------- | -------------------------------------------------------------------- |
| **Ticket ID**        | S24-003                                                              |
| **Title**            | Implement key rotation grace period in `verify_payload` or remove it |
| **Priority**         | P2                                                                   |
| **Type**             | Security                                                             |
| **Estimated Effort** | S                                                                    |
| **Source Finding**   | S-M03 (MEDIUM)                                                       |

#### Context

The key rotation grace period is broken: `verify_payload` only tries the current key, not the old key. If the key is rotated, all pending elevated commands with the old key signature would be rejected. The `read_old_key()` function exists but is never called.

**Decision point:** If S24-002 chose to keep the rotation code, this ticket must implement the grace period. If S24-002 chose to remove the dead code, this ticket is a no-op (mark as completed by removal).

#### Acceptance Criteria (if keeping rotation)

- [x] `verify_payload` accepts a slice of keys: `&[&[u8]]` or tries current key first, then old key
- [x] In `elevated.rs::run()`, after `read_key()`, also try `read_old_key()` and pass both to `verify_payload`
- [x] If old key validates, log a warning that key rotation is in progress
- [x] Unit test verifies payload signed with old key is accepted during grace period

#### Acceptance Criteria (if removing rotation)

- [x] `key_needs_rotation()`, `rotate_key()`, `read_old_key()` are removed from `auth.rs`
- [x] All references removed
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S24-004 — Replace `expect()` on thread spawn in `start_hook()`

| Field                | Value                                                                                      |
| -------------------- | ------------------------------------------------------------------------------------------ |
| **Ticket ID**        | S24-004                                                                                    |
| **Title**            | Replace `expect()` on thread spawn in `hotkeys::start_hook()` with graceful error handling |
| **Priority**         | P2                                                                                         |
| **Type**             | Stability / Error Handling                                                                 |
| **Estimated Effort** | S                                                                                          |
| **Source Finding**   | A-M01 (MEDIUM)                                                                             |

#### Context

In `src-tauri/src/hw/hotkeys.rs:288`, `start_hook()` calls `.expect("Failed to spawn hotkey thread")` on `thread::spawn()`. If the system is under resource exhaustion (too many threads), this panics the entire application instead of degrading gracefully.

#### Acceptance Criteria

- [x] `start_hook()` returns `Result<(), HardwareError>` instead of panicking
- [x] `.expect()` is replaced with `?` and proper error mapping (`io::Error` → `HardwareError`)
- [x] Callers of `start_hook()` handle the error (log warning, continue without hotkeys)
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S24-005 — Replace `expect()` on battery static data

| Field                | Value                                                                                   |
| -------------------- | --------------------------------------------------------------------------------------- |
| **Ticket ID**        | S24-005                                                                                 |
| **Title**            | Replace `expect()` on `BATTERY_STATIC_DATA` with `ok_or_else` returning `HardwareError` |
| **Priority**         | P2                                                                                      |
| **Type**             | Stability / Error Handling                                                              |
| **Estimated Effort** | XS                                                                                      |
| **Source Finding**   | A-M02 (MEDIUM)                                                                          |

#### Context

In `src-tauri/src/hw/battery.rs:155`, the code calls `.expect("battery static data should be initialized after init")` on the cached static data. While logically safe if `init()` is always called first, this panics if the code is refactored and `init()` is skipped.

#### Acceptance Criteria

- [x] `.expect()` is replaced with `.ok_or_else(|| HardwareError::Battery("static data not initialized".to_string()))?`
- [x] The function returns `HardwareResult<T>` (or `Result<T, ErrorResponse>`)
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S24-006 — Use `lock_or_recover` consistently in all modules

| Field                | Value                                                                                                     |
| -------------------- | --------------------------------------------------------------------------------------------------------- |
| **Ticket ID**        | S24-006                                                                                                   |
| **Title**            | Replace silent poison recovery patterns with `lock_or_recover` in state, hotkeys, battery, and iotservice |
| **Priority**         | P2                                                                                                        |
| **Type**             | Architecture / Consistency                                                                                |
| **Estimated Effort** | M                                                                                                         |
| **Source Finding**   | A-M03, A-M04, A-M05 (MEDIUM)                                                                              |

#### Context

`lock_or_recover()` in `util/panic.rs` is the standard pattern for mutex poison recovery across the codebase. However, 4 modules still use inconsistent patterns:

1. `state.rs:46-48` — `if let Ok(mut guard) = self.hardware_profile.write()` (RwLock, silently skips on poison)
2. `hotkeys.rs:271-276` — `.ok()` / `if let Ok(...)` (RwLock, silently skips)
3. `battery.rs:330-345` — `if let Ok(cache) = ...lock()` (Mutex, silently skips)
4. `iotservice.rs:490-495` — `.unwrap_or_else(|e| e.into_inner())` (Mutex, recovers but doesn't log)

#### Acceptance Criteria

- [x] `state.rs`: Replace silent `if let Ok(...)` with `lock_or_recover` (or add `lock_write_or_recover` for RwLock to `panic.rs`)
- [x] `hotkeys.rs`: Replace `.ok()` patterns with `lock_or_recover`
- [x] `battery.rs`: Replace `if let Ok(...)` with `lock_or_recover`
- [x] `iotservice.rs`: Replace `.unwrap_or_else(|e| e.into_inner())` with `lock_or_recover`
- [x] If RwLock support is needed, add `lock_write_or_recover` / `lock_read_or_recover` to `util/panic.rs`
- [x] All poison recovery now logs a warning via `log::warn!`
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S24-007 — Use `OnceLock` for battery static data initialization

| Field                | Value                                                                                             |
| -------------------- | ------------------------------------------------------------------------------------------------- |
| **Ticket ID**        | S24-007                                                                                           |
| **Title**            | Replace `Mutex<Option<BatteryStaticData>>` with `OnceLock` to avoid holding lock across WMI query |
| **Priority**         | P2                                                                                                |
| **Type**             | Performance / Architecture                                                                        |
| **Estimated Effort** | M                                                                                                 |
| **Source Finding**   | A-M06 (MEDIUM)                                                                                    |

#### Context

In `src-tauri/src/hw/battery.rs:100-150`, the `BATTERY_STATIC_DATA` is a `Mutex<Option<BatteryStaticData>>`. During `init()`, the mutex is locked while a slow WMI query runs, blocking all `get_battery_info()` callers. Using `OnceLock` would allow one-time initialization without holding a lock during the WMI query, and subsequent reads would be lock-free.

#### Acceptance Criteria

- [x] `BATTERY_STATIC_DATA` is changed from `Mutex<Option<BatteryStaticData>>` to `OnceLock<BatteryStaticData>`
- [x] `init()` uses `BATTERY_STATIC_DATA.get_or_init(|| { /* WMI query */ })`
- [x] `get_battery_info()` uses `BATTERY_STATIC_DATA.get()` which returns `Option<&BatteryStaticData>` without locking
- [x] If `get()` returns `None` (init not called), return appropriate error (not panic)
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S24-008 — Standardize error codes and remove ad-hoc codes

| Field                | Value                                                                                 |
| -------------------- | ------------------------------------------------------------------------------------- |
| **Ticket ID**        | S24-008                                                                               |
| **Title**            | Standardize fallback error codes to `"other"` and remove ad-hoc `INVALID_STATUS` code |
| **Priority**         | P2                                                                                    |
| **Type**             | Architecture / Error Handling                                                         |
| **Estimated Effort** | S                                                                                     |
| **Source Finding**   | A-M07, A-M08 (MEDIUM)                                                                 |

#### Context

In `src-tauri/src/hw/errors.rs:210-218`, there are three different fallback error codes:

- `From<String>` → `"UNKNOWN_ERROR"`
- `From<anyhow::Error>` → `"INTERNAL_ERROR"`
- `from_display` → `"other"`

The frontend must handle all three. Additionally, in `src-tauri/src/commands/hardware.rs:230-234`, an ad-hoc `ErrorResponse { code: "INVALID_STATUS", ... }` is manually constructed, bypassing the `HardwareError` enum entirely.

#### Acceptance Criteria

- [x] `From<String> for ErrorResponse` uses code `"other"` (not `"UNKNOWN_ERROR"`)
- [x] `From<anyhow::Error> for ErrorResponse` uses code `"other"` (not `"INTERNAL_ERROR"`)
- [x] `hardware.rs:230` replaces manual `ErrorResponse { code: "INVALID_STATUS"... }` with `HardwareError::InvalidConfig(format!("Invalid laptop status: {status}"))` or appropriate typed variant
- [x] Add a comment in `errors.rs` documenting the full list of error codes
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

## Batch B — Frontend (S24-009 through S24-014)

### S24-009 — Add `Sentry.captureException()` in ErrorBoundary `didCatch`

| Field                | Value                                                                 |
| -------------------- | --------------------------------------------------------------------- |
| **Ticket ID**        | S24-009                                                               |
| **Title**            | Call `Sentry.captureException()` in `ErrorBoundary.componentDidCatch` |
| **Priority**         | P2                                                                    |
| **Type**             | Observability / UX                                                    |
| **Estimated Effort** | XS                                                                    |
| **Source Finding**   | U-M01 (MEDIUM)                                                        |

#### Context

In `src/components/ErrorBoundary.tsx:61`, `componentDidCatch` only logs to `console.error`. React crashes never reach Sentry, so production crashes are invisible to developers.

#### Acceptance Criteria

- [x] `componentDidCatch` calls `Sentry.captureException(error, { extra: { componentStack: errorInfo.componentStack } })`
- [x] Import `@sentry/react` and use `Sentry.captureException`
- [x] Sentry call is wrapped in try/catch to prevent crash-loop if Sentry fails
- [x] `npx tsc --noEmit` passes, `npm run lint` passes, `npm run build` passes

---

### S24-010 — Add focus trap and Escape handler to OnboardingWizard

| Field                | Value                                                                       |
| -------------------- | --------------------------------------------------------------------------- |
| **Ticket ID**        | S24-010                                                                     |
| **Title**            | Add focus trap, Escape key handler, and ARIA attributes to OnboardingWizard |
| **Priority**         | P2                                                                          |
| **Type**             | Accessibility / UX                                                          |
| **Estimated Effort** | M                                                                           |
| **Source Finding**   | U-M02 (MEDIUM)                                                              |

#### Context

`src/components/OnboardingWizard.tsx` lacks keyboard navigation. Unlike `ConsentDialog` which has a proper focus trap, Escape handler, and ARIA attributes, OnboardingWizard has none. Keyboard users cannot navigate or dismiss the wizard.

#### Acceptance Criteria

- [x] Modal container has `role="dialog"` and `aria-modal="true"`
- [x] Focus trap implemented: Tab key cycles within modal
- [x] Escape key handler closes the wizard
- [x] Initial focus set to first interactive element on open
- [x] Focus restored to trigger element on close
- [x] Follow the same pattern as `ConsentDialog.tsx`
- [x] `npx tsc --noEmit` passes, `npm run lint` passes, `npm run build` passes

---

### S24-011 — Route error messages through i18n system

| Field                | Value                                                                |
| -------------------- | -------------------------------------------------------------------- |
| **Ticket ID**        | S24-011                                                              |
| **Title**            | Replace hardcoded English in `getUserFriendlyMessage` with i18n keys |
| **Priority**         | P2                                                                   |
| **Type**             | i18n / UX                                                            |
| **Estimated Effort** | M                                                                    |
| **Source Finding**   | U-M03 (MEDIUM)                                                       |

#### Context

In `src/types/error.ts:73-89`, `getUserFriendlyMessage` returns hardcoded English strings, bypassing the i18n system. Non-English users see English error messages.

#### Acceptance Criteria

- [x] `getUserFriendlyMessage` accepts a translation function `t: (key: string) => string`
- [x] All hardcoded English strings replaced with i18n keys (e.g., `t('errors.wifiInvalidSsid')`)
- [x] New keys added to all 4 locale files (`en.json`, `pt.json`, `es.json`, `fr.json`)
- [x] All callers updated to pass the `t` function from `useI18n`
- [x] `npx tsc --noEmit` passes, `npm run lint` passes, `npm run build` passes

---

### S24-012 — Add per-tab ErrorBoundary

| Field                | Value                                                                       |
| -------------------- | --------------------------------------------------------------------------- |
| **Ticket ID**        | S24-012                                                                     |
| **Title**            | Wrap each tab content in its own ErrorBoundary to prevent full-window crash |
| **Priority**         | P2                                                                          |
| **Type**             | UX / Resilience                                                             |
| **Estimated Effort** | S                                                                           |
| **Source Finding**   | U-M04 (MEDIUM)                                                              |

#### Context

In `src/App.tsx:62-64`, a single top-level `ErrorBoundary` wraps the entire app. A crash in one tab (e.g., Fan tab) brings down the whole window. Per-tab boundaries would isolate crashes.

#### Acceptance Criteria

- [x] Each tab content wrapped in its own `<ErrorBoundary>` component
- [x] Tab-level ErrorBoundary shows compact error message with "Reload tab" button
- [x] Top-level ErrorBoundary kept as last-resort catch-all
- [x] `npx tsc --noEmit` passes, `npm run lint` passes, `npm run build` passes

---

### S24-013 — Wrap sync hw calls in `run_blocking`

| Field                | Value                                                                                     |
| -------------------- | ----------------------------------------------------------------------------------------- |
| **Ticket ID**        | S24-013                                                                                   |
| **Title**            | Wrap `get_performance_mode`, `get_charging_threshold`, `get_perf_debug` in `run_blocking` |
| **Priority**         | P2                                                                                        |
| **Type**             | Performance / Concurrency                                                                 |
| **Estimated Effort** | S                                                                                         |
| **Source Finding**   | P-M01, P-M02, P-M03, P-L01 (MEDIUM/LOW)                                                   |

#### Context

In `src-tauri/src/commands/hardware.rs:37-39, 50-52, 63-65`, three Tauri commands call synchronous hardware functions (`hw_get_perf()`, `hw_get_charge()`, `hw_perf_debug()`) directly from async context without `run_blocking`. These functions perform WMI/registry I/O and block the Tokio runtime. Additionally, `get_ai_brightness_config` in `system.rs:131-133` has the same issue.

#### Acceptance Criteria

- [x] `get_performance_mode` wraps `hw_get_perf()` in `run_blocking(move || Ok(hw_get_perf()))`
- [x] `get_charging_threshold` wraps `hw_get_charge()` in `run_blocking(move || Ok(hw_get_charge()))`
- [x] `get_perf_debug` wraps `hw_perf_debug()` in `run_blocking(move || Ok(hw_perf_debug()))`
- [x] `get_ai_brightness_config` in `system.rs` wraps registry read in `run_blocking`
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S24-014 — Split useHardware context to prevent re-render storm

| Field                | Value                                                                                  |
| -------------------- | -------------------------------------------------------------------------------------- |
| **Ticket ID**        | S24-014                                                                                |
| **Title**            | Split `useHardware` useMemo to prevent all consumers from re-rendering every 2 seconds |
| **Priority**         | P2                                                                                     |
| **Type**             | Performance / Frontend                                                                 |
| **Estimated Effort** | M                                                                                      |
| **Source Finding**   | P-M05 (MEDIUM)                                                                         |

#### Context

In `src/hooks/useHardware.ts:860-950`, a single `useMemo` spreads `...fanState` and `...systemState` into one return object. Since the hardware hook polls every 2 seconds, all consumers of the context re-render every 2 seconds, even if only fan data changed.

#### Acceptance Criteria

- [x] Split the single `useMemo` into separate slices (fan, system, battery) OR use a selector pattern
- [x] Minimum viable fix: 3 separate `useMemo` calls so each slice only re-renders when its data changes
- [x] Use `React.memo` on consumer components to prevent unnecessary re-renders
- [x] `npx tsc --noEmit` passes, `npm run lint` passes, `npm run build` passes

---

## Batch C — AI & DevOps (S24-015 through S24-019)

### S24-015 — Add URL validation for AI `base_url`

| Field                | Value                                                           |
| -------------------- | --------------------------------------------------------------- |
| **Ticket ID**        | S24-015                                                         |
| **Title**            | Validate `base_url` scheme and host before passing to `reqwest` |
| **Priority**         | P2                                                              |
| **Type**             | Security / AI Responsibility                                    |
| **Estimated Effort** | S                                                               |
| **Source Finding**   | AI-M01 (MEDIUM)                                                 |

#### Context

In `src-tauri/src/commands/ai.rs:43-55`, `base_url` is user-controlled and passed directly to `reqwest::Client`. A malicious URL could exfiltrate the API key to an attacker-controlled server.

#### Acceptance Criteria

- [x] Parse `base_url` with `url::Url::parse()`
- [x] Validate scheme is `https` (or `http` for localhost/Ollama)
- [x] If `http`, require host to be `localhost` or `127.0.0.1`
- [x] Return descriptive error on validation failure
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S24-016 — Persist AI usage tracking to file

| Field                | Value                                                          |
| -------------------- | -------------------------------------------------------------- |
| **Ticket ID**        | S24-016                                                        |
| **Title**            | Persist `AiUsageStats` to file and enforce backend daily limit |
| **Priority**         | P2                                                             |
| **Type**             | AI Responsibility / Persistence                                |
| **Estimated Effort** | M                                                              |
| **Source Finding**   | AI-M02 (MEDIUM)                                                |

#### Context

In `src-tauri/src/util/ai_usage.rs:22-40`, AI usage tracking is in-memory only. Stats are lost on restart, and there is no backend-enforced daily limit.

#### Acceptance Criteria

- [x] `AiUsageStats` has `save_to_file()` and `load_from_file()` methods
- [x] Stats saved to `%LOCALAPPDATA%\MiControl\ai_usage.json`
- [x] Stats loaded on startup
- [x] Backend-enforced daily limit: if `today_count >= ai_daily_analyses`, return error
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S24-017 — Fail release if no code signing certificate

| Field                | Value                                           |
| -------------------- | ----------------------------------------------- |
| **Ticket ID**        | S24-017                                         |
| **Title**            | Make code signing mandatory in release workflow |
| **Priority**         | P2                                              |
| **Type**             | DevOps / Security                               |
| **Estimated Effort** | XS                                              |
| **Source Finding**   | D-M01 (MEDIUM)                                  |

#### Context

In `.github/workflows/release.yml:62-70`, code signing is optional. If `WINDOWS_CERTIFICATE` is not set, the release proceeds unsigned, triggering SmartScreen warnings for users.

#### Acceptance Criteria

- [x] Release workflow fails with `::error::` if `WINDOWS_CERTIFICATE` secret is not set
- [x] No `continue-on-error` on the signing step
- [x] Error message is clear: "WINDOWS_CERTIFICATE secret is required for production releases"

---

### S24-018 — Add Dependabot configuration

| Field                | Value                                                            |
| -------------------- | ---------------------------------------------------------------- |
| **Ticket ID**        | S24-018                                                          |
| **Title**            | Create `.github/dependabot.yml` for automated dependency updates |
| **Priority**         | P2                                                               |
| **Type**             | DevOps                                                           |
| **Estimated Effort** | XS                                                               |
| **Source Finding**   | D-M02 (MEDIUM)                                                   |

#### Context

No `dependabot.yml` or `renovate.json` exists. Dependencies are not automatically updated, creating security risk from outdated packages.

#### Acceptance Criteria

- [x] `.github/dependabot.yml` created with:
  - `cargo` ecosystem for `/src-tauri` (weekly)
  - `npm` ecosystem for `/` (weekly)
  - `github-actions` ecosystem for `/` (weekly)
- [x] Configuration follows Dependabot v2 schema

---

### S24-019 — Add CodeQL SAST to CI

| Field                | Value                                                         |
| -------------------- | ------------------------------------------------------------- |
| **Ticket ID**        | S24-019                                                       |
| **Title**            | Add CodeQL static analysis workflow for JavaScript/TypeScript |
| **Priority**         | P2                                                            |
| **Type**             | DevOps / Security                                             |
| **Estimated Effort** | XS                                                            |
| **Source Finding**   | D-M03 (MEDIUM)                                                |

#### Context

No SAST tool (CodeQL, Semgrep) is configured. `cargo audit` and `npm audit` only catch dependency CVEs, not code-level security anti-patterns.

#### Acceptance Criteria

- [x] `.github/workflows/codeql.yml` created
- [x] Workflow runs on push to `master` and on PRs to `master`
- [x] Analyzes JavaScript/TypeScript (Rust support in CodeQL is limited)
- [x] `permissions: security-events: write` set
- [x] Uses `github/codeql-action/init@v3` and `github/codeql-action/analyze@v3`

---

## Sprint Commit

```bash
git add -A
git commit -m "feat(sprint-24): architecture, UI, performance, AI, and DevOps improvements (P2)

Batch A - Rust Backend:
- S24-001: Call flush_nonces() before exit(0) (S-M01)
- S24-002: Wire key_needs_rotation() into startup (S-M02)
- S24-003: Implement key rotation grace period (S-M03)
- S24-004: Replace expect() on thread spawn (A-M01)
- S24-005: Replace expect() on battery data (A-M02)
- S24-006: Use lock_or_recover consistently (A-M03/M04/M05)
- S24-007: Use OnceLock for battery init (A-M06)
- S24-008: Standardize error codes (A-M07/M08)

Batch B - Frontend:
- S24-009: Add Sentry.captureException in ErrorBoundary (U-M01)
- S24-010: Add focus trap to OnboardingWizard (U-M02)
- S24-011: Route error messages through i18n (U-M03)
- S24-012: Add per-tab ErrorBoundary (U-M04)
- S24-013: Wrap sync hw calls in run_blocking (P-M01/M02/M03)
- S24-014: Split useHardware context (P-M05)

Batch C - AI & DevOps:
- S24-015: Validate AI base_url (AI-M01)
- S24-016: Persist AI usage tracking (AI-M02)
- S24-017: Fail release if no signing cert (D-M01)
- S24-018: Add Dependabot config (D-M02)
- S24-019: Add CodeQL SAST (D-M03)"
```
