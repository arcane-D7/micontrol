# Sprint 4 — Performance Optimization

## Sprint Metadata

| Field | Value |
|-------|-------|
| **Sprint Name** | Performance Optimization |
| **Sprint Goal** | Eliminate WMI connection churn, blocking async calls, redundant IPC polling, and per-frame HID allocations |
| **Duration Estimate** | 2.5 weeks (12–13 working days) |
| **Priority** | P1 — Performance. Directly impacts UI responsiveness and battery life. |
| **Sprint Type** | Performance / Refactor |
| **Primary Owner** | Rust performance engineer |
| **Secondary Owner** | Frontend engineer (polling/IPC) |

## Sprint Goal Statement

The app currently creates and destroys 7 WMI connections every 2 seconds, fires 8 IPC calls per polling cycle, and clones HID preparsed data on every raw input frame (60–125 Hz). By the end of this sprint, WMI connections are cached and reused, all blocking calls run on `spawn_blocking`, the dual 2s pollers are consolidated, and HID preparsed data is allocated once per device. Measurable targets: WMI connection count drops from 7/cycle to 1 (cached), IPC calls per cycle drop from 8 to ≤3, and HID allocations drop to once-per-device.

---

## Background

Three critical performance findings: (P1) blocking WMI calls on the async runtime with no `spawn_blocking`, (P2) no WMI connection reuse — 7 connections created/destroyed every 2s, and (P3) dual 2s pollers firing 8 IPC calls per cycle. Three high findings: (P4) preparsed HID data cloned every raw input frame, (P5) adaptive brightness loop calls blocking WMI on async runtime, (P6) `useHardware` returns a new object every render causing full tree re-render every 2s.

Note: P1 and P5 overlap with Sprint 3's S3-004 (`spawn_blocking`). This sprint focuses on the connection caching and polling consolidation; coordinate with Sprint 3 to avoid duplicate work.

---

## Tickets

### S4-001 — Cache and reuse WMI connections

| Field | Value |
|-------|-------|
| **Ticket ID** | S4-001 |
| **Title** | Implement a WMI connection pool/cache to eliminate per-query connection churn |
| **Priority** | P0 |
| **Type** | Performance |
| **Estimated Effort** | L |

#### Description

The system polling path creates and destroys 7 WMI connections every 2 seconds (one per query: battery, brightness, charging, etc.). WMI connection setup involves COM initialization and WMI namespace binding, which is expensive (~10–50ms each). This ticket implements a cached, reusable WMI connection.

#### Affected Files and Line Ranges

- `src-tauri/src/commands/system.rs` — the polling path that creates 7 connections per cycle.
- `src-tauri/src/hw/display.rs` — `adaptive_brightness_loop` WMI calls (~lines 255–265).
- A new `src-tauri/src/hw/wmi_cache.rs` (or extend an existing util module).

#### Root Cause Analysis

Each hardware query independently calls `WMIConnection::new(COMLibrary::new()?)` (or equivalent), uses it once, and drops it. There is no shared connection. Over a 2s cycle with 7 queries, that's 7 COM init + WMI bind + teardown operations, consuming 70–350ms of CPU and producing visible UI jank.

#### Acceptance Criteria

