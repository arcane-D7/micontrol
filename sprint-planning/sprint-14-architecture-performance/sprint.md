# Sprint 14 — Architecture & Performance (P1)

## Sprint Metadata

| Field                 | Value                                                                                                                                                                      |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Sprint Name**       | Architecture & Performance                                                                                                                                                 |
| **Sprint Goal**       | Complete the HardwareResult migration, fix WMI caching, bundle local fonts, implement HMAC key rotation, and resolve all HIGH/MEDIUM architecture and performance findings |
| **Duration Estimate** | ~7 days                                                                                                                                                                    |
| **Priority**          | P1 — High priority, blocks GA readiness                                                                                                                                    |
| **Sprint Type**       | Multi-domain (Architecture, Performance, Security)                                                                                                                         |
| **Primary Owner**     | Backend engineer (Rust)                                                                                                                                                    |
| **Secondary Owner**   | Frontend engineer (React)                                                                                                                                                  |
| **Source**            | `docs/stability-report-2026-06-24-post-sprints.md`                                                                                                                         |
| **Depends On**        | Sprint 13                                                                                                                                                                  |

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **OBRIGATÓRIO: 100% dos tickets desta sprint devem ser concluídos. A sprint não será aceita como entregue se qualquer ticket permanecer incompleto.**
>
> **MANDATORY: 100% of the tickets in this sprint MUST be completed. The sprint will NOT be accepted as delivered if any ticket remains incomplete.**

Every ticket must pass its acceptance criteria AND the full health check suite (9/9) before the sprint commit is made. The HardwareResult migration (S14-001) is the largest ticket — it must cover ALL remaining modules, not just a subset.

---

## Sprint Goal Statement

The post-audit stability report identified 2 CRITICAL and 4 HIGH architecture findings, plus 2 HIGH and 8 MEDIUM performance findings. The most significant gap is the incomplete HardwareResult migration (only 3/16 modules done) from S11-001. This sprint completes the migration, fixes WMI caching inefficiencies, bundles fonts locally for privacy, and implements security hardening (HMAC rotation, nonce persistence, audit log integrity).

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

## Tickets

### S14-001 — Complete HardwareResult migration (all remaining modules)

| Field                | Value                                                                 |
| -------------------- | --------------------------------------------------------------------- |
| **Ticket ID**        | S14-001                                                               |
| **Title**            | Complete HardwareResult migration for all remaining `hw/*.rs` modules |
| **Priority**         | P1                                                                    |
| **Type**             | Architecture                                                          |
| **Estimated Effort** | XL                                                                    |
| **Source Finding**   | ARCH-001 (CRITICAL) + ARCH-006 (HIGH)                                 |
| **Regression of**    | S11-001 (incomplete — only 3/16 modules migrated)                     |

#### Context

S11-001 migrated only `fan.rs` and `wifi.rs` (and partially `battery.rs`) to `HardwareResult<T>`. 12+ modules still use `anyhow::Result`, creating inconsistent error handling across the hardware layer.

#### Modules to Migrate

The following modules must be migrated to `HardwareResult<T>`:

1. `hw/battery.rs` (partially done — complete it)
2. `hw/audio.rs`
3. `hw/charging.rs`
4. `hw/display.rs`
5. `hw/keyboard.rs`
6. `hw/touchpad.rs`
7. `hw/processes.rs`
8. `hw/ecram.rs`
9. `hw/hotkeys.rs`
10. `hw/iotservice.rs`
11. `hw/discovery.rs`
12. `hw/elev_bridge.rs`
13. Any other `hw/*.rs` module still using `anyhow::Result`

#### Acceptance Criteria

- [x] All `hw/*.rs` modules return `HardwareResult<T>` instead of `anyhow::Result`
- [x] `HardwareError` enum has appropriate variants for all error cases
- [x] No `anyhow::Result` remains in any `hw/*.rs` module (except `#[cfg(test)]` helpers if needed)
- [x] `cargo check` passes
- [x] `cargo clippy -- -D warnings` passes
- [x] `cargo test` passes (all existing tests + any new tests)
- [x] Error messages are structured and user-friendly
- [x] No regressions in functionality

