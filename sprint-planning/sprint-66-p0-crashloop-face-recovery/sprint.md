# Sprint 66 — Elevated Helper Crash-Loop and Face Service Recovery

## Sprint Metadata

| Field                 | Value                                                                                                                                                                                                                                       |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Sprint Name**       | Elevated Helper Crash-Loop and Face Service Recovery                                                                                                                                                                                        |
| **Sprint Goal**       | Stop the 0xc0000005 combase crash-loop of the `--elevated` helper (9,718 WER crashes since 2026-10-20), eliminate the unnecessary scheduled-task fallback churn, and make the Face auth service self-recover when its SCM entry disappears. |
| **Duration Estimate** | ~1 week                                                                                                                                                                                                                                     |
| **Priority**          | P0                                                                                                                                                                                                                                          |
| **Sprint Type**       | Stability                                                                                                                                                                                                                                   |
| **Primary Owner**     | miControl desktop team                                                                                                                                                                                                                      |
| **Source**            | Live log forensics on the user machine (2026-10-23): Windows Event Log (Application Error + WER), `%LOCALAPPDATA%\MiControl\logs\tauri-app.log*`, `C:\ProgramData\MiControl\logs\bridge.log*`, `C:\ProgramData\MiControl\face\face_svc.log` |
| **Depends On**        | Sprint 65 (installer reliability) — independent code paths, can run in parallel                                                                                                                                                             |
| **Status**            | 📌 Active                                                                                                                                                                                                                                   |

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **MANDATORY: 100% of the tickets in this sprint MUST be completed. The sprint will
> NOT be accepted as delivered if any ticket remains incomplete.**
>
> Every ticket must pass its acceptance criteria AND the full health check suite
> before the sprint commit is made.

## Evidence Summary (2026-10-23 forensics)

### E1 — Crash-loop of the elevated helper (P0)

- **9,718 WER `Application Error` events** for `micontrol.exe` between 2026-10-20 19:03 and now (~90–150/hour, still active).
- Every crash: `Faulting module: combase.dll 10.0.26100.9168`, `Exception 0xc0000005`, **fixed offset `0xd398d`** (deterministic).
- The crashing process is the **elevated helper** (`micontrol.exe --elevated` spawned by the `MiControlElevated` scheduled task), NOT the GUI app:
  - `tauri-app.log` shows the helper's own lines (`Elevated helper started` → `Elevated command result` → `Elevated writing result to ...`) ending abruptly, then a fresh `persistent app logging enabled` block ~15–45 s later.
  - The GUI app (PID 16052, started 15-Oct) is alive and healthy the whole time; heartbeat fresh, `ui_ok=1`.
  - Crash timestamps correlate 1:1 with helper invocations (e.g. 2026-10-23 16:48:55 helper ran → 16:48:55 WER crash).
- **Onset**: 2026-10-20 19:03:49. Before that, zero micontrol WER crashes in 60 days. The loop of _restarts_ (watchdog relaunches) started earlier (2026-10-10 18:09, after a user-quit re-arm), but those were silent exits, not WER crashes.
- **Root-cause hypothesis**: the S37-003 fix (return from `main` instead of `std::process::exit(0)` so the CRT unwinds COM properly) is being defeated. The helper now returns from `main` with live COM/WMI state and crashes in `combase.dll` at thread/process teardown — the same class as the v0.1.16 regression (documented in `micontrol-regression-v0116.md`), but now at exit rather than mid-call. A Windows update (combase 10.0.26100.9168) or the WMI provider degraded state (0x80041002, see E2) may have changed teardown timing.
- **Impact**: each crash spawns a WER report (disk churn, Event Log flooding), and every poll that goes through the task path pays a crash + relaunch cycle. The elevated results ARE written before the crash, so functionality mostly works — but the machine accumulates ~2,000+ WER events/day.

### E2 — WMI provider degraded state (0x80041002) — episodic (P1, tracked)

- `wmi_ec_read_adapter_power` fails with `Hardware error: 0x80041002` (WBEM_E_NOT_FOUND) in bursts: 2026-10-10/11 (26+66), 10-15/16/17 (6+159+66), 10-20 (130), 10-21 (177), today 87 so far.
- Known issue (see `mipc-hardware-bug-research.md` § WMI MICommonInterface): the MICommonInterface WMI provider intermittently degrades; `Put("InData")` fails with 0x80041002. Previously downgraded to TRACE logging.
- **New observation**: the failure bursts correlate with the crash-loop onset windows. The helper that gets a 0x80041002 may be the one crashing at exit (COM teardown after a failed WMI call leaves references in a bad state).

