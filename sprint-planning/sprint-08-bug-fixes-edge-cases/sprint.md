# Sprint 8 — Remaining Bug Fixes & Edge Cases

## Sprint Metadata

| Field | Value |
|-------|-------|
| **Sprint Name** | Remaining Bug Fixes & Edge Cases |
| **Sprint Goal** | Resolve all remaining medium/low-severity Rust and frontend bugs from the audit |
| **Duration Estimate** | 2.5 weeks (12–13 working days) |
| **Priority** | P2 — Cleanup and hardening after critical sprints. |
| **Sprint Type** | Bug Fix |
| **Primary Owner** | Rust backend engineer |
| **Secondary Owner** | Frontend engineer (for any frontend edge cases) |

## Sprint Goal Statement

After the critical, security, architecture, performance, frontend, and DevOps sprints, a set of medium and low-severity bugs remain across the Rust backend (battery cache race, unconfirmed IoT message types, hotkey remap race, ECRAM validation gaps, IoTService response/timeout issues, charging response handling) and frontend. This sprint closes all remaining audit findings so the audit is fully resolved. By the end, every audit finding has a corresponding fix or documented acceptance.

---

## Background

This sprint addresses the "long tail" of audit findings: B1–B10 (additional Rust bugs) and any frontend edge cases not covered in Sprint 5. These are individually lower-severity but collectively important for robustness. Several involve input validation and race conditions that could become exploitable under specific conditions.

---

## Tickets

### S8-001 — Fix cache clear/probe race in battery.rs

| Field | Value |
|-------|-------|
| **Ticket ID** | S8-001 |
| **Title** | Eliminate the cache clear/probe race in battery status caching |
| **Priority** | P2 |
| **Type** | Bug |
| **Estimated Effort** | S |

#### Description

In `src-tauri/src/hw/battery.rs` (~lines 155–175), there is a race between cache clearing and cache probing: one path clears the cache while another probes (reads) it, leading to inconsistent or stale battery readings.

#### Affected Files and Line Ranges

- `src-tauri/src/hw/battery.rs` — cache clear/probe (~lines 155–175).

#### Root Cause Analysis

The cache is likely a `Mutex<Option<BatteryState>>` (or similar) where a "clear" operation (setting to `None`) and a "probe" operation (reading and possibly repopulating) are not atomic relative to each other. A probe that reads `None` (just cleared) may repopulate with stale data, or a clear that happens mid-probe may leave the cache in an inconsistent state.

#### Acceptance Criteria

- [ ] The clear and probe operations are serialized under the same lock (or use a single atomic operation) so no interleaving produces inconsistency.
- [ ] A probe that finds the cache empty repopulates it atomically (read-through with lock held).
- [ ] A clear invalidates the cache such that the next probe repopulates fresh.
- [ ] Unit test: spawn two threads, one clearing and one probing in a tight loop; assert no panic and no stale-value read after a configurable freshness threshold.
- [ ] Manual test: battery readings remain consistent during rapid charge/discharge transitions.

#### Implementation Notes

- Hold the lock for the duration of the read-through (probe + repopulate), or use a `RwLock` where clears take a write lock and probes take a read lock with upgrade.
- Consider a timestamp-based freshness check: a probe returns cached data only if it's younger than `MAX_AGE`; otherwise it repopulates.

#### Testing Strategy

- **Concurrency unit test**: multi-thread clear/probe loop.
- **Manual test**: battery display stability.

#### Dependencies

- None.

---

### S8-002 — Confirm or reject unconfirmed IoT message types 0x5001/0x5002

| Field | Value |
|-------|-------|
| **Ticket ID** | S8-002 |
| **Title** | Validate and document IoT message types 0x5001/0x5002 in iotservice.rs |
| **Priority** | P1 |
| **Type** | Bug |
| **Estimated Effort** | M |

#### Description

In `src-tauri/src/hw/iotservice.rs` (~lines 560–580), message types `0x5001` and `0x5002` are handled but their semantics are unconfirmed — they may be unsupported, deprecated, or misinterpreted. Handling unknown message types can lead to incorrect behavior or security issues.

#### Affected Files and Line Ranges

- `src-tauri/src/hw/iotservice.rs` — message type handling (~lines 560–580).

#### Root Cause Analysis