---

### S14-002 — Fix dual hardware profile storage synchronization

| Field                | Value                                                 |
| -------------------- | ----------------------------------------------------- |
| **Ticket ID**        | S14-002                                               |
| **Title**            | Fix dual hardware profile storage synchronization gap |
| **Priority**         | P1                                                    |
| **Type**             | Architecture                                          |
| **Estimated Effort** | M                                                     |
| **Source Finding**   | ARCH-003 (HIGH)                                       |

#### Context

Hardware profiles are stored in both `discovery.rs` and `state.rs`, with no synchronization mechanism. Changes in one are not reflected in the other, leading to stale data.

#### Acceptance Criteria

- [x] Single source of truth for hardware profiles (either `discovery.rs` or `state.rs`, not both)
- [x] If both must exist, a synchronization function ensures consistency
- [x] Unit test verifies profile changes propagate correctly
- [x] No stale profile data after update
- [x] `cargo clippy -- -D warnings` passes

---

### S14-003 — Replace tokio "full" with explicit features

| Field                | Value                                                                |
| -------------------- | -------------------------------------------------------------------- |
| **Ticket ID**        | S14-003                                                              |
| **Title**            | Replace `tokio = { features = ["full"] }` with explicit feature list |
| **Priority**         | P1                                                                   |
| **Type**             | Architecture                                                         |
| **Estimated Effort** | XS                                                                   |
| **Source Finding**   | ARCH-004 (HIGH)                                                      |

#### Context

`Cargo.toml` uses `tokio` with `features = ["full"]`, which pulls in all features including unused ones (e.g., `signal`, `test-util`). This bloats compile times and binary size.

#### Acceptance Criteria

- [x] `tokio` features are explicitly listed (e.g., `["rt-multi-thread", "macros", "sync", "time", "fs", "io-util", "net"]`)
- [x] Only features actually used by the codebase are included
- [x] `cargo check` passes
- [x] `cargo test` passes
- [x] Binary size does not increase
- [x] `cargo clippy -- -D warnings` passes

---

### S14-004 — Wire or remove dead code functions

| Field                | Value                                                                 |
| -------------------- | --------------------------------------------------------------------- |
| **Ticket ID**        | S14-004                                                               |
| **Title**            | Wire dead code functions (`get_profile`, `invalidate`) or remove them |
| **Priority**         | P1                                                                    |
| **Type**             | Architecture                                                          |
| **Estimated Effort** | XS                                                                    |
| **Source Finding**   | ARCH-005 (HIGH)                                                       |

#### Context

Key architectural functions in `state.rs`, `panic.rs`, and `wmi_cache.rs` are defined but never called. Dead code increases maintenance burden and confuses readers.

#### Acceptance Criteria

- [x] Each dead code function is either wired into the codebase or removed
- [x] If removed, no references remain
- [x] If wired, unit test covers the new call path
- [x] `cargo clippy -- -D warnings` passes with no dead code warnings
- [x] `cargo check` passes

---

### S14-005 — Cache static WMI data (BatteryStaticData, CPU info)

| Field                | Value                                                 |
| -------------------- | ----------------------------------------------------- |
| **Ticket ID**        | S14-005                                               |
| **Title**            | Cache static WMI data using `OnceLock` or `LazyLock`  |
| **Priority**         | P1                                                    |
| **Type**             | Performance                                           |
| **Estimated Effort** | S                                                     |
| **Source Finding**   | PERF-001 (HIGH) + PERF-002 (MEDIUM) + PERF-013 (HIGH) |

#### Context

`battery.rs` issues 3 sequential WMI queries on every poll, even though `BatteryStaticData` never changes at runtime. `processes.rs` issues 2 sequential WMI queries on every call. Static data should be cached.

#### Acceptance Criteria

