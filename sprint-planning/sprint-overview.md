# Sprint Plan Overview — Stability Report Remediation (v1 + v2) + Audit_Final

**Created:** 2026-06-25
**Last Updated:** 2026-10-23 (S30–S33 reconciled; S39 evidence aligned; S63 regression reopened; S65 planned; S66 opened from live-machine log forensics)
**Sources:**

- v1: `docs/stability-report-2026-06-24-post-sprints-13-15.md` (Sprints 16–19)
- v2: `docs/STABILITY_REPORT_v2.md` (Sprints 22–25)
- v3: `docs/STABILITY_REPORT_v3.md` (Sprints 26–27)
- Deferred: `sprint-planning/sprint-28-p3-deferred-backlog/sprint.md` (Sprint 28)
- v4: `docs/STABILITY_REPORT_v4.md` (Sprint 29)
- Sprints 20–21: Post-v1 audit CRITICAL/HIGH fixes (committed d514bdf)
- **Audit_Final**: `C:\Users\mafsc\Documents\Audit_Final.md` (Sprints 30–33) — 26 issues, 2 regressions + 24 pre-existing
- **Audit_miControl_2026**: C:\Users\mafsc\Documents\Audit_Report_miControl.md (Sprints 34–37) — 10 issues (6 user-reported bugs + 4 security)
- **Installer decoupling**: `docs/installer-decoupling.md` and current shell/CI implementation — productionization gaps tracked in Sprint 65
- **Live-machine forensics (2026-10-23)**: elevated helper combase crash-loop, face service loss, bridge latency outlier, IoTSvc churn, LocalDumps gap — tracked in Sprint 66

**Total findings addressed:** 63 (v1) + 44 (v2) + 27 (v3) + 20 (deferred) + 27 (v4) + 26 (Audit_Final) + 10 (Audit_miControl_2026) = 217
**Total estimated tickets:** 63 (v1) + 44 (v2) + 19 (v3) + 14 (S28) + 3 (S29) + 26 (S30–33) + 23 (S34–37) = 192
**Total estimated effort:** 9–13 days (v1) + 9–12 days (v2) + ~10 days (v3+S28) + ~1–2 days (S29) + ~12–19 days = 41–56 days

---

## Sprint Summary

### v1 Sprints (from post-sprints-13-15 report) — ✅ ALL COMPLETE

| Sprint | Priority    | Focus                      | Tickets | Effort   | Status     | File                                        |
| ------ | ----------- | -------------------------- | ------- | -------- | ---------- | ------------------------------------------- |
| 16     | P0 CRITICAL | Pre-release blockers       | 15      | 2–3 days | ✅ de5e344 | `sprint-16-p0-critical-fixes/sprint.md`     |
| 17     | P1 HIGH     | Security & DevOps          | 15      | 2–3 days | ✅ cb9005f | `sprint-17-p1-security-devops/sprint.md`    |
| 18     | P1 HIGH     | Error handling & stability | 15      | 2–3 days | ✅ c76236f | `sprint-18-p1-error-stability/sprint.md`    |
| 19     | P2 MEDIUM   | Architecture & tests       | 18      | 3–4 days | ✅ 1a383c0 | `sprint-19-p2-architecture-tests/sprint.md` |

### Post-v1 Audit Fixes — ✅ COMPLETE

| Sprint | Priority | Focus                       | Tickets | Effort | Status     | File                                       |
| ------ | -------- | --------------------------- | ------- | ------ | ---------- | ------------------------------------------ |
| 20–21  | P0/P1    | Post-v1 audit CRITICAL/HIGH | 9+6     | 2 days | ✅ d514bdf | `sprint-20-21-p0-p1-audit-fixes/sprint.md` |

### v2 Sprints (from Stability Report v2) — ✅ COMPLETE

| Sprint | Priority    | Focus                          | Tickets | Effort  | Status                                        | File                                        |
| ------ | ----------- | ------------------------------ | ------- | ------- | --------------------------------------------- | ------------------------------------------- |
| 22     | P0 CRITICAL | Async blocking I/O             | 2       | ~1 day  | ✅ 3a73f4b                                    | `sprint-22-p0-async-blocking/sprint.md`     |
| 23     | P1 HIGH     | Stability & security edges     | 5       | ~3 days | ✅ fef49f9                                    | `sprint-23-p1-stability-security/sprint.md` |
| 24     | P2 MEDIUM   | Architecture/UI/Perf/AI/DevOps | 19      | ~5 days | ✅ b4e467b (Batch A) / ✅ 5bd819b (Batch B+C) | `sprint-24-p2-medium-batch/sprint.md`       |
| 25     | P3 LOW      | Polish & consistency           | 18      | ~3 days | ✅ 100a1d2                                    | `sprint-25-p3-low-polish/sprint.md`         |