The message types were likely reverse-engineered or copied from a reference without confirmation of their meaning. The code handles them as if their semantics are known, but if they're actually different (or unsupported), the handling is wrong. This is especially risky for an IoT service that may receive messages from untrusted sources.

#### Acceptance Criteria

- [ ] The semantics of `0x5001` and `0x5002` are confirmed via protocol documentation, vendor specs, or traffic capture analysis.
- [ ] If confirmed valid, the handling is documented with a comment citing the source.
- [ ] If unconfirmable or invalid, the message types are rejected (logged and dropped) rather than handled speculatively.
- [ ] A default/unknown message-type handler is added that logs and drops unrecognized types (fail-closed).
- [ ] Unit test: sending `0x5001`/`0x5002` produces the confirmed behavior (or rejection).
- [ ] Unit test: sending an unknown type (e.g. `0x9999`) is logged and dropped.

#### Implementation Notes

- If protocol docs are unavailable, capture traffic with the real IoT service and correlate the message types with observed behavior.
- Fail-closed (reject unknown) is the safe default for a service that may receive untrusted input.
- Document the investigation findings in a comment and/or `docs/iot-protocol.md`.

#### Testing Strategy

- **Unit tests** for confirmed/rejected/unknown types.
- **Traffic capture** (if needed) to confirm semantics.

#### Dependencies

- None.

---

### S8-003 — Fix hotkey remap state race

| Field | Value |
|-------|-------|
| **Ticket ID** | S8-003 |
| **Title** | Serialize hotkey remap state transitions to prevent race |
| **Priority** | P2 |
| **Type** | Bug |
| **Estimated Effort** | S |

#### Description

In `src-tauri/src/hw/hotkeys.rs` (~lines 1240–1280), the remap state (when a user is reassigning a hotkey) has a race: the state can be read as "remapping" by one path while another path clears it, leading to a missed or incorrect remap.

#### Affected Files and Line Ranges

- `src-tauri/src/hw/hotkeys.rs` — remap state (~lines 1240–1280).

#### Root Cause Analysis

The remap state is likely a flag (e.g. `AtomicBool` or `Mutex<bool>`) that is set when remapping begins and cleared on completion/cancel. If the hotkey input handler reads the flag and proceeds, but a concurrent cancel clears it mid-handling, the captured key may be applied incorrectly or lost.

#### Acceptance Criteria

- [ ] The remap state transition (begin → capture → commit/cancel) is atomic under a single lock.
- [ ] A cancel during capture cleanly aborts without applying a partial key.
- [ ] Unit test: simulate concurrent begin/cancel/capture; assert no partial or incorrect remaps.
- [ ] Manual test: start a remap, press a key, confirm it applies; start a remap, cancel, confirm no key is captured.

#### Implementation Notes

- Use a `Mutex<RemapState>` where `RemapState` is an enum (`Idle`, `AwaitingKey { target_action }`) and all transitions go through the lock.
- Avoid holding the lock during the actual key capture if it blocks; instead, set the state and let the input handler commit.

#### Testing Strategy

- **Concurrency unit test**: begin/cancel/capture interleaving.
- **Manual test**: remap UX.

#### Dependencies

- None.

---

### S8-004 — Make WMI debounce per-key in hotkeys.rs

| Field | Value |
|-------|-------|
| **Ticket ID** | S8-004 |
| **Title** | Replace global WMI debounce with per-hotkey debounce |
| **Priority** | P2 |
| **Type** | Bug |
| **Estimated Effort** | S |

#### Description

In `src-tauri/src/hw/hotkeys.rs` (~line 1380), the WMI debounce is global — a single debounce timer applies to all hotkeys. This means firing one hotkey suppresses others within the debounce window, causing missed inputs when multiple hotkeys fire in quick succession.

#### Affected Files and Line Ranges

- `src-tauri/src/hw/hotkeys.rs` — WMI debounce (~line 1380).

#### Root Cause Analysis

A global debounce timestamp is checked before processing any hotkey's WMI action. If hotkey A fires and sets the debounce, hotkey B fired within the window is dropped. The debounce should be per-hotkey (per action or per key) so independent hotkeys don't suppress each other.

#### Acceptance Criteria

- [ ] The debounce is tracked per hotkey key (or per action), not globally.
- [ ] Firing hotkey A does not suppress hotkey B within the debounce window.
- [ ] Repeated firing of the same hotkey within the debounce window is still suppressed (debounce purpose preserved).
- [ ] Unit test: fire two different hotkeys within the debounce window; assert both process.
- [ ] Unit test: fire the same hotkey twice within the window; assert the second is debounced.