- [x] `BatteryStaticData` (design capacity, design voltage, chemistry, etc.) is cached in `OnceLock` or `LazyLock`
- [x] `NumberOfLogicalProcessors` is cached in `OnceLock`
- [x] Dynamic data (current capacity, charge rate) is still queried on every poll
- [x] `battery.rs` issues at most 1 WMI query per poll (down from 3)
- [x] `processes.rs` issues at most 1 WMI query per call (down from 2)
- [x] Unit test verifies cache is populated on first call and reused on second
- [x] `cargo clippy -- -D warnings` passes

---

### S14-006 — Bundle Google Fonts locally

| Field                | Value                                                     |
| -------------------- | --------------------------------------------------------- |
| **Ticket ID**        | S14-006                                                   |
| **Title**            | Replace Google Fonts `@import` with locally bundled fonts |
| **Priority**         | P1                                                        |
| **Type**             | Performance / Privacy                                     |
| **Estimated Effort** | S                                                         |
| **Source Finding**   | PERF-005 / PERF-027 (MEDIUM) + PROD-007 (HIGH)            |

#### Context

`globals.css` imports Google Fonts via `@import url(...)`. This sends the user's IP address to Google on every app launch (privacy violation) and blocks rendering (performance issue).

#### Acceptance Criteria

- [x] Google Fonts `@import` removed from `globals.css`
- [x] Font files (`.woff2`) are bundled locally in `src/assets/fonts/` or `public/fonts/`
- [x] `@font-face` declarations reference local files
- [x] No external font requests on app launch
- [x] `npm run build` passes
- [x] Visual appearance is unchanged (same font family and weights)
- [x] `npm run lint` passes

---

### S14-007 — Add `React.memo` to sidebar in MainWindow

| Field                | Value                                        |
| -------------------- | -------------------------------------------- |
| **Ticket ID**        | S14-007                                      |
| **Title**            | Memoize sidebar navigation with `React.memo` |
| **Priority**         | P1                                           |
| **Type**             | Performance                                  |
| **Estimated Effort** | XS                                           |
| **Source Finding**   | PERF-017 (MEDIUM)                            |

#### Context

`MainWindow.tsx` sidebar re-renders on every state change, even though the navigation items are static.

#### Acceptance Criteria

- [x] Sidebar component is wrapped in `React.memo`
- [x] Sidebar does not re-render when unrelated state changes
- [x] `npx tsc --noEmit` passes
- [x] `npm run lint` passes
- [x] `npm run build` passes
- [x] No visual regressions

---

### S14-008 — Add `manualChunks` to Vite config

| Field                | Value                                                |
| -------------------- | ---------------------------------------------------- |
| **Ticket ID**        | S14-008                                              |
| **Title**            | Add `manualChunks` configuration to `vite.config.ts` |
| **Priority**         | P1                                                   |
| **Type**             | Performance                                          |
| **Estimated Effort** | XS                                                   |
| **Source Finding**   | PERF-007 (MEDIUM)                                    |

#### Context

Vite's default chunking may create oversized bundles. Manual chunking can improve initial load time by separating vendor code.

#### Acceptance Criteria

- [x] `build.rollupOptions.output.manualChunks` is configured in `vite.config.ts`
- [x] Vendor libraries (React, Tauri API, etc.) are in separate chunks
- [x] `npm run build` passes
- [x] Bundle analysis shows improved chunking
- [x] No runtime errors from chunk splitting

---

### S14-009 — Fix WMI cache invalidation to only trigger on connection errors

| Field                | Value                                                                           |
| -------------------- | ------------------------------------------------------------------------------- |
| **Ticket ID**        | S14-009                                                                         |
| **Title**            | Fix WMI cache invalidation to only trigger on connection errors, not all errors |
| **Priority**         | P1                                                                              |
| **Type**             | Architecture                                                                    |
| **Estimated Effort** | S                                                                               |
| **Source Finding**   | ARCH-008 (MEDIUM)                                                               |

#### Context

`wmi_cache.rs` invalidates the cache on ANY query failure, including transient errors (e.g., a single WMI class not found). This causes unnecessary reconnections.

#### Acceptance Criteria