- [ ] A `WmiCache` struct holds a single `WMIConnection` (or a small pool) initialized once and reused across queries.
- [ ] The cache is stored in `AppState` (per Sprint 3's S3-002) or a `OnceLock` if state centralization is not yet complete.
- [ ] All hardware query sites retrieve the cached connection instead of creating a new one.
- [ ] The connection is validated before reuse (a lightweight `SELECT * FROM Win32_ComputerSystem` or similar); if invalid, it is recreated transparently.
- [ ] Connection recreation is logged at `info!` level for diagnostics.
- [ ] Performance measurement: WMI connection creations per 2s cycle drop from 7 to ≤1 (only on cache miss/invalidation).
- [ ] Performance measurement: total CPU time per polling cycle drops by ≥60%.
- [ ] Manual test: run for 10 minutes; confirm no WMI-related freezes and stable connection count in logs.

#### Implementation Notes

- WMI connections are not `Send` in all configurations; if so, keep the cache in a `thread_local` or access it only from `spawn_blocking` (per S3-004).
- Consider a pool of 2–3 connections if queries run concurrently, but a single connection with serialized access is simpler and likely sufficient for 2s polling.
- Add a `WmiCache::invalidate()` method for error recovery.

#### Testing Strategy

- **Performance benchmark**: measure connection creations and CPU time before/after using a `criterion` benchmark or a manual timing harness.
- **Unit test**: mock the WMI connection; verify the cache returns the same instance across calls.
- **Soak test**: 10-minute run with connection-count logging.

#### Dependencies

- S3-004 (spawn_blocking) — the cached connection should be accessed inside `spawn_blocking` since WMI is blocking.

---

### S4-002 — Consolidate dual 2s pollers into a single batched IPC call

| Field | Value |
|-------|-------|
| **Ticket ID** | S4-002 |
| **Title** | Merge the two 2s hardware pollers into one batched Tauri command returning all state |
| **Priority** | P0 |
| **Type** | Performance |
| **Estimated Effort** | M |

#### Description

The frontend runs two separate 2s pollers (in `useHardware.ts` and elsewhere) that together fire 8 IPC calls per cycle (battery, brightness, volume, charging, etc.). Each IPC call crosses the Tauri bridge with serialization overhead. This ticket consolidates them into a single batched command.

#### Affected Files and Line Ranges

- `src/hooks/useHardware.ts` — the polling logic (~lines 371–496).
- `src-tauri/src/commands/system.rs` — individual query commands.
- A new `src-tauri/src/commands/system.rs::get_hardware_state_batch` command.

#### Root Cause Analysis

Each hardware property is queried via a separate Tauri command invocation. With 8 properties polled every 2s by two pollers, that's 16 IPC round-trips per cycle (8 per poller × 2 pollers). Each round-trip incurs serialization, deserialization, and bridge scheduling overhead (~1–5ms each), totaling 16–80ms of overhead per cycle.

#### Acceptance Criteria

- [ ] A single `get_hardware_state_batch` Tauri command is added that returns a `HardwareState` struct containing all polled properties (battery level, charging state, brightness, volume, etc.) in one response.
- [ ] The backend batches the underlying WMI/IOCTL queries using the cached connection (S4-001) within a single `spawn_blocking` call.
- [ ] The frontend's two pollers are merged into one 2s interval calling `get_hardware_state_batch`.
- [ ] IPC calls per cycle drop from 8 (or 16 with dual pollers) to 1.
- [ ] The `HardwareState` struct is typed on both sides (Rust + TypeScript via `ts-rs` or manual types).
- [ ] Manual test: hardware state updates every 2s with no regression in freshness.
- [ ] Performance measurement: frontend re-render count per cycle drops (coordinate with S4-004).

#### Implementation Notes

- If some properties change at different rates (e.g. battery slowly, volume frequently), consider a hybrid: batch the slow ones at 2s, keep fast ones event-driven. But for this sprint, a single batch is the target.
- Use `serde` for the `HardwareState` struct; generate TypeScript types with `ts-rs` if already in use, or define a matching interface manually.

#### Testing Strategy

- **Unit test**: `get_hardware_state_batch` returns all expected fields.
- **Manual test**: confirm all hardware state displays update correctly.
- **Performance measurement**: IPC call count per cycle (via Tauri devtools or logging).

#### Dependencies

- S4-001 (cached WMI connection makes the batched query efficient).

---

### S4-003 — Eliminate per-frame HID preparsed data allocation

| Field | Value |
|-------|-------|
| **Ticket ID** | S4-003 |
| **Title** | Cache HID preparsed data per device instead of cloning every raw input frame |
| **Priority** | P1 |
| **Type** | Performance |
| **Estimated Effort** | M |

#### Description

In `src-tauri/src/hw/touchpad.rs` (~lines 955–965), the preparsed HID data is cloned on every raw input frame (60–125 Hz). Preparsed data allocation and `HidP_GetCaps`/`HidP_GetValueCaps` calls are expensive. This data is device-static and should be allocated once per device and cached.

#### Affected Files and Line Ranges

- `src-tauri/src/hw/touchpad.rs` — the preparsed data handling (~lines 955–965) and the `GESTURE_STATE`/`TOUCHPAD_DEVICE_CACHE` access.

#### Root Cause Analysis

The code calls `GetPreparsedData` and processes the caps on each frame, or clones a cached preparsed buffer per frame. At 125 Hz, this is 125 allocations/second of a non-trivial buffer, plus the GC pressure. Preparsed data is tied to the device, not the frame, so it should be fetched once when the device is first seen and reused.

#### Acceptance Criteria

- [ ] Preparsed data is fetched once per device (keyed by `device_key`) and stored in `TOUCHPAD_DEVICE_CACHE` alongside the touchpad boolean.
- [ ] Per-frame processing reads the cached preparsed data by reference, with no clone.
- [ ] The caps (`y_max`, `x_max`, etc.) are read once and stored in `GESTURE_STATE` (already partially done via `caps_read`); verify this path is complete and not re-reading.
- [ ] Allocation count per second (measured via a debug allocator or `tracing`) drops to near-zero during steady-state touchpad input.
- [ ] No behavior change in gesture recognition.
- [ ] Manual test: touchpad gestures work identically; no input lag introduced.

#### Implementation Notes

- `HidP_GetPreparsedData` returns a handle that must be freed with `HidP_FreePreparsedData`. Cache the handle and free it on device removal or app exit.
- If the preparsed data is large, store it as `Arc<[u8]>` to allow cheap reference sharing.
- Ensure thread safety: the cache is accessed from the raw-input callback; use `RefCell` (thread-local, as currently) or move to `AppState` per S3-002.

#### Testing Strategy

- **Allocation profiling**: use a custom allocator or `tracing` span to count allocations before/after.
- **Manual test**: 1-minute touchpad usage; confirm no input lag and correct gesture behavior.

#### Dependencies

- None (independent), but coordinate with S3-002 if moving the cache to `AppState`.

---

### S4-004 — Stabilize `useHardware` return value to prevent full-tree re-renders

| Field | Value |
|-------|-------|
| **Ticket ID** | S4-004 |
| **Title** | Memoize `useHardware` return value; prevent new-object creation every render |
| **Priority** | P1 |
| **Type** | Performance |
| **Estimated Effort** | M |

#### Description

`useHardware.ts` returns a new object every render, causing React to see a new reference and re-render the entire consuming tree every 2s (every poll). This is wasteful when the underlying values haven't changed. This ticket memoizes the return value with referential stability.

#### Affected Files and Line Ranges

- `src/hooks/useHardware.ts` — the hook's return statement (~lines 371–496).

#### Root Cause Analysis

The hook likely returns an object literal `{ battery, brightness, volume, ... }` directly. Even if the values are unchanged, the object reference is new each render, so `useMemo`/`useCallback` consumers and `React.memo` children all re-render. At a 2s poll cadence, this means a full tree re-render every 2s regardless of whether anything changed.

#### Acceptance Criteria

- [ ] The hook's return value is wrapped in `useMemo` with dependencies on the actual state values (not the object itself).
- [ ] When no polled value has changed between cycles, the returned object reference is stable (same identity).
- [ ] Consumers wrapped in `React.memo` do not re-render when the hardware state is unchanged.
- [ ] A React DevTools profiler trace shows no re-renders of memoized children when values are unchanged.
- [ ] Manual test: hardware displays update when values change; no unnecessary re-renders when idle.
- [ ] Performance measurement: render count over 10s of idle polling drops from ~5 (every 2s) to ~1 (initial only).

#### Implementation Notes

- Use `useMemo(() => ({ battery, brightness, volume }), [battery, brightness, volume])`.
- If the hook returns functions (callbacks), wrap them in `useCallback` with appropriate deps.
- Consider splitting the hook into per-domain hooks (`useBattery`, `useBrightness`) so consumers only re-render when their domain changes — but this is a larger refactor; memoization is the sprint target.

#### Testing Strategy

- **React Profiler**: capture a trace during idle polling; confirm no re-renders of memoized children.
- **Unit test** (if React Testing Library is set up): assert referential stability of the return value across renders with unchanged deps.

#### Dependencies

- S4-002 (batched polling changes the data shape; coordinate the memoization deps).

---

### S4-005 — Move adaptive brightness loop's WMI calls to `spawn_blocking` with cached connection

| Field | Value |
|-------|-------|
| **Ticket ID** | S4-005 |
| **Title** | Refactor `adaptive_brightness_loop` to use cached WMI via `spawn_blocking` |
| **Priority** | P1 |
| **Type** | Performance |
| **Estimated Effort** | M |

#### Description

`adaptive_brightness_loop` (`src-tauri/src/hw/display.rs` ~lines 236–345) makes blocking WMI calls directly on the async runtime. This is the same issue as S3-004 but specifically scoped to the brightness loop, which runs continuously. This ticket ensures the loop uses the cached WMI connection (S4-001) inside `spawn_blocking`.

#### Affected Files and Line Ranges

- `src-tauri/src/hw/display.rs` — `adaptive_brightness_loop` (~lines 236–345, WMI calls at ~255–265).

#### Root Cause Analysis

The brightness loop queries ambient light / current brightness via WMI every few seconds. Each query blocks the tokio worker thread. Because the loop is long-running, this creates recurring runtime starvation. Combined with no connection caching, each iteration also pays connection setup cost.

#### Acceptance Criteria

- [ ] All WMI calls in `adaptive_brightness_loop` are wrapped in `spawn_blocking`.
- [ ] The loop uses the cached WMI connection from S4-001 (accessed inside the `spawn_blocking` closure).
- [ ] The loop's `sleep`/interval remains on the async runtime (only the WMI call is blocking).
- [ ] Manual test: brightness adaptation continues to function; UI remains responsive during adaptation.
- [ ] Profiling: no long-running tasks on the tokio worker pool during brightness adaptation.

#### Implementation Notes

- This may be fully covered by S3-004 + S4-001; if so, this ticket verifies and closes the brightness-loop-specific path. Coordinate to avoid duplicate work.
- If the loop uses IGCL (Intel Graphics) for brightness, ensure those calls are also in `spawn_blocking`.

#### Testing Strategy

- **Manual test**: cover/uncover the ambient light sensor; confirm brightness adapts smoothly with no UI freeze.
- **Profiling**: `tokio-console` trace during adaptation.

#### Dependencies

- S3-004 (spawn_blocking pattern).
- S4-001 (cached WMI connection).

---

## Sprint Exit Criteria

- [ ] All 5 tickets merged.
- [ ] `cargo check`, `cargo test`, and `npm run build` pass.
- [ ] Performance benchmarks show: WMI connections/cycle ≤1, IPC calls/cycle ≤3, HID allocations/frame ≈0, idle re-renders/10s ≈1.
- [ ] No UI freezes during hardware polling or brightness adaptation.
- [ ] No regression in hardware state freshness or accuracy.

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Cached WMI connection goes stale and returns errors | Validate before reuse; recreate transparently on failure. |
| Batched IPC loses per-property error granularity | Return per-field error status in the `HardwareState` struct. |
| HID cache lifetime management (handle leaks) | Free preparsed data on device removal; use RAII wrapper. |
| `useMemo` deps miss a value, causing stale UI | Include all returned values in deps; add a lint rule or code review checklist. |