### E3 — MiControlFace service vanished today (P1)

- `face_svc.log`: 16:48:31 stopping → 16:48:37 starting (v0.2.19-beta) → 16:48:39 stopping. Then nothing. `sc query MiControlFace` = **1060 (does not exist)**.
- No 7045 (service install) events for MiControlFace since 2026-09-16; no uninstaller ran today (no NSIS activity; the 0.2.19 installer in ProgramData is from 16-Sep).
- `ensure_face_service` (elevated.rs:2233) returns `{"action":"not_installed","service_installed":false}` and **does not attempt recovery** — the comment says "The UI installer path (`face_service_install`) handles a fresh install", but nothing calls it automatically. The health supervisor probes `ensure_face_service` every 30 s (4,869 calls in 4 days) and each one is a no-op.
- The binary `micontrol_face_svc.exe` (22 MB, 15-Sep) is still present in Program Files.
- **Root-cause hypothesis**: the stop/start/stop sequence at 16:48:31–39 matches the `install` command's stop-poll-recreate path being interrupted (the middle "starting" at 16:48:37 is a fresh service start that immediately stopped again). Something invoked `micontrol_face_svc.exe install` (or `remove`) at 16:48 — possibly the crash-looping helper processing a stale `face_service_install` command file, or the SCM delete from a prior install racing. Needs instrumentation to confirm.

### E4 — Bridge dispatch outliers (P2)

- `read_thermal_readings`: 33,499 calls, avg 35 ms, **max 652,026 ms** (2026-10-21 17:42:48 — an 11-minute dispatch). One-off, but the dispatch has no internal deadline; the client-side timeout is the only bound.
- `ensure_ecram_service`: 12 calls, avg 6,031 ms, max 14,302 ms — known slow (ELEV_TIMEOUT_SLOW 90 s), OK.

### E5 — IoTSvc churn (P2)

- `ensure_ecram_service` re-created the IoTSvc service 2× today (07:42, 14:30), 3× on 20-Oct; historical peaks: 37×/day (28-Sep), 28×/day (27-Sep). Each recreation stops/copies/re-creates the service — disruptive and logs 7045 events each time.
- The IoT watchdog (health_supervisor) probes the pipe every 30 s with a 300 s recovery cooldown; when the pipe probe fails transiently, the full destructive `ensure_ecram_service` runs.

### E6 — LocalDumps not capturing micontrol.exe dumps (P3)

- Registry `HKLM\...\WER\LocalDumps\MiControl.exe` (and lowercase variant) configured: DumpFolder=`%LOCALAPPDATA%\MiControl\crashdumps`, DumpType=2, DumpCount=10.
- The crashdumps folder is **empty** (and its ACL denies even directory listing to the user — the folder was created by an elevated helper with a restrictive ACL).
- 9,718 crashes → 0 dumps. The WER ReportArchive/ReportQueue entries exist but are admin-only. This blocks stack-trace forensics for E1.

## Health Check Commands (must pass before commit)