- [x] Cache invalidation only triggers on `WMIConnectionError` (or equivalent connection-level error)
- [x] Transient query errors (class not found, property not found) do NOT invalidate the cache
- [x] Different error types are distinguished (ARCH-008)
- [x] Unit test verifies cache is NOT invalidated on transient error
- [x] Unit test verifies cache IS invalidated on connection error
- [x] `cargo clippy -- -D warnings` passes

---

### S14-010 — Add HMAC key rotation mechanism

| Field                | Value                                 |
| -------------------- | ------------------------------------- |
| **Ticket ID**        | S14-010                               |
| **Title**            | Implement HMAC key rotation mechanism |
| **Priority**         | P1                                    |
| **Type**             | Security                              |
| **Estimated Effort** | M                                     |
| **Source Finding**   | SEC-004 (HIGH, CWE-778)               |

#### Context

The HMAC key used for IPC authentication lives forever — there is no rotation mechanism. If the key is compromised, there is no way to rotate it without a full reinstall.

#### Acceptance Criteria

- [x] HMAC key has a configurable rotation period (default: 30 days)
- [x] Key rotation generates a new key and stores it in the keyring
- [x] Old key is accepted for a grace period (default: 7 days) during rotation
- [x] Key rotation is logged
- [x] Unit test verifies key rotation produces a new key
- [x] Unit test verifies old key is accepted during grace period
- [x] `cargo clippy -- -D warnings` passes

---

### S14-011 — Persist nonces with TTL for replay protection

| Field                | Value                                           |
| -------------------- | ----------------------------------------------- |
| **Ticket ID**        | S14-011                                         |
| **Title**            | Persist nonces with TTL across process restarts |
| **Priority**         | P1                                              |
| **Type**             | Security                                        |
| **Estimated Effort** | M                                               |
| **Source Finding**   | SEC-005 (HIGH, CWE-778)                         |
| **Regression of**    | S10-002 (incomplete — nonces not persisted)     |

#### Context

Nonce anti-replay only lasts for the process lifetime. If the process restarts, the nonce set is cleared, allowing replay of captured messages.

#### Acceptance Criteria

- [x] Nonces are persisted to disk (e.g., in a nonce store file)
- [x] Nonces have a TTL (default: 5 minutes) and are expired automatically
- [x] On process restart, persisted nonces are loaded
- [x] Expired nonces are purged on load
- [x] Unit test verifies nonce is rejected after process restart
- [x] Unit test verifies expired nonce is purged
- [x] `cargo clippy -- -D warnings` passes

---

### S14-012 — Add rate limiting to IoTService IPC writes

| Field                | Value                                      |
| -------------------- | ------------------------------------------ |
| **Ticket ID**        | S14-012                                    |
| **Title**            | Add rate limiting to IoTService IPC writes |
| **Priority**         | P1                                         |
| **Type**             | Security                                   |
| **Estimated Effort** | XS                                         |
| **Source Finding**   | SEC-016 (MEDIUM)                           |

#### Context

IoTService IPC writes are not rate-limited. A rogue process could flood the pipe, causing DoS.

#### Acceptance Criteria

- [x] IPC writes are rate-limited (e.g., max 100 writes/second)
- [x] Rate limit is configurable
- [x] Exceeded rate limit returns a clear error
- [x] Unit test verifies rate limiting triggers
- [x] `cargo clippy -- -D warnings` passes

---

### S14-013 — Curate Clippy lints instead of `all = "warn"`

| Field                | Value                                                          |
| -------------------- | -------------------------------------------------------------- |
| **Ticket ID**        | S14-013                                                        |
| **Title**            | Replace `clippy::all = "warn"` with curated lint configuration |
| **Priority**         | P1                                                             |
| **Type**             | Architecture                                                   |
| **Estimated Effort** | XS                                                             |
| **Source Finding**   | ARCH-007 (MEDIUM)                                              |

#### Context

`Cargo.toml` sets all Clippy lints to `warn`, which is overly aggressive. Some lints (e.g., `needless_pass_by_value`) may not be actionable and create noise.

#### Acceptance Criteria

