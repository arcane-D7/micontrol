# Sprint 39 — OS Turbo Measured Adaptive Control

## Sprint Metadata

| Field                 | Value                                                                                                                            |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| **Sprint Name**       | OS Turbo Measured Adaptive Control                                                                                               |
| **Sprint Goal**       | Replace the unsafe OS Turbo implementation with a reversible, measured, statistically grounded, privacy-preserving control loop. |
| **Duration Estimate** | ~2–3 weeks                                                                                                                       |
| **Priority**          | P1                                                                                                                               |
| **Sprint Type**       | Feature                                                                                                                          |
| **Primary Owner**     | miControl desktop team                                                                                                           |
| **Source**            | User-requested OS Turbo redesign, 2026-09-18                                                                                     |
| **Depends On**        | Sprint 37 and the existing authenticated bridge service                                                                          |
| **Status**            | 🔄 In progress — S39-002 implemented, validation pending                                                                         |

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **MANDATORY: 100% of the tickets in this sprint MUST be completed. The sprint will
> NOT be accepted as delivered if any ticket remains incomplete.**
>
> Every ticket must pass its acceptance criteria AND the full health check suite
> before the sprint commit is made.

## Sprint Goal Statement

OS Turbo currently uses an undocumented native process-information call, broad elevation fallbacks, and no measured control boundary. This sprint establishes a reversible, bounded, statistically evaluated adaptive policy with explicit privacy and user-approval controls. The learner is statistical, not a deep-learning model: deterministic windowed statistics drive bounded policy decisions, while optional BYOK suggestions remain advisory and independent of the control loop.

All writes that require administrator rights remain behind the authenticated bridge service. UAC prompts, scheduled-task fallbacks for OS Turbo writes, thermal-limit changes, EC writes, release work, installation, and publishing are out of scope.

## Health Check Commands (must pass before commit)

```bash
pwsh -File scripts/health-check.ps1
```

## Execution Order

Tickets are intentionally sequential because each ticket writes data or policy consumed by the next ticket. Do not parallelize these tickets.

## Tickets

### S39-001 — Document the public EcoQoS API and enforce the bridge-only write boundary

| Field                | Value                                                                                                                 |
| -------------------- | --------------------------------------------------------------------------------------------------------------------- |
| **Ticket ID**        | S39-001                                                                                                               |
| **Title**            | Document the public EcoQoS API and enforce the bridge-only write boundary                                             |
| **Priority**         | P0                                                                                                                    |
| **Type**             | Fix                                                                                                                   |
| **Estimated Effort** | M                                                                                                                     |
| **Source Finding**   | User request: wrong `NtSetInformationProcess` class 4 call and OS Turbo writes must use the authenticated bridge only |
| **Status**           | ✅ Complete                                                                                                           |

#### Context

`os_turbo.rs` currently links the undocumented `NtSetInformationProcess` entry point and hardcodes information class 4, even though the repository already has a working public `SetProcessInformation` pattern. The generic elevation path can fall back to a scheduled task or UAC, which is not acceptable for this feature's administrator write boundary. The first ticket must establish the narrow contract that later reversible and measured controls depend on.

#### Acceptance Criteria

- [x] Replace the raw native call with the Windows crate's public `SetProcessInformation` using `ProcessPowerThrottling` and `PROCESS_POWER_THROTTLING_STATE`.
- [x] Preserve the official state contract: `Version = 1`, `ControlMask = PROCESS_POWER_THROTTLING_EXECUTION_SPEED`, and `StateMask` set only for the enable operation.
- [x] Record numeric, safe Windows error codes for failed process actions without treating a failed action as successful; the returned action count includes successful calls only.
- [x] Route `set_os_turbo` through an authenticated bridge-only helper that does not use direct dispatch, scheduled-task fallback, or UAC fallback.
- [x] Add focused state/API tests and, where supported by the Windows test environment, a disposable-child smoke test that never targets an existing user process.
- [x] Run the health check command successfully.

#### Execution Evidence — 2026-09-18

- [x] Public EcoQoS API disposable-child readback verified: 2/2.
- [x] `node scripts/health-check.mjs` passed all 11/11 checks on September 18, 2026.
- [x] `git diff --check` passed.
- [x] Commit `13a7009` created with message `feat(s39-001): correct EcoQoS API and require authenticated bridge`.

### S39-002 — Capture reversible process identity and the prior power scheme