#### Implementation Notes

- Use a `HashMap<KeyCode, Instant>` (or `HashMap<ActionId, Instant>`) of last-fired timestamps, guarded by a lock.
- Clean up stale entries periodically to avoid unbounded growth.

#### Testing Strategy

- **Unit tests** for per-key debounce behavior.

#### Dependencies

- None.

---

### S8-005 — Validate bytes_returned and ERAM index in ecram.rs

| Field | Value |
|-------|-------|
| **Ticket ID** | S8-005 |
| **Title** | Add bounds validation for bytes_returned and ERAM index in EC RAM access |
| **Priority** | P1 |
| **Type** | Bug / Security |
| **Estimated Effort** | M |

#### Description

In `src-tauri/src/hw/ecram.rs`, two validation gaps exist: (B5) no `bytes_returned` validation at ~lines 330–345, and (B6) an unvalidated ERAM index at ~line 395. These can lead to out-of-bounds reads/writes to EC RAM, potentially causing hardware malfunction or undefined behavior.

#### Affected Files and Line Ranges

- `src-tauri/src/hw/ecram.rs` — `bytes_returned` check (~lines 330–345), ERAM index validation (~line 395).

#### Root Cause Analysis

(B5): The EC RAM read returns a `bytes_returned` count that is trusted without checking it against the expected/buffer size. A short read (fewer bytes than expected) is processed as if complete, reading uninitialized buffer data. (B6): An ERAM index is used without bounds checking against the EC's address space, allowing an out-of-range index to be sent to the EC, which may cause unpredictable hardware behavior.

#### Acceptance Criteria

- [ ] `bytes_returned` is validated against the expected read size; if it's less, the function returns an error (or reads only the valid bytes) and logs a warning.
- [ ] The ERAM index is validated against a known maximum (e.g. `ECRAM_MAX_INDEX`, documented from the EC datasheet); out-of-range indices return an error.
- [ ] No `unsafe` pointer arithmetic uses an unvalidated length or index.
- [ ] Unit test: a short read returns an error, not partial/garbage data.
- [ ] Unit test: an out-of-range index is rejected.
- [ ] Manual test: EC RAM reads/writes function correctly for valid indices.

#### Implementation Notes

- Define `ECRAM_MAX_INDEX` as a `const` from the EC datasheet; if unknown, use a conservative bound and document the assumption.
- For `bytes_returned`, compare against the requested size; on mismatch, log and return `Err(HardwareError::Io)` (per S3-003).
- This is especially important because EC RAM writes can affect hardware state (fan, power) — see S5 (ECRAM raw-write risk) in the security audit, addressed in Sprint 2's scope consideration; this ticket handles the validation aspect.

#### Testing Strategy

- **Unit tests** for short-read and out-of-range-index rejection.
- **Manual test**: valid EC RAM operations.

#### Dependencies

- S3-003 (typed errors for the return type) — optional, can use `anyhow` if not yet complete.

---

### S8-006 — Add response authentication and enforce timeout in iotservice.rs

| Field | Value |
|-------|-------|
| **Ticket ID** | S8-006 |
| **Title** | Authenticate IoTService responses and enforce the request timeout |
| **Priority** | P1 |
| **Type** | Bug / Security |
| **Estimated Effort** | M |

#### Description

In `src-tauri/src/hw/iotservice.rs`, three issues: (B8) no response authentication at ~line 490, (B9) timeout not actually enforced at ~lines 430–450, and (B10) no response read for charging at `charging.rs:100-110`. Unauthenticated responses can be spoofed; an unenforced timeout can hang indefinitely; a missing response read can leave the service in an inconsistent state.

#### Affected Files and Line Ranges

- `src-tauri/src/hw/iotservice.rs` — response handling (~line 490), timeout (~lines 430–450).
- `src-tauri/src/hw/charging.rs` — response read (~lines 100–110).

#### Root Cause Analysis

(B8): Responses from the IoT service are trusted without verifying they correspond to the request (no nonce/sequence match) and without integrity protection. A spoofed response could inject false state. (B9): A timeout value is set but not actually enforced — the read waits indefinitely, so a hung service blocks the caller. (B10): The charging path sends a command but doesn't read the response, so the service's reply buffer may fill or the command's success is unverified.