### v3 Sprints (from Stability Report v3) — ✅ ALL COMPLETE

| Sprint | Priority  | Focus                           | Tickets | Effort  | Status     | File                                      |
| ------ | --------- | ------------------------------- | ------- | ------- | ---------- | ----------------------------------------- |
| 26     | P2 MEDIUM | Residual blocking I/O, ACL gaps | 8       | ~3 days | ✅ a98a24a | `sprint-26-p2-medium-residual/sprint.md`  |
| 27     | P3 LOW    | Polish & defense-in-depth       | 11      | ~2 days | ✅ eaf8e85 | `sprint-27-p3-low-polish-v3/sprint.md`    |
| 28     | P3 LOW    | Deferred backlog cleanup        | 14      | ~5 days | ✅ 80872b5 | `sprint-28-p3-deferred-backlog/sprint.md` |

### v4 Sprint (from Stability Report v4) — ✅ COMPLETE

| Sprint | Priority    | Focus                       | Tickets | Effort    | Status     | File                                   |
| ------ | ----------- | --------------------------- | ------- | --------- | ---------- | -------------------------------------- |
| 29     | P0 CRITICAL | Pre-release security & GDPR | 3       | ~1–2 days | ✅ bec0d42 | `sprint-29-p0-critical-high/sprint.md` |

### Audit_Final Sprints (from `Audit_Final.md`) — ✅ COMPLETE

| Sprint | Priority    | Focus                           | Tickets | Effort    | Status       | File                                  |
| ------ | ----------- | ------------------------------- | ------- | --------- | ------------ | ------------------------------------- |
| 30     | P0 CRITICAL | Audit regressions & UX blockers | 4       | ~2 days   | ✅ `b597d66` | `sprint-30-p0-critical-fixes/plan.md` |
| 31     | P1 HIGH     | UX fixes + FULL stub impls      | 8       | ~4–5 days | ✅ `9a8f9fc` | `sprint-31-p1-high-stubs/plan.md`     |
| 32     | P2 MEDIUM   | Hardware reliability & security | 7       | ~3–4 days | ✅ `8918a9c` | `sprint-32-p2-medium-fixes/plan.md`   |
| 33     | P3 LOW      | Code cleanup & dead code        | 7       | ~1–2 days | ✅ `4fafc07` | `sprint-33-p3-low-cleanup/plan.md`    |

**Audit_Final totals:** 4 (S30) + 8 (S31) + 7 (S32) + 7 (S33) = 26 tickets, ~10–13 days effort

**Key highlights:**

- **S30:** Fixes 2 regressions (useHardware polling, check_sentry_consent) + 2 UX blockers (consent F5 race, sidebar scroll)
- **S31:** FULL implementation of ALL stubs — Miracast via WinRT (Option B, not wrapper), ECRAM WMI discovery, audio device names, WiFi CREATE_NO_WINDOW, touchpad HID logging, startup autostart, EC Debug dev-gate
- **S32:** Adaptive brightness logging, auto-discovery, IoT bridge, battery powercfg fallback, credential allowlist
- **S33:** Dead code removal, dependency cleanup, dev-gating, documentation

### miControl Audit Sprints (from `Audit_Report_miControl.md`) — ✅ COMPLETE

| Sprint | Priority    | Focus                                   | Tickets | Effort     | Status       | File                                             |
| ------ | ----------- | --------------------------------------- | ------- | ---------- | ------------ | ------------------------------------------------ |
| 34     | P0 CRITICAL | Auth Bridge UAC + Quick Wins            | 7       | ~2–3 days  | ✅ `309bfa3` | `sprint-34-p0-critical-auth-bridge/plan.md`      |
| 35     | P1 HIGH     | Temperature, Volume, Tray, WiFi         | 7       | ~3–4 days  | ✅ `45b0d5a` | `sprint-35-p1-high-temperature-volume/plan.md`   |
| 36     | P2 MEDIUM   | Security Hardening                      | 4       | ~2–3 days  | ✅ `0a67897` | `sprint-36-p2-medium-security-hardening/plan.md` |
| 37     | P1 HIGH     | Audio Switching + WiFi WlanAPI          | 5       | ~5–10 days | ✅ `b47dea0` | `sprint-37-p1-high-audio-wifi-features/plan.md`  |
| S55    | P0 CRITICAL | Stability: crash-loop, UAC, tray        | —       | —          | ✅ `6e645c9` | (stabilization round, no plan file)              |
| 38     | P1 HIGH     | IoT Hardware Opportunities (MIOT-01–13) | 13      | ~2–3 weeks | ✅ `5341ac0` | `sprint-38-iot-hardware/plan.md`                 |
| 39     | P1 HIGH     | Measured adaptive OS Turbo              | 9       | ~2–3 weeks | ✅ `a9ad6ce` | [sprint](sprint-39-p1-os-turbo/sprint.md)        |