- [x] `Cargo.toml` or `clippy.toml` has a curated set of lints
- [x] Actionable lints are set to `deny`
- [x] Non-actionable or opinionated lints are set to `allow` with a comment explaining why
- [x] `cargo clippy -- -D warnings` passes with 0 warnings
- [x] No new clippy warnings introduced

---

### S14-014 — Add hardware profile JSON integrity check

| Field                | Value                                                    |
| -------------------- | -------------------------------------------------------- |
| **Ticket ID**        | S14-014                                                  |
| **Title**            | Add integrity check to hardware profile JSON persistence |
| **Priority**         | P1                                                       |
| **Type**             | Security                                                 |
| **Estimated Effort** | S                                                        |
| **Source Finding**   | SEC-014 (MEDIUM)                                         |

#### Context

Hardware profile JSON is persisted to disk without integrity check. An attacker could modify the profile to inject malicious configuration.

#### Acceptance Criteria

- [x] Hardware profile JSON is signed with HMAC before persistence
- [x] On load, HMAC is verified before deserialization
- [x] Tampered profile is rejected with a clear error and falls back to discovery
- [x] Unit test verifies tampered profile is detected
- [x] `cargo clippy -- -D warnings` passes

---

### S14-015 — Fix retry.rs blocking `thread::sleep` in `spawn_blocking`

| Field                | Value                                                           |
| -------------------- | --------------------------------------------------------------- |
| **Ticket ID**        | S14-015                                                         |
| **Title**            | Replace blocking `thread::sleep` in `retry.rs` with async sleep |
| **Priority**         | P1                                                              |
| **Type**             | Performance                                                     |
| **Estimated Effort** | XS                                                              |
| **Source Finding**   | PERF-020 (MEDIUM)                                               |

#### Context

`retry.rs` uses `thread::sleep` inside a `spawn_blocking` context, which blocks the blocking thread pool. This can cause thread pool starvation under load.

#### Acceptance Criteria

- [x] `thread::sleep` replaced with `tokio::time::sleep` (or equivalent async sleep)
- [x] Retry function is async or properly yields to the runtime
- [x] Unit test verifies retry still works correctly
- [x] `cargo clippy -- -D warnings` passes

---

## Sprint Completion Checklist

Before committing the sprint, verify ALL of the following:

- [x] All 15 tickets have their acceptance criteria fully met
- [x] `cargo fmt --manifest-path src-tauri/Cargo.toml --check` passes
- [x] `cargo check --manifest-path src-tauri/Cargo.toml` passes
- [x] `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` passes (0 warnings)
- [x] `cargo test --manifest-path src-tauri/Cargo.toml` passes (all tests green)
- [x] `npx tsc --noEmit` passes (0 errors)
- [x] `npm run lint` passes (0 warnings)
- [x] `npm run format:check` passes
- [x] `npm run build` passes
- [x] `npm run version:check` passes
- [x] ALL `hw/*.rs` modules use `HardwareResult<T>` (no `anyhow::Result` in hw layer)
- [x] No Google Fonts `@import` in CSS
- [x] No `tokio` `features = ["full"]` in Cargo.toml
- [x] No dead code warnings from clippy
- [x] Sprint commit message follows format: `feat(sprint-14): <summary>`

---

## Commit Message Template

```
feat(sprint-14): architecture and performance improvements

S14-001: Complete HardwareResult migration (all remaining modules)
S14-002: Fix dual hardware profile storage synchronization
S14-003: Replace tokio "full" with explicit features
S14-004: Wire or remove dead code functions
S14-005: Cache static WMI data (BatteryStaticData, CPU info)
S14-006: Bundle Google Fonts locally
S14-007: Add React.memo to sidebar in MainWindow
S14-008: Add manualChunks to Vite config
S14-009: Fix WMI cache invalidation to only trigger on connection errors
S14-010: Add HMAC key rotation mechanism
S14-011: Persist nonces with TTL for replay protection
S14-012: Add rate limiting to IoTService IPC writes
S14-013: Curate Clippy lints instead of all = "warn"
S14-014: Add hardware profile JSON integrity check
S14-015: Fix retry.rs blocking thread::sleep in spawn_blocking
```