| Field                | Value                                                                 |
| -------------------- | --------------------------------------------------------------------- |
| **Ticket ID**        | S39-002                                                               |
| **Title**            | Capture reversible process identity and the prior power scheme        |
| **Priority**         | P1                                                                    |
| **Type**             | Feature                                                               |
| **Estimated Effort** | M                                                                     |
| **Source Finding**   | User request: OS Turbo must be reversible and preserve original state |
| **Status**           | 🔄 Implemented — awaiting focused validation                          |

#### Context

The current implementation identifies targets only by process name and restores a hardcoded Balanced plan. That loses process identity across exits and can overwrite the user's prior power scheme. A reversible design needs a versioned snapshot of the exact process identity and original state, plus the active scheme captured before the first write.

#### Acceptance Criteria

- [ ] Persist a versioned, bounded snapshot containing allowlisted process identity, observed original process state, and the prior active power scheme before any OS Turbo write.
- [ ] Restore only matching process identities that are still valid and skip exited, reused, or unverifiable processes.
- [ ] Restore the captured prior power scheme instead of assuming Balanced.
- [ ] Make enable/disable idempotent and ensure partial failures do not erase the last known reversible state.
- [ ] Add tests for snapshot creation, process identity mismatch, restore ordering, and missing prior-state data.
- [ ] Run the health check command successfully.

#### Execution Evidence — 2026-09-18

- [x] Implemented bounded versioned recovery snapshots with PID plus creation-time validation.
- [x] Captured and conditionally restored the prior active power scheme without overwriting external scheme changes.
- [x] Preserved snapshots across partial failures and removed the startup reapply sweep until S39-003.
- [ ] Focused Rust tests and health check pending.

### S39-003 — Add measured status and a bounded periodic worker

| Field                | Value                                                                     |
| -------------------- | ------------------------------------------------------------------------- |
| **Ticket ID**        | S39-003                                                                   |
| **Title**            | Add measured status and a bounded periodic worker                         |
| **Priority**         | P1                                                                        |
| **Type**             | Feature                                                                   |
| **Estimated Effort** | L                                                                         |
| **Source Finding**   | User request: OS Turbo needs measured status and periodic bounded control |
| **Status**           | ⏳ Planned                                                                |

#### Context

The current restore path launches an unbounded background sweep and reports a placeholder throttled-process count. Adaptive control needs observable measurements, explicit lifecycle ownership, and bounded work. The worker must not become a second elevation or hardware-control path.

#### Acceptance Criteria

- [ ] Expose measured status for inspected, successfully changed, skipped, failed, and restored allowlisted processes without exposing identifying process data to the UI.
- [ ] Implement one cancellable periodic worker with fixed interval, work-item, queue, and duration bounds.
- [ ] Ensure worker restart, disable, shutdown, and bridge failure paths are deterministic and do not leak threads or duplicate actions.
- [ ] Keep status reads separate from administrator writes and preserve the bridge-only write boundary from S39-001.
- [ ] Add tests for worker cadence, cancellation, bounded batches, duplicate suppression, and bridge unavailability.
- [ ] Run the health check command successfully.

### S39-004 — Implement deterministic windowed statistics

| Field                | Value                                                                            |
| -------------------- | -------------------------------------------------------------------------------- |
| **Ticket ID**        | S39-004                                                                          |
| **Title**            | Implement deterministic windowed statistics                                      |
| **Priority**         | P1                                                                               |
| **Type**             | Feature                                                                          |
| **Estimated Effort** | XL                                                                               |
| **Source Finding**   | User request: deterministic statistics adaptive control with defensible evidence |
| **Status**           | ⏳ Planned                                                                       |

#### Context

OS Turbo has no evidence model for deciding whether an action helped. This ticket adds the statistical learner used by later policy code. It must be described accurately as deterministic statistics, not deep learning, and must reject evidence that cannot support a comparison.

#### Acceptance Criteria

- [ ] Use deterministic fixed-size or fixed-duration windows with Welford mean and standard-deviation updates and numerically stable merge/reset behavior.
- [ ] Enforce a documented minimum sample count and reject missing, invalid, stale, or unmatched workload data.
- [ ] Compare matched workloads with confidence bounds and effect sizes; retain the inputs needed to explain a decision without retaining identifying data.
- [ ] Refuse causal claims when no matched control or valid baseline exists, and surface the result as insufficient evidence instead.
- [ ] Make ordering, window boundaries, rounding, confidence calculation, and serialization deterministic across runs.
- [ ] Add tests for Welford updates, variance edge cases, minimum samples, missing data, matched controls, confidence/effect-size thresholds, and the no-causal-claim path.
- [ ] Document that this is a statistical learner and does not claim deep-learning behavior.
- [ ] Run the health check command successfully.