**miControl Audit totals:** 7 (S34) + 7 (S35) + 4 (S36) + 5 (S37) = 23 tickets, ~12–19 days effort

**Key highlights:**

- **S34:** Fixes Auth Bridge UAC regression (4 root causes: tri-state return, XML encoding, StopExisting, timeout) + 3 quick wins (color-mix CSS, resize guard, audio poll)
- **S35:** Fixes temperature sensors (remove 50°C fallback, use Option<f32>), volume slider dirty flag, tray CSS verification, WiFi delay increase
- **S36:** Security hardening (atomic result write, directory ACL, diag_ps gate, USERNAME→GetUserNameW)
- **S37:** New features (IPolicyConfig audio device switching, WlanAPI replacing netsh for WiFi)
- **S55:** Service crash-loop fix (watchdog off by default, catch_unwind on poll/restore), no boot re-apply, heartbeat on dedicated OS thread, tray Open App via run_on_main_thread + SW_RESTORE, OS Turbo HKCU→HKLM migration, debloater UI/i18n polish, mcp-bridge enabled in release behind HKCU toggle

---

## Finding Coverage Matrix

### CRITICAL (10 findings → all in Sprint 16)

| #   | Finding                        | Sprint | Ticket         |
| --- | ------------------------------ | ------ | -------------- |
| 1   | Incomplete data deletion       | S16    | S16-01, S16-11 |
| 2   | KEYRING_SERVICE mismatch       | S16    | S16-02         |
| 3   | Battery OnceLock panic         | S16    | S16-03         |
| 4   | Double font loading            | S16    | S16-04         |
| 5   | lint-staged no config          | S16    | S16-05         |
| 6   | Hardcoded English TrayPopup    | S16    | S16-06         |
| 7   | Hardcoded English theme labels | S16    | S16-07         |
| 8   | ErrorBoundary locale imports   | S16    | S16-08         |
| 9   | No AI HTTP timeout             | S16    | S16-09         |
| 10  | ErrorResponse.code unused      | S16    | S16-10         |

### HIGH (24 findings → Sprints 16-18)

| #   | Finding                           | Sprint  | Ticket           |
| --- | --------------------------------- | ------- | ---------------- |
| S1  | WiFi XOR encryption               | S17     | S17-02           |
| S2  | Script path ends_with bypass      | S17     | S17-01           |
| D2  | Authenticode signing silent skip  | S17     | S17-03           |
| D3  | No .env.example                   | S17     | S17-04           |
| D4  | README placeholder URLs           | S16/S17 | S16-13, S17-13   |
| E2  | elevated.rs Mutex poison          | S18     | S18-01           |
| E3  | touchpad.rs Mutex poison          | S18     | S18-02           |
| E4  | useHardware.ts console.error only | S18     | S18-07           |
| E5  | ErrorResponse.code unused         | S16     | S16-10           |
| R3  | No prompt injection protection    | S18     | S18-13           |
| R4  | No content filters                | S18     | S18-13           |
| R5  | Sentry no PII stripping           | S17     | S17-11           |
| T1  | battery.rs .expect() panic        | S16     | S16-03           |
| T2  | Audit log unbounded               | S18     | S18-06           |
| T3  | Only 3 frontend test files        | S19     | S19-11           |
| A1  | touchpad.rs god-module            | S19     | S19-02           |
| A2  | WMI extraction duplicated         | S19     | S19-01           |
| A3  | Tests essentially absent          | S19     | S19-07 to S19-10 |
| Q1  | expect() in BATTERY_STATIC_DATA   | S16     | S16-03           |
| Q2  | Duplicate Props in MainWindow     | S19     | S19-12           |
| Q3  | 21 #[allow(dead_code)]            | S19     | S19-13           |
| E8  | hotkeys.rs Mutex poison           | S18     | S18-03           |
| U4  | OnboardingWizard no aria          | —       | Deferred (S20)   |
| U5  | ConsentDialog no focus ring       | —       | Deferred (S20)   |