#### Acceptance Criteria

- [ ] Each request includes a nonce/sequence number; the response must match it or be rejected.
- [ ] Responses include an HMAC or checksum (reuse the mechanism from S2-001 if applicable, or a simpler shared-secret HMAC).
- [ ] The timeout is enforced using `tokio::time::timeout` (or a synchronous equivalent) wrapping the response read; on timeout, the function returns `Err(Timeout)`.
- [ ] The charging path reads and validates the response (or explicitly documents why a response is not expected).
- [ ] Unit test: a spoofed response (wrong nonce) is rejected.
- [ ] Unit test: a hung service causes a timeout error after the configured duration.
- [ ] Unit test: the charging command's response is read and validated.

#### Implementation Notes

- If the IoT service protocol doesn't support nonces/HMAC natively, layer them in the miPC wrapper or document the limitation and at minimum enforce the timeout.
- Use `tokio::time::timeout` for async paths; for synchronous paths, use a thread with a join timeout or `WaitForSingleObject` with a timeout on Windows.
- For B10, if the charging command genuinely expects no response, document it and ensure the send buffer is flushed; otherwise add the read.

#### Testing Strategy

- **Unit tests** for nonce mismatch, timeout, and charging response.
- **Manual test**: charging state updates correctly; no hangs.

#### Dependencies

- S2-001 (HMAC mechanism, if reused) — optional.

---

### S8-007 — Fix unchecked pointer cast in hotkeys.rs

| Field | Value |
|-------|-------|
| **Ticket ID** | S8-007 |
| **Title** | Validate pointer before cast in hotkeys.rs:1620 |
| **Priority** | P2 |
| **Type** | Bug |
| **Estimated Effort** | S |

#### Description

In `src-tauri/src/hw/hotkeys.rs` (~line 1620), a pointer is cast without a null check, which can cause a null-pointer dereference if the source returns null.

#### Affected Files and Line Ranges

- `src-tauri/src/hw/hotkeys.rs` — pointer cast (~line 1620).

#### Root Cause Analysis

A Win32 API call returns a pointer that is cast directly (e.g. `&*(ptr as *const T)`) without checking for null. If the API returns null on failure, the dereference is UB.

#### Acceptance Criteria

- [ ] The pointer is checked for null before the cast/dereference.
- [ ] On null, the function returns an error or default (not a crash).
- [ ] Unit test: simulate a null return; assert graceful handling.

#### Implementation Notes

- Use `if ptr.is_null() { return Err(...); }` before the cast.
- If the API's failure mode is documented, handle it specifically.

#### Testing Strategy

- **Unit test**: null-pointer handling.

#### Dependencies

- None.

---

### S8-008 — Audit and fix any remaining frontend edge cases

| Field | Value |
|-------|-------|
| **Ticket ID** | S8-008 |
| **Title** | Sweep frontend for remaining unhandled error states and edge cases |
| **Priority** | P2 |
| **Type** | Bug |
| **Estimated Effort** | M |

#### Description

After Sprint 5's targeted frontend fixes, a sweep for remaining edge cases: unhandled `invoke` rejections, missing loading states, unguarded `JSON.parse`, and any components not covered by the Sprint 5 tickets.

#### Affected Files and Line Ranges

- `src/**/*.tsx` and `src/**/*.ts` — all frontend components and hooks.

#### Root Cause Analysis

The audit focused on the most impactful frontend bugs (Sprint 5); a broader sweep is needed to catch lower-severity issues like unhandled promise rejections, missing null checks, and inconsistent loading/error states.

#### Acceptance Criteria

- [ ] A grep for `invoke(` confirms every call site has a `.catch()` or `try/catch` handler.
- [ ] A grep for `JSON.parse(` confirms every call is wrapped in `try/catch`.
- [ ] Components with async data show loading states (not blank or broken layouts).
- [ ] No unhandled promise rejections in the console during a 10-minute active-use session.
- [ ] A checklist of reviewed components is documented.
- [ ] Manual test: 10-minute active-use session with console monitoring; no unhandled errors.

#### Implementation Notes

- Use `eslint`'s `no-floating-promises` and `no-unsafe-optional-chaining` rules (from S7-002) to catch many of these automatically.
- Prioritize components handling user input and hardware commands.

#### Testing Strategy