### S39-005 — Add a bounded allowlisted adaptive policy with cooldown and rollback

| Field                | Value                                                                         |
| -------------------- | ----------------------------------------------------------------------------- |
| **Ticket ID**        | S39-005                                                                       |
| **Title**            | Add a bounded allowlisted adaptive policy with cooldown and rollback          |
| **Priority**         | P1                                                                            |
| **Type**             | Feature                                                                       |
| **Estimated Effort** | L                                                                             |
| **Source Finding**   | User request: adaptive control must be bounded, reversible, and hardware-safe |
| **Status**           | ⏳ Planned                                                                    |

#### Context

Measured results are not safe to apply directly. Policy decisions need fixed bounds, allowlists, cooldowns, rollback, and explicit refusal states. OS Turbo must remain a scheduler/power-plan feature and must never write thermal limits or EC registers.

#### Acceptance Criteria

- [ ] Apply only a versioned, reviewable allowlist of process and power-policy actions within fixed limits.
- [ ] Require sufficient S39-004 evidence before an action, enforce cooldowns and rate limits, and make repeated decisions idempotent.
- [ ] Roll back on failed verification, degraded measurements, bridge errors, policy version mismatch, or explicit disable.
- [ ] Prove by tests and code review that no path writes thermal limits, EC registers, firmware settings, or unallowlisted process state.
- [ ] Keep optional BYOK suggestions outside the deterministic control decision and require no network access for the core policy.
- [ ] Add tests for bounds, cooldowns, rollback, insufficient evidence, policy migration, and forbidden-operation rejection.
- [ ] Run the health check command successfully.

### S39-006 — Add anonymous bounded action logs and privacy tests

| Field                | Value                                                                         |
| -------------------- | ----------------------------------------------------------------------------- |
| **Ticket ID**        | S39-006                                                                       |
| **Title**            | Add anonymous bounded action logs and privacy tests                           |
| **Priority**         | P0                                                                            |
| **Type**             | Security                                                                      |
| **Estimated Effort** | XL                                                                            |
| **Source Finding**   | User request: fully anonymized before/after logs with retention and byte caps |
| **Status**           | ⏳ Planned                                                                    |

#### Context

Adaptive control needs before/after evidence, but ordinary process and error logs can identify users or disclose local machine details. The log contract must be anonymous by construction and bounded in both retention and storage, including failure paths.

#### Acceptance Criteria

- [ ] Log only allowlisted anonymous action categories and aggregate before/after measurements; never log process names, paths, PIDs, user/device identifiers, identifying timestamps, raw errors, prompt text, or suggestion text.
- [ ] Apply explicit record-count, byte-size, and retention caps with deterministic eviction and atomic writes.
- [ ] Redact or map all errors to safe stable codes before logging and ensure disabled logging produces no action records.
- [ ] Add privacy tests scanning serialized records and rendered log output for names, paths, PIDs, IDs, identifying timestamps, raw errors, and prompt text.
- [ ] Add tests for caps, eviction, corruption recovery, concurrent writers, and deletion/retention behavior.
- [ ] Run the health check command successfully.

### S39-007 — Keep optional BYOK suggestions structured, bounded, and user-approved

| Field                | Value                                                                                 |
| -------------------- | ------------------------------------------------------------------------------------- |
| **Ticket ID**        | S39-007                                                                               |
| **Title**            | Keep optional BYOK suggestions structured, bounded, and user-approved                 |
| **Priority**         | P2                                                                                    |
| **Type**             | Feature                                                                               |
| **Estimated Effort** | M                                                                                     |
| **Source Finding**   | User request: existing optional BYOK suggestions must remain advisory and independent |
| **Status**           | ⏳ Planned                                                                            |

#### Context

The repository already has optional BYOK suggestion support. It must not become an implicit control authority or bypass deterministic evidence, fixed policy bounds, privacy logging, or user consent. Suggestions are advisory text/data, never an autonomous write trigger.

#### Acceptance Criteria