### MEDIUM (60 findings → Sprints 16-19 + deferred)

Key MEDIUM findings addressed:

| Finding                                 | Sprint | Ticket         |
| --------------------------------------- | ------ | -------------- |
| S3 URL validation basic                 | S17    | S17-12         |
| S4 Data deletion incomplete             | S16    | S16-01         |
| S6 CSP missing directives               | S16    | S16-12         |
| S7 shell:default overly broad           | —      | Deferred (S20) |
| A4 ai.rs/hotkeys.rs bypass typed errors | S19    | S19-04, S19-05 |
| A8 unsafe blocks lack SAFETY comments   | S19    | S19-14         |
| E7 retry.rs no backoff                  | S18    | S18-05         |
| E10 wmi_cache unwrap                    | S18    | S18-10         |
| E11 Silent fallback battery values      | S16    | S16-03         |
| E12 get_process_list silent             | S18    | S18-11         |
| E13 Audit log unbounded                 | S18    | S18-06         |
| E14 Nonce replay window                 | S18    | S18-08         |
| Q4 Blanket From<String>                 | S19    | S19-03         |
| Q6 useSettings God object               | —      | Deferred (S20) |
| Q9 spawn_blocking boilerplate           | S19    | S19-06         |
| V4 No data portability                  | S19    | S19-16         |
| V5 WiFi XOR encryption                  | S17    | S17-02         |
| V6 HMAC key reused                      | S19    | S19-17         |
| D5 npm audit not enforced               | S17    | S17-05         |
| D6 Coverage failures silent             | S17    | S17-06         |
| D8 No PR template                       | S17    | S17-09         |
| D9 No issue templates                   | S17    | S17-10         |
| D10 cargo fmt missing pre-commit        | S16    | S16-05         |

### LOW / INFO (91 findings → partially addressed, mostly deferred)

Key LOW findings addressed:

- U19 Spanish typo → S16-14
- U14 French diacritics → S16-14
- D13 No SECURITY.md → S17-07
- D14 No CODE_OF_CONDUCT.md → S17-08
- D15 MSI dead code → S17-14
- Q14 CHANGELOG placeholder date → Deferred
- Q15 console.error in useHardware → S18-07

---

## Execution Order

### v1 Sprints (COMPLETE)

```
Sprint 16 (P0) ──► Sprint 17 (P1) ──► Sprint 18 (P1) ──► Sprint 19 (P2)
   ✅ de5e344        ✅ cb9005f        ✅ c76236f          ✅ 1a383c0
```

### Post-v1 Audit (COMPLETE)

```
Sprint 20–21 (P0/P1) ──► Stability Report v2
   ✅ d514bdf
```

### v2 Sprints (COMPLETE)

```
Sprint 22 (P0) ──► Sprint 23 (P1) ──► Sprint 24 (P2) ──► Sprint 25 (P3) ──► Audit v3
   ✅ 3a73f4b        ✅ fef49f9        ✅ b4e467b         ✅ 100a1d2
   2 tickets         5 tickets         19 tickets         18 tickets
   ~1 day            ~3 days           ~5 days            ~3 days
```

### v3 Sprints (COMPLETE)

```
Sprint 26 (P2) ──► Sprint 27 (P3) ──► Sprint 28 (P3) ──► Final Audit
   ✅ a98a24a        ✅ eaf8e85         ✅ 80872b5
   8 tickets         11 tickets         14 tickets
   ~3 days           ~2 days            ~5 days
```

**Sprint 26** addresses 7 MEDIUM findings (residual blocking I/O, ACL gaps, rate limiting, key rotation).
**Sprint 27** addresses 12 LOW findings (PII redaction, TOCTOU, accessibility, test gaps, DevOps).
**Sprint 28** addresses 11 remaining deferred backlog items (i18n, architecture refactoring, E2E testing, RAI).
**After S28:** Run final audit to verify 0 CRITICAL / 0 HIGH / 0 MEDIUM / 0 LOW.

### Audit_Final Sprints (COMPLETE)

```
Sprint 30 (P0) ──► Sprint 31 (P1) ──► Sprint 32 (P2) ──► Sprint 33 (P3)
   4 tickets         8 tickets          7 tickets          7 tickets
   ~2 days           ~4–5 days          ~3–4 days           ~1–2 days
```