- **Automated**: eslint rules catch floating promises and unsafe optional chaining.
- **Manual**: active-use console audit.

#### Dependencies

- S7-002 (eslint config enables the catching rules).

---

## Sprint Exit Criteria

- [ ] All 8 tickets merged.
- [ ] `cargo check`, `cargo test`, `cargo clippy -D warnings`, `npm run build`, `npm run lint` all pass.
- [ ] Every audit finding (across all 8 sprints) has a corresponding fix or documented acceptance.
- [ ] No unhandled promise rejections or console errors during a 30-minute active-use soak test.
- [ ] The audit findings tracker shows 100% resolution.

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| IoT message type semantics unconfirmable | Fail-closed (reject unknown); document the gap. |
| EC RAM max index unknown | Use conservative bound; document assumption; err on the side of rejection. |
| Frontend sweep is open-ended | Timebox to the sprint; prioritize hardware-command paths. |
| Concurrency tests are flaky | Run with multiple iterations; use deterministic synchronization primitives. |

---

## Cross-Sprint Audit Resolution Tracker

| Audit ID | Severity | Sprint | Ticket | Status |
|----------|----------|--------|--------|--------|
| Touchpad ghost touch (6 causes) | Critical | 1 | S1-001–006 | Planned |
| S1 Elev bridge escalation | Critical | 2 | S2-001 | Planned |
| S2 WiFi XML injection | Critical | 2 | S2-002 | Planned |
| S3 Hotkey script exec | Critical | 2 | S2-003 | Planned |
| A1 Blocking async calls | Critical | 3 | S3-004 | Planned |
| A2 Elev command file ACL/TOCTOU | Critical | 2 | S2-001 | Planned |
| A3 State fragmentation | High | 3 | S3-002 | Planned |
| A4 Mutex poison panic | High | 3 | S3-001, S3-005 | Planned |
| A5 panic=abort | Critical | 3 | S3-001 | Planned |
| A6 thiserror unused | High | 3 | S3-003 | Planned |
| A7 No HAL traits | High | 3 | S3-006 | Planned |
| A8 No DI/testing | High | 3 | S3-006 | Planned |
| P1 Blocking WMI async | Critical | 3/4 | S3-004, S4-005 | Planned |
| P2 No WMI reuse | Critical | 4 | S4-001 | Planned |
| P3 Dual pollers/8 IPC | Critical | 4 | S4-002 | Planned |
| P4 HID clone per frame | High | 4 | S4-003 | Planned |
| P5 Brightness loop WMI | High | 4 | S4-005 | Planned |
| P6 useHardware re-render | High | 4 | S4-004 | Planned |
| R1 Toggle a11y | High | 6 | S6-001 | Planned |
| R2 API key plaintext | High | 6 | S6-002 | Planned |
| R3 No telemetry consent | High | 6 | S6-003 | Planned |
| D1 No CI/CD | Critical | 7 | S7-001 | Planned |
| D2 No linting | High | 7 | S7-002 | Planned |
| D3 No release signing | High | 7 | S7-003 | Planned |
| D4 Version 3 places | High | 7 | S7-004 | Planned |
| F1 handleDetect interval | Critical | 5 | S5-001 | Planned |
| F2 Optimistic no revert | High | 5 | S5-002 | Planned |
| F3 Redundant audio poll | High | 5 | S5-003 | Planned |
| F4 setTimeout no cleanup | Medium | 5 | S5-004 | Planned |
| F5 touchpadDirtyUntil race | Low | 5 | S5-005 | Planned |
| B1 Battery cache race | Medium | 8 | S8-001 | Planned |
| B2 Unconfirmed msg types | High | 8 | S8-002 | Planned |
| B3 Remap state race | Medium | 8 | S8-003 | Planned |
| B4 Global WMI debounce | Medium | 8 | S8-004 | Planned |
| B5 bytes_returned validation | High | 8 | S8-005 | Planned |
| B6 ERAM index validation | Medium | 8 | S8-005 | Planned |
| B7 Unchecked pointer cast | Low | 8 | S8-007 | Planned |
| B8 No response auth | Medium | 8 | S8-006 | Planned |
| B9 Timeout not enforced | High | 8 | S8-006 | Planned |
| B10 No charging response read | Medium | 8 | S8-006 | Planned |
| S4–S8 (other security) | High/Med | 2/8 | S2-*, S8-005, S8-006 | Planned |