- [ ] Reuse the existing BYOK integration and return a structured suggestion schema with strict size, enum, numeric, and action validation.
- [ ] Reject malformed, out-of-range, unallowlisted, or privacy-violating suggestions before they reach policy code.
- [ ] Require explicit, current user approval for every suggestion that could cause a write, with a clear no-op path on denial or expiry.
- [ ] Demonstrate that BYOK availability, failure, or output cannot change deterministic statistics or trigger autonomous OS Turbo actions.
- [ ] Add tests for schema validation, bounds, approval/expiry, malformed output, provider failure, and independence from the policy loop.
- [ ] Run the health check command successfully.

### S39-008 — Integrate the OS Turbo UI across four locales

| Field                | Value                                                |
| -------------------- | ---------------------------------------------------- |
| **Ticket ID**        | S39-008                                              |
| **Title**            | Integrate the OS Turbo UI across four locales        |
| **Priority**         | P1                                                   |
| **Type**             | Feature                                              |
| **Estimated Effort** | L                                                    |
| **Source Finding**   | User request: integrated UI, four locales, and tests |
| **Status**           | ⏳ Planned                                           |

#### Context

The UI currently exposes a simple enable/disable result, while the redesigned feature needs status, evidence quality, reversible state, bounded policy state, and approval/error states. The workflow must remain understandable and fully localized in the four locales already supported by the application.

#### Acceptance Criteria

- [ ] Integrate measured status, evidence sufficiency, policy state, cooldown, rollback, bridge availability, and approval state without exposing private process identifiers.
- [ ] Localize all new strings in each of the four existing locales and keep error/status messages based on stable codes rather than raw backend errors.
- [ ] Make enable, disable, rollback, insufficient-data, bridge-unavailable, suggestion-pending, and suggestion-rejected states accessible and testable.
- [ ] Add frontend tests for rendering, transitions, approval, disabled controls, errors, and locale completeness.
- [ ] Ensure the UI cannot invoke a generic elevation/UAC path for OS Turbo writes.
- [ ] Run the health check command successfully.

### S39-009 — Run the runtime and integration audit with full gates

| Field                | Value                                                                            |
| -------------------- | -------------------------------------------------------------------------------- |
| **Ticket ID**        | S39-009                                                                          |
| **Title**            | Run the runtime and integration audit with full gates                            |
| **Priority**         | P0                                                                               |
| **Type**             | Test                                                                             |
| **Estimated Effort** | M                                                                                |
| **Source Finding**   | User request: runtime verification, integration audit, and complete health gates |
| **Status**           | ⏳ Planned                                                                       |

#### Context

The feature crosses the frontend, Tauri commands, authenticated bridge, Windows process APIs, persistence, statistics, policy, logging, and optional BYOK paths. Static compilation alone cannot prove that the runtime boundary is correct. The final ticket verifies the complete flow and records limitations without claiming a runtime result that was not observed.

#### Acceptance Criteria

- [ ] Run focused Rust/API/state tests, bridge-auth tests, privacy tests, frontend tests, and the full health check suite.
- [ ] Run a Windows disposable-child integration smoke test that verifies enable/query/disable behavior without UAC, third-party services, or targeting an existing user process.
- [ ] Verify that an unavailable bridge returns a bounded, safe error and never falls back to UAC or scheduled task for OS Turbo writes.
- [ ] Verify rollback restores the captured prior power scheme and process state after a failed or rejected action.
- [ ] Audit logs and UI output for privacy, locale completeness, missing-data handling, and absence of causal claims without matched controls.
- [ ] Record runtime model verification honestly: only the requested GPT-5.6-Luna (codex-for-copilot) runtime may be claimed, and execution must stop if the exposed runtime differs.
- [ ] Run the health check command successfully.

## Sprint Commit

```bash
git add -A
git commit -m "feat(sprint-39): implement measured adaptive OS Turbo"

- S39-001: Document the public EcoQoS API and enforce the bridge-only write boundary (user request)
- S39-002: Capture reversible process identity and the prior power scheme (user request)
- S39-003: Add measured status and a bounded periodic worker (user request)
- S39-004: Implement deterministic windowed statistics (user request)
- S39-005: Add a bounded allowlisted adaptive policy with cooldown and rollback (user request)
- S39-006: Add anonymous bounded action logs and privacy tests (user request)
- S39-007: Keep optional BYOK suggestions structured, bounded, and user-approved (user request)
- S39-008: Integrate the OS Turbo UI across four locales (user request)
- S39-009: Run the runtime and integration audit with full gates (user request)
```
