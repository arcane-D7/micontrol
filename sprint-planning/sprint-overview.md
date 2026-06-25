# Sprint Plan Overview — Stability Report Remediation (v1 + v2)

**Created:** 2026-06-25
**Last Updated:** 2026-06-25 (Sprint 28 added — deferred backlog triaged)
**Sources:**

- v1: `docs/stability-report-2026-06-24-post-sprints-13-15.md` (Sprints 16–19)
- v2: `docs/STABILITY_REPORT_v2.md` (Sprints 22–25)
- v3: `docs/STABILITY_REPORT_v3.md` (Sprints 26–27)
- Deferred: `sprint-planning/sprint-28-p3-deferred-backlog/sprint.md` (Sprint 28)
- Sprints 20–21: Post-v1 audit CRITICAL/HIGH fixes (committed d514bdf)

**Total findings addressed:** 63 (v1) + 44 (v2) + 19 (v3) + 20 (deferred) = 146
**Total estimated tickets:** 63 (v1) + 44 (v2) + 19 (v3) + 14 (S28) = 140
**Total estimated effort:** 9–13 days (v1) + 9–12 days (v2) + ~10 days (v3+S28) = 28–35 days

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

### v2 Sprints (from Stability Report v2) — 🔄 PLANNED

| Sprint | Priority    | Focus                          | Tickets | Effort  | Status                                        | File                                        |
| ------ | ----------- | ------------------------------ | ------- | ------- | --------------------------------------------- | ------------------------------------------- |
| 22     | P0 CRITICAL | Async blocking I/O             | 2       | ~1 day  | ✅ 3a73f4b                                    | `sprint-22-p0-async-blocking/sprint.md`     |
| 23     | P1 HIGH     | Stability & security edges     | 5       | ~3 days | ✅ fef49f9                                    | `sprint-23-p1-stability-security/sprint.md` |
| 24     | P2 MEDIUM   | Architecture/UI/Perf/AI/DevOps | 19      | ~5 days | ✅ b4e467b (Batch A) / ✅ 5bd819b (Batch B+C) | `sprint-24-p2-medium-batch/sprint.md`       |
| 25     | P3 LOW      | Polish & consistency           | 18      | ~3 days | ✅ 100a1d2                                    | `sprint-25-p3-low-polish/sprint.md`         |

### v3 Sprints (from Stability Report v3) — 🔄 PLANNED

| Sprint | Priority  | Focus                           | Tickets | Effort  | Status     | File                                      |
| ------ | --------- | ------------------------------- | ------- | ------- | ---------- | ----------------------------------------- |
| 26     | P2 MEDIUM | Residual blocking I/O, ACL gaps | 8       | ~3 days | 🔄 Planned | `sprint-26-p2-medium-residual/sprint.md`  |
| 27     | P3 LOW    | Polish & defense-in-depth       | 11      | ~2 days | 🔄 Planned | `sprint-27-p3-low-polish-v3/sprint.md`    |
| 28     | P3 LOW    | Deferred backlog cleanup        | 14      | ~5 days | 🔄 Planned | `sprint-28-p3-deferred-backlog/sprint.md` |

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

### v3 Sprints (PLANNED)

```
Sprint 26 (P2) ──► Sprint 27 (P3) ──► Sprint 28 (P3) ──► Final Audit
   8 tickets         11 tickets         14 tickets
   ~3 days           ~2 days            ~5 days
```

**Sprint 26** addresses 7 MEDIUM findings (residual blocking I/O, ACL gaps, rate limiting, key rotation).
**Sprint 27** addresses 12 LOW findings (PII redaction, TOCTOU, accessibility, test gaps, DevOps).
**Sprint 28** addresses 11 remaining deferred backlog items (i18n, architecture refactoring, E2E testing, RAI).
**After S28:** Run final audit to verify 0 CRITICAL / 0 HIGH / 0 MEDIUM / 0 LOW.

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

_Generated 2026-06-25 based on `docs/stability-report-2026-06-24-post-sprints-13-15.md` (v1), `docs/STABILITY_REPORT_v2.md` (v2), `docs/STABILITY_REPORT_v3.md` (v3), and deferred backlog review. Sprints 16–25 complete. Sprints 26–28 planned._
