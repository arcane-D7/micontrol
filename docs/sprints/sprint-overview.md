# Sprint Plan Overview — Post-Sprints 13-15 Stability Report Remediation

**Created:** 2026-06-25
**Source:** `docs/stability-report-2026-06-24-post-sprints-13-15.md`
**Total findings to address:** 63 (out of 178 total)
**Total estimated tickets:** 63
**Total estimated effort:** 9–13 days

---

## Sprint Summary

| Sprint | Priority    | Focus                      | Tickets | Effort   | File                                 |
| ------ | ----------- | -------------------------- | ------- | -------- | ------------------------------------ |
| 16     | P0 CRITICAL | Pre-release blockers       | 15      | 2–3 days | `sprint-16-p0-critical-fixes.md`     |
| 17     | P1 HIGH     | Security & DevOps          | 15      | 2–3 days | `sprint-17-p1-security-devops.md`    |
| 18     | P1 HIGH     | Error handling & stability | 15      | 2–3 days | `sprint-18-p1-error-stability.md`    |
| 19     | P2 MEDIUM   | Architecture & tests       | 18      | 3–4 days | `sprint-19-p2-architecture-tests.md` |

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

```
Sprint 16 (P0 Critical) ──────► Sprint 17 (P1 Security/DevOps) ──────► Sprint 18 (P1 Error/Stability) ──────► Sprint 19 (P2 Arch/Tests)
   15 tickets, 2-3 days              15 tickets, 2-3 days                   15 tickets, 2-3 days                  18 tickets, 3-4 days
```

**Sprints 16-17 can partially overlap** — S17-07 through S17-10 (docs/templates) are independent of S16 code changes.

**Sprints 18-19 are sequential** — S19-02 (touchpad split) depends on S18-02 (touchpad lock_or_recover) being done first.

---

## Deferred to Sprint 20+ (Not in this plan)

These findings are lower priority and can be addressed in a future sprint:

- U4: OnboardingWizard accessibility (role="dialog", focus trap)
- U5: ConsentDialog focus ring
- U6: Hardcoded English in EcrDebugPanel and AiConfigForm
- U7-U22: Remaining UI/UX MEDIUM/LOW findings
- Q6: useSettings God object refactor
- Q10: Duplicate type definitions
- Q11: TODO tech debt in hotkeys.rs
- Q16: Co-located type definitions (no src/types/ directory)
- S7: shell:default capability granularity
- S10: write_iot_hex hardcoded safe list
- S13: Support scripts in root
- S14: Rust crate versions not pinned
- A5: Global statics proliferation
- A6: useSettings scope violation (AI prompt builder)
- A7: IoT IPC commands excessively granular
- A9-A12: Minor architecture improvements
- T11: No E2E testing
- T16-T19: Minor stability improvements
- D11-D12: Minor DevOps improvements
- R6-R12: Minor RAI improvements

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

_Generated 2026-06-25 based on `docs/stability-report-2026-06-24-post-sprints-13-15.md`_
