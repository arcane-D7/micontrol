# Sprint 3 — Architecture & Stability: Crash Prevention

## Sprint Metadata

| Field | Value |
|-------|-------|
| **Sprint Name** | Architecture & Stability — Crash Prevention |
| **Sprint Goal** | Eliminate fatal crashes from `panic = "abort"`, fix state management fragmentation, introduce structured error handling, and move blocking calls off the async runtime |
| **Duration Estimate** | 3 weeks (15 working days) |
| **Priority** | P1 — Stability-critical. Crashes are user-visible and data-loss-prone. |
| **Sprint Type** | Refactor / Architecture |
| **Primary Owner** | Rust architect |
| **Secondary Owner** | Backend engineer (async runtime) |

## Sprint Goal Statement

The app currently crashes instantly on any `Mutex` poison, any `unwrap`/`expect` failure, or any blocking WMI/IGCL call that starves the tokio runtime. By the end of this sprint, panics are caught and logged (not fatal), state is centralized and consistent, errors are structured with codes, and blocking calls run on `spawn_blocking`. The app must survive a forced mutex-poison scenario without exiting.

---

## Background

Two critical architecture findings drive this sprint: (A1) blocking WMI/IGCL/PowerShell calls inside `adaptive_brightness_loop` starve the tokio runtime, and (A2) elevated command files use default ACLs with a TOCTOU race (this is addressed in Sprint 2's S2-001 from a security angle; this sprint addresses the broader stability pattern). The dominant crash amplifier is `panic = "abort"` in `Cargo.toml`, which turns every recoverable panic into an instant process death.

---

## Tickets

### S3-001 — Replace `panic = "abort"` with panic-catching and graceful degradation

| Field | Value |
|-------|-------|
| **Ticket ID** | S3-001 |
| **Title** | Remove `panic = "abort"`; add panic hooks and graceful error paths |
| **Priority** | P0 |
| **Type** | Refactor |
| **Estimated Effort** | M |

#### Description

`Cargo.toml` (~line 55) sets `panic = "abort"`. Combined with pervasive `.unwrap()`/`.expect()` calls (e.g. `commands/hardware.rs:42,63`), any panic — including a benign `Mutex` poison — kills the app instantly with no recovery. This ticket removes the abort strategy and replaces it with panic-catching in critical paths plus a global panic hook for logging.

#### Affected Files and Line Ranges

- `src-tauri/Cargo.toml` — `panic = "abort"` (~line 55).
- `src-tauri/src/main.rs` — panic hook installation.
- `src-tauri/src/commands/hardware.rs` — `.lock().unwrap()` calls (~lines 42, 63).

#### Root Cause Analysis

`panic = "abort"` is sometimes used to shrink binary size or avoid unwind tables, but for a long-running desktop app it is catastrophic: a single poisoned mutex (which happens whenever a thread panics while holding the lock) aborts the entire process. The standard library's `Mutex::lock().unwrap()` panics on poison by design; with unwind enabled, this is recoverable (use `lock().unwrap_or_else(|e| e.into_inner())`).

#### Acceptance Criteria

- [ ] `panic = "abort"` is removed from `Cargo.toml` (or set to `panic = "unwind"` explicitly).
- [ ] A global panic hook is installed in `main.rs` that logs the panic payload and backtrace to the app's log file before the default behavior.
- [ ] All `Mutex::lock().unwrap()` calls in `commands/hardware.rs` (and audited across the codebase) are replaced with a helper `fn lock_or_recover<T>(m: &Mutex<T>) -> MutexGuard<T>` that uses `unwrap_or_else(|e| e.into_inner())` to recover poisoned mutexes.
- [ ] Critical async tasks are wrapped in `tokio::spawn` with a `.unwrap_or_else` that logs and restarts, so a panic in one task does not crash the runtime.
- [ ] Manual test: deliberately poison a mutex in a debug build (inject a panic while holding the lock); confirm the app continues running and logs the recovery.
- [ ] Binary size impact is measured and documented (unwind tables add ~50–200 KB; acceptable for a desktop app).

#### Implementation Notes

- The panic hook should write to both `stderr` and the log file, and optionally emit a Tauri event so the frontend can show a non-fatal error toast.
- `lock_or_recover` should be a free function in a `util` module, not a trait, to keep it simple.
- Audit the codebase for `.unwrap()` on `Mutex::lock` specifically — these are the poison-prone sites. Other `.unwrap()` calls are addressed in S3-004.

#### Testing Strategy

- **Unit test**: a test that poisons a mutex and asserts `lock_or_recover` returns a usable guard.
- **Manual crash test**: trigger a panic in a background task; confirm the app survives and logs.
- **Soak test**: run the app for 1 hour with simulated random panics in non-critical paths; confirm no crash.

#### Dependencies

- None (foundational).

---

### S3-002 — Centralize state management; eliminate OnceLock/static global fragmentation

| Field | Value |
|-------|-------|
| **Ticket ID** | S3-002 |
| **Title** | Consolidate AppState and module-level statics into a single state container |
| **Priority** | P1 |
| **Type** | Refactor |
| **Estimated Effort** | XL |

#### Description

`AppState` holds only 2 fields, while the real application state lives in `OnceLock`/`static` globals scattered across 5+ modules (touchpad, display, hotkeys, charging, etc.). This makes state inconsistent, hard to test, and impossible to reset. This ticket centralizes state into a single `AppState` struct managed by Tauri's state system.

#### Affected Files and Line Ranges

- `src-tauri/src/state.rs` (or wherever `AppState` is defined) — expand to hold all state.
- `src-tauri/src/hw/touchpad.rs` — `GESTURE_STATE`, `TOUCHPAD_DEVICE_CACHE` thread-locals.
- `src-tauri/src/hw/display.rs` — display state statics.
- `src-tauri/src/hw/hotkeys.rs` — hotkey state statics.
- `src-tauri/src/hw/charging.rs` — charging state statics.
- All command handlers that currently read statics.

#### Root Cause Analysis

State was added incrementally per-module using `thread_local!` and `OnceLock` for convenience, without a unifying container. This means: (1) state cannot be injected for testing, (2) state lifetime is unclear (some statics persist across app restarts within a process), (3) there is no single place to observe or reset state, and (4) the Tauri `AppState` is misleadingly empty, suggesting no significant state exists.

#### Acceptance Criteria

- [ ] A single `AppState` struct is defined that holds all previously-static state as fields (each wrapped in `Mutex` or `RwLock` as appropriate).
- [ ] `AppState` is registered with Tauri via `app.manage(state)` and accessed in handlers via `State<'_, AppState>`.
- [ ] All `thread_local!` and `OnceLock` statics in the audited modules are removed and replaced with `AppState` access.
- [ ] For the touchpad raw-input callback (which cannot easily take a Tauri `State`), state is accessed via a single `OnceLock<Arc<AppState>>` set once at startup — documented as the sole exception.
- [ ] A `AppState::reset()` method is added for testing and for a future "reset settings" feature.
- [ ] Unit tests can construct an `AppState` directly and inject it, with no reliance on process-global statics.
- [ ] No behavior change in production (state values are preserved, just relocated).

#### Implementation Notes

- This is a large refactor; do it module-by-module to keep PRs reviewable (touchpad first, then display, then hotkeys, etc.).
- The raw-input callback exception is acceptable because `RegisterRawInputDevices` requires a `static` callback; use `OnceLock<Arc<AppState>>` set in `setup` and read with `Relaxed` ordering.
- Prefer `parking_lot::Mutex` (non-poisoning) over `std::sync::Mutex` to align with S3-001's recovery goals — but this is optional and can be a follow-up.

#### Testing Strategy

- **Unit tests**: construct `AppState`, mutate fields, assert isolation between test instances.
- **Integration test**: verify the raw-input callback still functions after the refactor.
- **Regression test**: run the full app and confirm all hardware features work identically to pre-refactor.

#### Dependencies

- S3-001 (panic recovery should be in place before this large refactor, so a mid-refactor panic doesn't abort).

---

### S3-003 — Introduce structured error types with thiserror; replace opaque anyhow→String

| Field | Value |
|-------|-------|
| **Ticket ID** | S3-003 |
| **Title** | Define typed error enums with thiserror; stop converting errors to opaque Strings |
| **Priority** | P1 |
| **Type** | Refactor |
| **Estimated Effort** | L |

#### Description

`thiserror` is declared in `Cargo.toml` but unused. All errors are converted to `anyhow::Error` then to `String` via `to_string()` or `format!`, losing error codes, categories, and the ability to match on error variants. This ticket introduces structured error enums for each domain (hardware, network, config) and propagates them typed.

#### Affected Files and Line Ranges

- `src-tauri/Cargo.toml` — `thiserror` dependency (already present).
- `src-tauri/src/hw/mod.rs` — define `HardwareError` enum.
- `src-tauri/src/hw/wifi.rs`, `display.rs`, `touchpad.rs`, etc. — replace `anyhow::Result` with typed `Result<T, HardwareError>`.
- `src-tauri/src/commands/*.rs` — map typed errors to Tauri response codes.

#### Root Cause Analysis

Using `anyhow` everywhere is convenient for prototyping but loses type information. When a command handler does `e.to_string()`, the frontend receives a raw string with no machine-readable error code, making error handling in the UI brittle (string matching). `thiserror` was added to the manifest but never used, suggesting the intent was always to type errors.

#### Acceptance Criteria

- [ ] A `HardwareError` enum is defined with `#[derive(thiserror::Error)]` and variants for each failure mode (e.g. `WmiQuery`, `Io`, `Hid`, `InvalidConfig`, `Timeout`, `NotSupported`).
- [ ] Each variant carries relevant context (e.g. `WmiQuery { source: wmi::Error, query: String }`).
- [ ] Hardware modules return `Result<T, HardwareError>` instead of `anyhow::Result`.
- [ ] Command handlers map `HardwareError` to a serializable error response with a `code` field (string enum) and `message` field, so the frontend can switch on `code`.
- [ ] `anyhow` is retained only at the top-level command boundary for ergonomic `?` propagation where the error type doesn't matter.
- [ ] Unit test: a WMI failure produces a `HardwareError::WmiQuery` that serializes with `code: "wmi_query"`.
- [ ] The frontend error handling is updated (coordinate with frontend team) to switch on `code` rather than string-matching messages.

#### Implementation Notes

- Define error enums per-domain to avoid a single mega-enum; a top-level `AppError` can wrap domain errors if needed.
- Use `#[error("...")]` for human-readable messages and `#[from]` for source conversion.
- Keep `anyhow` for application bootstrap where error types are not consumed programmatically.

#### Testing Strategy

- **Unit tests**: each error variant serializes correctly and carries context.
- **Integration test**: trigger a known failure (e.g. invalid WiFi SSID) and assert the frontend receives the correct `code`.

#### Dependencies

- None (can proceed in parallel with S3-002, but coordinate on state access patterns).

---

### S3-004 — Move blocking WMI/IGCL/PowerShell calls to `spawn_blocking`

| Field | Value |
|-------|-------|
| **Ticket ID** | S3-004 |
| **Title** | Wrap all blocking WMI/IGCL/PowerShell calls in `tokio::task::spawn_blocking` |
| **Priority** | P0 |
| **Type** | Bug / Performance |
| **Estimated Effort** | L |

#### Description

`adaptive_brightness_loop` (`src-tauri/src/hw/display.rs` ~lines 236–345) and `commands/system.rs` make blocking WMI/IGCL/PowerShell calls directly on the async runtime, starving the tokio worker threads. This causes the entire app to freeze during these calls. This ticket wraps all blocking calls in `spawn_blocking`.

#### Affected Files and Line Ranges

- `src-tauri/src/hw/display.rs` — `adaptive_brightness_loop` (~lines 236–345, the WMI/IGCL calls at ~255–265).
- `src-tauri/src/commands/system.rs` — blocking WMI calls.
- Any other async function making synchronous WMI/IGCL/PowerShell calls.

#### Root Cause Analysis

WMI (via `wmi` crate), IGCL (Intel Graphics Control Library), and `std::process::Command` for PowerShell are all synchronous, blocking operations. Calling them directly in an `async fn` running on tokio's runtime blocks the worker thread, preventing other tasks (including UI event handling) from progressing. The correct pattern is `spawn_blocking` which moves the work to a dedicated blocking thread pool.

#### Acceptance Criteria

- [ ] Every blocking WMI query is wrapped in `tokio::task::spawn_blocking(move || { ... })`.
- [ ] Every IGCL call is wrapped in `spawn_blocking`.
- [ ] Every `Command::new("powershell")` (or `cmd`) call in async context is wrapped in `spawn_blocking`.
- [ ] The `adaptive_brightness_loop` no longer blocks the runtime between iterations — the loop `await`s the `spawn_blocking` future.
- [ ] A code audit (grep for `wmi::`, `IGCL`, `Command::new` in `async fn`) confirms no remaining un-wrapped blocking calls.
- [ ] Manual test: during a brightness adjustment, the UI remains responsive (no freeze).
- [ ] Performance test: measure the runtime's idle task latency during a WMI call cycle; confirm it stays under 16ms (one frame).

#### Implementation Notes

- `spawn_blocking` has overhead (~10–50µs); for very high-frequency calls, consider a dedicated thread with a channel instead. For 2s pollers, `spawn_blocking` is fine.
- This overlaps with Sprint 4's WMI connection caching (P2) — coordinate so the cached connection is accessed inside `spawn_blocking`.
- Be careful with `Send` bounds: closures passed to `spawn_blocking` must be `Send`; WMI connection objects may not be — clone or recreate per call if needed.

#### Testing Strategy

- **Manual test**: trigger brightness changes and confirm UI responsiveness.
- **Profiling**: use `tokio-console` or tracing to confirm no long-running tasks on the worker pool.
- **Regression test**: confirm brightness adjustment still functions correctly after the change.

#### Dependencies

- S3-001 (panic recovery should be in place; a panic inside `spawn_blocking` should not crash the app).

---

### S3-005 — Replace `.lock().unwrap()` panic sites with poison-recovery helpers

| Field | Value |
|-------|-------|
| **Ticket ID** | S3-005 |
| **Title** | Audit and replace all `Mutex::lock().unwrap()` with poison-recovering accessors |
| **Priority** | P1 |
| **Type** | Refactor |
| **Estimated Effort** | M |

#### Description

Following S3-001's introduction of `lock_or_recover`, this ticket audits the entire codebase for `.lock().unwrap()` and `.lock().expect(...)` calls and replaces them with the recovery helper. The audit specifically calls out `commands/hardware.rs:42,63`, but a full sweep is needed.

#### Affected Files and Line Ranges

- `src-tauri/src/commands/hardware.rs` — lines 42, 63 (and any others).
- All `src-tauri/src/**/*.rs` files with `Mutex`/`RwLock` access.

#### Root Cause Analysis

`Mutex::lock()` returns `Result` because the mutex can be poisoned (a previous holder panicked). `.unwrap()` on this propagates the panic. With `panic = "unwind"` (S3-001), this is recoverable, but only if the code uses `unwrap_or_else(|e| e.into_inner())` rather than `unwrap()`.

#### Acceptance Criteria

- [ ] A `grep` for `\.lock\(\)\.unwrap\(\)` and `\.lock\(\)\.expect` across `src-tauri/src/` returns zero results (or each remaining instance is documented as intentional).
- [ ] All `RwLock` read/write lock sites are similarly audited and use `unwrap_or_else(|e| e.into_inner())`.
- [ ] A `clippy` lint or custom lint is configured (if feasible) to flag new `.lock().unwrap()` additions.
- [ ] Unit test: poison a mutex used in a command handler; confirm the handler recovers and returns a sensible default rather than panicking.

#### Implementation Notes

- If `parking_lot::Mutex` is adopted (non-poisoning), this ticket becomes a no-op — but the migration is larger and may be deferred. For this sprint, use the `lock_or_recover` helper.
- Document any intentional `.lock().unwrap()` (e.g. in `main` before threads start, where poison is impossible) with a comment.

#### Testing Strategy

- **Grep-based audit**: confirm zero unintended `.lock().unwrap()` calls.
- **Unit test**: poison-and-recover for at least one command handler.

#### Dependencies

- S3-001 (introduces the `lock_or_recover` helper).

---

### S3-006 — Introduce HAL trait abstractions for hardware operations

| Field | Value |
|-------|-------|
| **Ticket ID** | S3-006 |
| **Title** | Define HAL (Hardware Abstraction Layer) traits for swappable, mockable hardware ops |
| **Priority** | P2 |
| **Type** | Refactor |
| **Estimated Effort** | XL |

#### Description

Hardware operations are hardcoded to Win32/WMI/IOCTL calls with no trait abstraction. This makes the hardware layer untestable (no mocking) and non-portable. This ticket introduces HAL traits that decouple the business logic from the platform implementation, enabling dependency injection and testing.

#### Affected Files and Line Ranges

- `src-tauri/src/hw/mod.rs` — define traits.
- `src-tauri/src/hw/display.rs`, `touchpad.rs`, `wifi.rs`, `charging.rs`, `hotkeys.rs` — implement traits for Win32.
- `src-tauri/src/state.rs` — hold trait objects (boxed or generic).

#### Root Cause Analysis

Direct WMI/IOCTL calls in business logic couple the logic to the platform. There is no way to substitute a mock implementation for unit tests, so hardware logic is tested only manually. A HAL trait (e.g. `trait DisplayHardware { fn get_brightness(&self) -> Result<u8>; fn set_brightness(&self, v: u8) -> Result<()>; }`) allows a `MockDisplayHardware` in tests.

#### Acceptance Criteria

- [ ] HAL traits are defined for each hardware domain: `DisplayHardware`, `TouchpadHardware`, `WifiHardware`, `ChargingHardware`, `HotkeyHardware`.
- [ ] The Win32 implementations are moved into `impl DisplayHardware for Win32Display` (etc.).
- [ ] `AppState` holds `Box<dyn DisplayHardware>` (or `Arc<dyn ...>`) so the implementation is injectable.
- [ ] A `MockDisplayHardware` is implemented in `#[cfg(test)]` and used in at least 3 unit tests that previously had no hardware coverage.
- [ ] Production behavior is unchanged (the Win32 impl is wired by default).
- [ ] The traits are documented with the contract each method must uphold.

#### Implementation Notes

- This is a large refactor; scope it to one domain per PR (display first as a pilot).
- Use `Arc<dyn Trait>` for shared hardware access across tasks; `Box<dyn Trait>` if owned.
- Trait methods should return the structured errors from S3-003.
- This enables Sprint 4's testing and Sprint 8's edge-case coverage.

#### Testing Strategy

- **Unit tests** using mocks for each domain.
- **Regression test**: confirm production hardware behavior is identical.

#### Dependencies

- S3-002 (state centralization provides the injection point).
- S3-003 (typed errors are the trait return type).

---

## Sprint Exit Criteria

- [ ] All 6 tickets merged.
- [ ] `cargo check` and `cargo test` pass.
- [ ] The app survives a forced mutex-poison scenario (S3-001/S3-005).
- [ ] No blocking calls on the async runtime (S3-004, verified by profiling).
- [ ] At least one hardware domain has mock-based unit tests (S3-006 pilot).
- [ ] Error responses include machine-readable codes (S3-003).

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| State centralization (S3-002) is large and risky | Do it module-by-module; keep PRs small; full regression test after each module. |
| HAL traits (S3-006) over-abstract | Pilot with display only; do not refactor all domains in one sprint. |
| `spawn_blocking` overhead on hot paths | Only wrap genuinely blocking calls; use dedicated threads for high-frequency paths. |
| Removing `panic = "abort"` increases binary size | Acceptable for a desktop app; document the trade-off. |