**Sprint 30** fixes 2 regressions (useHardware polling, check_sentry_consent) + 2 UX blockers (consent F5 race, sidebar scroll).
**Sprint 31** fixes 7 HIGH issues + implements ALL stubs (Miracast WinRT Option B, ECRAM WMI discovery, audio PKEY_Device_FriendlyName, WiFi CREATE_NO_WINDOW, touchpad HID logging, startup autostart, EC Debug dev-gate).
**Sprint 32** fixes 7 MEDIUM issues (adaptive brightness, auto-discovery, IoT bridge, battery powercfg, diagnostics note, IoT retry, credential allowlist).
**Sprint 33** fixes 7 LOW issues (logging upgrades, dead code removal, dependency cleanup, dev-gating, documentation).
**After S33:** Zero stubs remaining in codebase. Zero CRITICAL / HIGH / MEDIUM / LOW from Audit_Final.

### miControl Audit Sprints (EXECUTION RECORDED; S63 REOPENED)

```
Sprint 34 (P0) ──► Sprint 35 (P1) ──► Sprint 36 (P2) ──► Sprint 37 (P1)
   7 tickets         7 tickets          4 tickets          5 tickets
   ~2–3 days         ~3–4 days          ~2–3 days           ~5–10 days
```

**Sprint 34** fixes the Auth Bridge UAC regression (4 root causes) + 3 quick-win UX fixes (tray CSS, resize guard, audio poll).
**Sprint 35** fixes temperature sensors (remove fabricated fallback), volume slider dirty flag, tray CSS verification, WiFi scan delay.
**Sprint 36** fixes 4 security issues (atomic writes, directory ACL, diag_ps gate, USERNAME spoofing).
**Sprint 37** implements audio device switching (IPolicyConfig COM) and replaces netsh with WlanAPI for locale-independent WiFi.
**After S37:** All 6 user-reported bugs fixed. All 4 security issues resolved. Zero UAC prompts in production.
**After S55 (stabilization round):** Bridge service crash-loop eliminated (fix17–fix19), UAC storm on power-profile change eliminated (stable pipe), tray Open App restored (fix13/16), debloater UI released with tiered removal + state badges.

**Sprint 39:** Replaces the undocumented OS Turbo process call with the public Windows API, enforces the authenticated bridge-only write boundary, and sequences reversible state, measured statistics, bounded policy, anonymous logs, optional BYOK approval, UI integration, and runtime audit. **Sprint 39 is complete** — all 9 tickets implemented and validated, 11/11 health gates passed, commits local only (not published).

### OS Turbo Redesign (Sprint 39) — ✅ Complete

| Requirement / finding                                       | Sprint | Ticket  | Coverage statement                                                                |
| ----------------------------------------------------------- | ------ | ------- | --------------------------------------------------------------------------------- |
| Public EcoQoS API and authenticated bridge-only admin write | S39    | S39-001 | Public `SetProcessInformation` API; no OS Turbo UAC or scheduled-task fallback    |
| Reversible process and power state                          | S39    | S39-002 | Versioned process identity snapshot and prior power scheme restoration            |
| Measured bounded status and worker                          | S39    | S39-003 | Bounded periodic worker with measured action outcomes                             |
| Deterministic statistical learner                           | S39    | S39-004 | Welford windows, minimum samples, matched controls, confidence and effect sizes   |
| Safe adaptive policy                                        | S39    | S39-005 | Allowlist, fixed bounds, cooldown, rollback, and no thermal/EC writes             |
| Anonymous before/after logs                                 | S39    | S39-006 | Privacy filters, retention caps, byte caps, and privacy tests                     |
| Optional BYOK suggestions                                   | S39    | S39-007 | Structured validation and explicit approval, independent of deterministic control |
| Four-locale integrated UI                                   | S39    | S39-008 | Localized status and approval/error workflows with frontend tests                 |
| Runtime and full-gate verification                          | S39    | S39-009 | Disposable-child smoke test, bridge boundary audit, and complete health gates     |

**Sprint 39 validation:** 83 `hw::os_turbo` tests + 67 module tests (stats 18, policy 18, log 14, suggest 17) pass with 0 failures; `node scripts/health-check.mjs` reports 11/11. Sprint commit `a9ad6ce`.

**Sprint 39 follow-up (not blocking):** the statistics, policy, log, and BYOK modules are implemented and unit-tested but not yet wired into the live sweep loop or surfaced in the UI.

### Post-S39 Fix Batches — Reconstructed from Local Commits