```bash
pnpm run health-check
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

## Tickets

### S66-001 — Stop the elevated helper crash at exit (combase 0xc0000005)

| Field                | Value                                                                      |
| -------------------- | -------------------------------------------------------------------------- |
| **Ticket ID**        | S66-001                                                                    |
| **Title**            | Make the `--elevated` helper exit without crashing in combase.dll teardown |
| **Priority**         | P0                                                                         |
| **Type**             | Fix                                                                        |
| **Estimated Effort** | M                                                                          |
| **Source Finding**   | E1                                                                         |
| **Status**           | ⏳ Planned                                                                 |

#### Context

`micontrol.exe --elevated` (spawned by the `MiControlElevated` scheduled task) crashes at process exit in `combase.dll` at fixed offset `0xd398d` (0xc0000005), ~90–150×/hour since 2026-10-20. The S37-003 fix (return from main instead of `std::process::exit`) is not holding. The helper holds live COM/WMI state (IWbemLocator/IWbemServices connections, possibly MICommonInterface method calls that failed with 0x80041002) when `main` returns; the CRT teardown then walks COM apartment release and faults.

#### Acceptance Criteria

- [ ] The helper process exits with code 0 and **zero** new `Application Error` events for `micontrol.exe` over a 24 h soak on the user machine (measured via `Get-WinEvent` count delta).
- [ ] The fix addresses teardown explicitly: either (a) release all COM interfaces + `CoUninitialize` on every exit path before returning from `main`, or (b) reinstate a controlled `std::process::exit(0)` AFTER explicit COM cleanup (the S37-003 comment says exit(0) triggered the crash — verify which path actually faults with a local repro before choosing).
- [ ] A unit or integration test covers the helper exit path (dispatch → cleanup → exit) with a mock WMI failure injected (0x80041002) to exercise the degraded-provider teardown.
- [ ] `pnpm run health-check` passes.

### S66-002 — Route polls away from the scheduled task when the bridge is healthy

| Field                | Value                                                     |
| -------------------- | --------------------------------------------------------- |
| **Ticket ID**        | S66-002                                                   |
| **Title**            | Eliminate scheduled-task fallback churn for routine polls |
| **Priority**         | P0                                                        |
| **Type**             | Fix                                                       |
| **Estimated Effort** | M                                                         |
| **Source Finding**   | E1 (corollary)                                            |
| **Status**           | ⏳ Planned                                                |

#### Context

The bridge pipe answers `wmi_ec_read_adapter_power` in ~14 ms average (47,486 ok responses in 4 days), yet ~90–150 calls/hour still fall through to the scheduled-task path (each one spawning the crash-looping helper). The fallback happens when the pipe round-trip fails transiently (timeout, busy, HMAC) — and each fallback spawns a process that crashes (E1). Even after S66-001, the task path is 1000× slower than the pipe.

#### Acceptance Criteria

- [ ] Pipe-round-trip failures are retried once (short backoff, e.g. 250 ms) BEFORE falling back to the scheduled task, so transient pipe hiccups don't spawn helper processes.
- [ ] A "pipe healthy" cache (similar to `IGCL_SET_AVAILABLE` / failure-cache patterns) marks the bridge as preferred for a cooldown window after a successful round-trip; during that window, transient failures retry the pipe instead of escalating.
- [ ] Measured on the user machine: scheduled-task invocations (`schtasks /run` count in bridge.log / Event Log 4698-adjacent telemetry or helper-start lines in tauri-app.log) drop by >90% over 24 h.
- [ ] `pnpm run health-check` passes.

### S66-003 — Self-heal the MiControlFace service when its SCM entry disappears

| Field                | Value                                                                                     |
| -------------------- | ----------------------------------------------------------------------------------------- |
| **Ticket ID**        | S66-003                                                                                   |
| **Title**            | `ensure_face_service` reinstalls the service when the binary exists but SCM entry is gone |
| **Priority**         | P1                                                                                        |
| **Type**             | Fix                                                                                       |
| **Estimated Effort** | M                                                                                         |
| **Source Finding**   | E3                                                                                        |
| **Status**           | ⏳ Planned                                                                                |

#### Context

Today at 16:48 the MiControlFace service stopped twice and its SCM entry vanished (1060). `ensure_face_service` (elevated.rs:2233) sees "does not exist" and returns `not_installed` without any recovery, even though `micontrol_face_svc.exe` is present in Program Files. The health supervisor calls it every 30 s (4,869 no-ops in 4 days). Face Unlock is dead until a manual reinstall.

#### Acceptance Criteria

- [ ] When `sc query MiControlFace` reports 1060 AND the deployed binary exists (`find_deployed_face_svc_exe()`), `ensure_face_service` runs the idempotent install path (`micontrol_face_svc.exe install`) and reports `action: "reinstalled"`.
- [ ] The reinstall is rate-limited (e.g. once per 10 min) so a persistently-failing install doesn't loop every 30 s with the supervisor.
- [ ] A regression test covers: service absent + binary present → reinstall attempted; service absent + binary absent → `not_installed` (unchanged).
- [ ] `pnpm run health-check` passes.

### S66-004 — Bound bridge dispatch latency for thermal reads

| Field                | Value                                                                |
| -------------------- | -------------------------------------------------------------------- |
| **Ticket ID**        | S66-004                                                              |
| **Title**            | Internal deadline for `read_thermal_readings` dispatch in the bridge |
| **Priority**         | P2                                                                   |
| **Type**             | Fix                                                                  |
| **Estimated Effort** | S                                                                    |
| **Source Finding**   | E4                                                                   |
| **Status**           | ⏳ Planned                                                           |

#### Context

One `read_thermal_readings` dispatch took 652 s (2026-10-21 17:42) — the WMI/ESIF call inside the bridge hung with no internal bound. The client times out at 15–30 s, but the bridge worker stays blocked, and the client retries pile new requests behind it.

#### Acceptance Criteria

- [ ] The thermal-read dispatch inside `micontrol_bridge.rs` is wrapped in a bounded wait (e.g. 30 s); on expiry it returns an error response to the client instead of blocking the worker.
- [ ] A WARN is logged (throttled) when the internal deadline fires, including the elapsed time.
- [ ] `pnpm run health-check` passes.

### S66-005 — Rate-limit destructive `ensure_ecram_service` recovery

| Field                | Value                                                         |
| -------------------- | ------------------------------------------------------------- |
| **Ticket ID**        | S66-005                                                       |
| **Title**            | Cooldown + probe-before-recreate for the IoTSvc recovery path |
| **Priority**         | P2                                                            |
| **Type**             | Fix                                                           |
| **Estimated Effort** | S                                                             |
| **Source Finding**   | E5                                                            |
| **Status**           | ⏳ Planned                                                    |

#### Context

`ensure_ecram_service` stops, copies a binary into the DriverStore and re-creates the IoTSvc service. It ran 2× today, 3× on 20-Oct, and historically up to 37×/day (28-Sep). Each run is disruptive (service downtime, 7045 events, DriverStore writes). The trigger is a transient pipe-probe failure in the IoT watchdog.

#### Acceptance Criteria

- [ ] Before the destructive path, a lightweight pipe probe (`OpenOptions` open test) confirms the pipe is REALLY absent (not just a transient connect failure) — retry the probe 2× with a short delay.
- [ ] A persistent cooldown (e.g. 15 min between full recreations) is enforced inside the bridge dispatch, returning a "cooldown" response when hit.
- [ ] Measured on the user machine: 7045 events for IoTSvc drop to ≤1/day under normal operation.
- [ ] `pnpm run health-check` passes.

### S66-006 — Fix LocalDumps capture for micontrol.exe

| Field                | Value                                                                |
| -------------------- | -------------------------------------------------------------------- |
| **Ticket ID**        | S66-006                                                              |
| **Title**            | WER LocalDumps actually writes dumps for the elevated helper crashes |
| **Priority**         | P3                                                                   |
| **Type**             | Fix                                                                  |
| **Estimated Effort** | S                                                                    |
| **Source Finding**   | E6                                                                   |
| **Status**           | ⏳ Planned                                                           |

#### Context

LocalDumps is registered for `MiControl.exe` (DumpType=2, DumpCount=10) but 9,718 crashes produced zero dumps. The crashdumps folder ACL denies user access (created by an elevated context). Possible causes: (a) the registry key name must match the process name exactly (`micontrol.exe` — both cases exist, but WER matches per-key), (b) the folder ACL prevents WER from writing, (c) the crash is in a context where LocalDumps is bypassed.

#### Acceptance Criteria

- [ ] The crashdumps folder is created with an ACL that allows WER (SYSTEM) to write and the user to read.
- [ ] A test crash (e.g. `micontrol.exe --elevated` repro from S66-001, or a deliberate `int 3`-style test binary) produces a .dmp file in the configured folder.
- [ ] If WER cannot be made to capture this crash class, document why in `docs/` and provide an alternative capture path (e.g. a watchdog-side MiniDumpWriteDump on helper exit-code != 0).
- [ ] `pnpm run health-check` passes.

## Deferred / Out of Scope

- **E2 (WMI 0x80041002 episodic degradation)**: known upstream provider bug, already handled gracefully (TRACE logging + fallbacks). Revisit only if S66-001 shows the crash persists after COM teardown fix.
- **Watchdog restart-loop from 2026-10-10** (silent restarts before the WER crashes began): the restarts are the watchdog doing its job after each helper crash kills the shared process? — NO: the helper is a separate process; the restart loop in tauri-app.log is the GUI app being relaunched by the watchdog after user-quit re-arm. Not a defect; not tracked here.

## Sprint Validation Gate

Run `pnpm run health-check` + `cargo test --manifest-path src-tauri/Cargo.toml --lib` after each ticket.
Live-machine validation (WER event count deltas, service state) requires a new build installed on the user machine — coordinate explicitly; do not publish without approval.