| Sprint | Focus                                                                               | Status / Evidence                            |
| ------ | ----------------------------------------------------------------------------------- | -------------------------------------------- |
| 55     | Bridge stability, UAC storm, tray launch, debloater                                 | ✅ `6e645c9`                                 |
| 57     | Route updater install through the SYSTEM bridge                                     | ✅ `7b0bc3e`                                 |
| 58     | Stop bridge service and confirm SCM state before update                             | ✅ `62eb768`                                 |
| 59     | Installer service restart and locked-binary recovery                                | ✅ `b2ea390`, `fec3911`                      |
| 60     | Performance-mode grouping and measured TDP presentation                             | ✅ `96dd66b`                                 |
| 61     | Write EC power limit through MiInterface WMAA                                       | ✅ `0b71bf4`                                 |
| 62     | Switch performance modes through no-prompt bridge path                              | ✅ `3bd8dd6`                                 |
| 63     | Tray popup geometry and opaque theme surfaces                                       | 🔄 Reopened by `4386c5e`; tracked by S65-001 |
| 64     | Brightness, tray DPI, IoT recovery, battery/WMI polling, logs, and pipe reliability | ✅ `d74d1ad`                                 |

These entries are retrospective status records from the local Git history; they
do not imply that the commits were pushed or released. Sprint 63 is not treated
as complete because the latest selector change reverted its theme-specificity
fix.

### Open-Work Audit — Sprints 63–65 (2026-10-14)

#### Sprint 63 — Tray popup geometry and theme surfaces — REOPENED

| Item         | Detail                                                                                                                                                                          |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Fix in scope | Opaque surface specificity for dark mode (commit `353b388` over-qualified the `.tray-window` selector).                                                                         |
| Reopen       | `4386c5e` reverted the selector back to `.tray-window`; it can lose to the `:root:not([data-theme=light])` rule (specificity 0,2,0 vs 0,1,0) and re-introduce ghosted surfaces. |
| Status       | Tracked by S65-001. No new issue on the 171px width collapse or DPI-stale geometry was filed in S64; those remain closed.                                                       |
| Lost items   | None — the S63 spec is fully captured by S65-001. The only open risk is the reverted CSS selector.                                                                              |

#### Sprint 64 — Stability batch (brightness, DPI, logs, pipe) — COMPLETE

| Item                                                                        | Evidence                                                                                                                                     |
| --------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| Brightness no-op on null IGCL device                                        | `d54a302`                                                                                                                                    |
| Tray DPI-stale geometry + log spam                                          | `d54a302`                                                                                                                                    |
| IoT recovery timeout, battery poll throttle, log rotation, touchpad timeout | `33dd779`                                                                                                                                    |
| ecram pipe lost connections (single-instance, recreated every 500 ms)       | `d74d1ad`                                                                                                                                    |
| Status                                                                      | ✅ Complete — all tickets in the batch are closed. S64 is not split into sub-plans; commits are the source of truth. No lost items detected. |

#### Sprint 65 — Installer reliability + S63 regression — PLANNED

All 30 open acceptance boxes are in [S65](sprint-65-p0-installer-reliability/sprint.md):

| Ticket  | Scope                                         | Open |
| ------- | --------------------------------------------- | ---- |
| S65-001 | Restore S63 tray theme fix + regression test  | 4    |
| S65-002 | Bootstrap shell for legacy NSIS installs      | 7    |
| S65-003 | Unify CI + local release contracts            | 6    |
| S65-004 | Fail-closed payload preflight                 | 6    |
| S65-005 | Staged install + rollback on critical failure | 7    |

#### Cross-Sprint Findings Audit (S1–S39)

No lost, blocked, or abandoned tickets found outside S65. The legacy plans
(S1–S29) keep their unchecked acceptance boxes as historical planning text;
execution is covered by the documented commits:

| Group      | Evidence                                                                                                                                                |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| S4–S6, S11 | Inline commits `785bbdd`, `a01a728`, `042f657`, `966ae13`, `822299d` + overview rows S16–S19                                                            |
| S7–S9      | `822299d`, `966ae13`, `e0da3ad`                                                                                                                         |
| S10        | `5ced26c` (all 22 high-priority findings)                                                                                                               |
| S12        | `dbcc014` (all 20 low-priority findings)                                                                                                                |
| S13–S15    | `b3277d2`, `ddfee3c`, `0807cbd`                                                                                                                         |
| S16–S29    | Overview completion rows with hashes `de5e344` … `bec0d42`                                                                                              |
| S30–S39    | Reconciled in this pass with commit hashes `b597d66`, `9a8f9fc`, `8918a9c`, `4fafc07`, `309bfa3`, `45b0d5a`, `0a67897`, `b47dea0`, `5341ac0`, `a9ad6ce` |
| S38        | ✅ Complete on `feature/IoT-hardware` (no plan row in overview — minor documentation gap, not a lost ticket)                                            |

**Disposal decisions (items that no longer make sense):**

- S39 follow-up (live-sweep wiring of stats/policy/log/BYOK): still valuable, keep as a non-blocking item on the S39 record — do not discard.
- S28 deferred items: all addressed in S28; no re-open.
- Legacy unchecked boxes S1–S29: documentation debt only; no functional work remains per the overview. Do not re-execute; keep as-is.
- `ecram_shim.exe` (S1-013 finding): dormant on the live system, outside the current install flow; no action unless a new IoTDriver version ships it again.
- All other S38 cross-device tickets: completed in `5341ac0`; no loss detected.

### Sprint 65 — Installer Release Reliability and S63 Regression

|   # | Priority | Focus                                                     | Tickets | Effort     | Status     | Plan                                                   |
| --: | :------: | --------------------------------------------------------- | ------: | ---------- | ---------- | ------------------------------------------------------ |
|  65 |    P0    | Decoupled installer productionization and tray regression |       5 | ~1–2 weeks | ⏳ Planned | [sprint](sprint-65-p0-installer-reliability/sprint.md) |

The current payload path cannot update legacy installations that have no setup
shell, CI still generates `latest.json` for the NSIS executable while the local
publisher targets the payload ZIP, payload assembly skips missing files, and
shell installation extracts in place while treating several critical failures
as warnings. S65 also restores the S63 tray selector fix that was reverted by
`4386c5e`. No release, installer run, or publication is authorized by this plan.

| Ticket  | Planned outcome                                                                                                                                                        |
| ------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| S65-001 | Restore and regression-test tray surface specificity under OS dark mode.                                                                                               |
| S65-002 | Bootstrap the permanent shell for legacy NSIS installations before payload updates.                                                                                    |
| S65-003 | Make CI and the local release publisher produce the same shell, payload, signature, and updater manifest contract.                                                     |
| S65-004 | Validate required payload contents and signatures before stopping services or modifying installed files; fail on missing release artifacts.                            |
| S65-005 | Stage installation, classify critical versus optional steps, and roll back binaries/services on critical failure instead of returning success after a partial install. |

**Sprint 65 validation gate:** run `pnpm run health-check`, build/test the
`micontrol-setup` workspace package, validate release asset names and updater
manifest against the assembled payload, and run non-destructive migration,
failure-injection, and dark-theme regression tests. Do not publish or install
the resulting artifacts without explicit authorization.

### Sprint 66 — Elevated Helper Crash-Loop and Face Service Recovery

|   # | Priority | Focus                                                                                                                                                                         | Tickets | Effort  | Status              | Plan                                                     |
| --: | :------: | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------: | ------- | ------------------- | -------------------------------------------------------- |
|  66 |    P0    | `--elevated` helper combase crash-loop (9,718 WER crashes since 2026-10-20), task-path churn, face service self-heal, bridge latency bounds, IoTSvc churn, LocalDumps capture |       6 | ~1 week | ✅ Complete (local) | [sprint](sprint-66-p0-crashloop-face-recovery/sprint.md) |

Live-machine log forensics (2026-10-23) found six issues not previously
tracked: the scheduled-task elevated helper crashes at exit in `combase.dll`
(fixed offset `0xd398d`, ~90–150/hour, still active); routine polls fall back
to the task path despite a healthy bridge pipe; the MiControlFace SCM entry
vanished (16:48 today) and `ensure_face_service` never reinstalls it; one
`read_thermal_readings` dispatch blocked for 652 s; `ensure_ecram_service`
destructively recreates IoTSvc up to 37×/day; and WER LocalDumps captures
zero dumps for the 9,718 crashes.

| Ticket  | Planned outcome                                                                                               |
| ------- | ------------------------------------------------------------------------------------------------------------- |
| S66-001 | Helper exits cleanly (COM teardown fixed); zero new WER crashes over 24 h.                                    |
| S66-002 | Transient pipe failures retry the bridge before spawning the scheduled-task helper.                           |
| S66-003 | `ensure_face_service` reinstalls the service when the binary exists but the SCM entry is gone (rate-limited). |
| S66-004 | Internal deadline for thermal-read dispatch in the bridge.                                                    |
| S66-005 | Probe-before-recreate + cooldown for the destructive IoTSvc recovery.                                         |
| S66-006 | LocalDumps actually captures micontrol.exe crash dumps.                                                       |

**Sprint 66 validation gate:** run `pnpm run health-check` and
`cargo test --manifest-path src-tauri/Cargo.toml --lib` per ticket.
Live-machine validation (WER event deltas, service state) requires a new
build installed on the user machine — coordinate explicitly; do not publish
without approval.

---

## Deferred Backlog — Reviewed & Triaged (Sprint 28)

A thorough investigation of all deferred items was conducted post-v3 audit. Each item was verified against the current codebase to determine if it was already resolved or still an open issue.

### ✅ Already Resolved (6 items — no action needed)

| Finding                            | Resolution                                                              |
| ---------------------------------- | ----------------------------------------------------------------------- |
| U4: OnboardingWizard accessibility | ✅ Resolved in S24-010 (role="dialog", focus trap, Escape handler)      |
| U5: ConsentDialog focus ring       | ✅ Resolved (global `*:focus-visible` CSS, no bare `outline: none`)     |
| Q10: Duplicate type definitions    | ✅ Resolved (each type defined once, imported where needed)             |
| S7: shell:default capability       | ✅ Resolved (only `core:default` granted, no shell permissions exposed) |
| S13: Support scripts in root       | ✅ Resolved (all scripts in `scripts/` directory)                       |
| S14: Rust crate versions           | ✅ Acceptable (Cargo.lock committed, standard practice)                 |

### ⚠️ Partially Resolved (3 items — minor actions in S28)

| Finding                 | Status                             | Action                                        |
| ----------------------- | ---------------------------------- | --------------------------------------------- |
| Q11: TODO in hotkeys.rs | Roadmap items, not broken code     | Documented as known roadmap (S28-009 context) |
| T16-T19: Stability      | osd.rs expect() already in S27-006 | No additional action                          |
| D11-D12: DevOps         | CI comprehensive, LICENSE missing  | S28-010                                       |

### ❌ Still Open Issues (11 items — addressed in Sprint 28)

| Finding                         | Sprint 28 Ticket   | Description                                                      |
| ------------------------------- | ------------------ | ---------------------------------------------------------------- |
| U6: Hardcoded English           | S28-001, S28-002   | EcrDebugPanel zero i18n; AiConfigForm PRESET labels + aria-label |
| Q6: useSettings God object      | S28-004            | 430-line hook mixing settings, AI, consent                       |
| Q16: Co-located types           | S28-003            | 17+ types in hook files, should be in src/types/                 |
| S10: write_iot_hex hardcoded    | S28-008            | 9 EC RAM offsets hardcoded in source                             |
| A5: Global statics              | S28-007            | 48 statics across 14 files                                       |
| A6: useSettings scope violation | S28-004            | buildPrompt() in useSettings.ts                                  |
| A7: IoT IPC granular            | S28-005            | ~25 IoT commands could be consolidated                           |
| A9-A12: Minor architecture      | S28-006            | hotkeys.rs ~2700 lines, should be split                          |
| T11: No E2E testing             | S28-009            | No playwright/cypress/puppeteer                                  |
| D11-D12: LICENSE missing        | S28-010            | README references MIT LICENSE but file doesn't exist             |
| R6-R12: RAI gaps                | S28-011 to S28-014 | No feedback, caching, model logging, or AI docs                  |

---

## Expected Outcomes

After completing Sprints 16-19:

| Metric                      | Current | Target    | Delta |
| --------------------------- | ------- | --------- | ----- |
| Rust tests                  | 193     | 230+      | +37   |
| Frontend tests              | 3 files | 13+ files | +10   |
| CRITICAL findings           | 10      | 0         | -10   |
| HIGH findings               | 24      | 0         | -24   |
| MEDIUM findings (addressed) | 60      | ~40       | -40   |
| Overall grade               | C+      | B+        | +2    |
| `Mutex::lock().unwrap()`    | 12+     | 0         | -12   |
| `#[allow(dead_code)]`       | 21      | ≤5        | -16   |
| Hardcoded English strings   | ~15     | 0         | -15   |
| External font requests      | Yes     | No        | ✅    |
| Pre-commit enforcement      | No      | Yes       | ✅    |
| GDPR Art. 17 (erasure)      | ❌      | ✅        | ✅    |
| GDPR Art. 20 (portability)  | ❌      | ✅        | ✅    |

---

_Updated 2026-10-14. Sprints 30–39 were reconciled against their completion commits and recorded health gates; post-S39 fix batches were reconstructed from local Git history. Earlier legacy sprint sections remain as previously recorded. Sprint 63 has a known reopened regression; Sprint 65 is planned._
