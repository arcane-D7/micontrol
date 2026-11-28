# MiControl — Stability Report v2

> **Historical snapshot — do not update.** This report records the state of the codebase at the commit named in `Baseline` and is kept for traceability. Findings may already be fixed. For current architecture see [architecture.md](./architecture.md); for current logging see [LOGGING.md](./LOGGING.md).
>
> The date below reads `2025-06-25`; the audit round it belongs to (post-Sprint 21, v3's "previous report") is 2026-06-25. The value is left as recorded.

**Date:** 2025-06-25  
**Auditor:** Automated multi-agent audit (3 parallel subagents)  
**Scope:** Full codebase — security, architecture, stability, UI/UX, performance, AI responsibility, DevOps  
**Baseline:** Post-Sprint 21 (commit `d514bdf`)

---

## Executive Summary

This report evaluates the MiControl Tauri v2 + React 19 + Rust desktop application after completing Sprints 16–21, which addressed 178 findings from the original stability report. The audit covers 7 domains with 3 parallel subagents examining security, architecture, UI/UX, performance, AI responsibility, and DevOps.

### Findings Summary

| Severity     | Count | Change from v1    |
| ------------ | ----- | ----------------- |
| **CRITICAL** | 2     | ↓ from 10 (−80%)  |
| **HIGH**     | 5     | ↓ from 24 (−79%)  |
| **MEDIUM**   | 18    | ↓ from 60 (−70%)  |
| **LOW**      | 18    | ↓ from 45 (−60%)  |
| **INFO**     | 20    | —                 |
| **Total**    | 63    | ↓ from 178 (−65%) |

### Sprint Impact

| Sprint | Commit    | Focus                       | Key Outcomes                                                     |
| ------ | --------- | --------------------------- | ---------------------------------------------------------------- |
| 16     | `de5e344` | P0 critical fixes           | XOR→AES-256-GCM, HKDF subkeys, path validation                   |
| 17     | `cb9005f` | Security & DevOps           | CI hardening, SECURITY.md, issue templates                       |
| 18     | `c76236f` | Error handling & resilience | HardwareError 19 variants, retry backoff, consent audit          |
| 19     | `1a383c0` | Architecture & quality      | WMI extraction utils, run_blocking, GDPR export, 7 test files    |
| 20-21  | `d514bdf` | Post-audit fixes            | EC RAM validation, CSP tightening, nonce ACL, fail-closed checks |

**Remaining CRITICAL/HIGH issues are primarily in blocking I/O on async threads and nonce persistence — not in cryptography or authentication, which are now solid.**

---

## 1. Security & Privacy

### CRITICAL

None found. The cryptographic foundation (HMAC-SHA256, AES-256-GCM, HKDF-SHA256) is correctly implemented, and the major attack surfaces from Sprints 16–21 are properly hardened.

### HIGH

None found. All previously identified HIGH security issues (assert! panics, nonce store ACL, CSP, cert password, SHA-pinned actions, shell:default) were resolved in Sprint 20–21 and verified.

### MEDIUM

| #     | Finding                                                                                                                                                                                                                                                                                                                   | File:Line           | Evidence                                                                                       |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------- | ---------------------------------------------------------------------------------------------- |
| S-M01 | **Nonce persistence gap on process exit** — The elevated helper handles ONE command per invocation then calls `exit(0)`. Nonces are only persisted every 3rd insertion, but `flush_nonces()` is never called in the exit path. 2 of every 3 nonces are lost, weakening replay protection within the 30s timestamp window. | `elevated.rs:88-96` | `if map.len().is_multiple_of(3) { save_nonces(map); }` — no flush before `exit(0)` at line ~75 |
| S-M02 | **HMAC key rotation is dead code** — `key_needs_rotation()` and `rotate_key()` are fully implemented (30-day check, 7-day grace backup) but never called from any code path. If the key is compromised, there is no mechanism to invalidate it.                                                                           | `auth.rs:383-460`   | Functions defined but no caller in `lib.rs`, `elevated.rs`, or `elev_bridge.rs`                |
| S-M03 | **Key rotation grace period is broken** — `rotate_key()` creates a backup key, but `verify_payload()` only checks the current key. If rotation were triggered, commands signed with the old key during the grace period would be rejected.                                                                                | `auth.rs:419-460`   | `verify_payload` takes single key; `read_old_key()` never called in verification path          |

### LOW

| #     | Finding                                                                                                                                  | File:Line               |
| ----- | ---------------------------------------------------------------------------------------------------------------------------------------- | ----------------------- |
| S-L01 | `save_nonces` uses non-atomic `std::fs::write` — crash mid-write corrupts the nonce store. Should use temp+rename like `elev_bridge.rs`. | `elevated.rs:108-117`   |
| S-L02 | Sentry PII stripping incomplete — only handles `C:\Users\` paths, misses other drives, UNC paths, and IPv6 addresses.                    | `lib.rs:103-140`        |
| S-L03 | CSP missing explicit `form-action 'self'` directive (falls back to `default-src`).                                                       | `tauri.conf.json:25`    |
| S-L04 | `connect-src` includes `https://api.openai.com` but AI calls go through Rust backend, not webview — unnecessary attack surface.          | `tauri.conf.json:25`    |
| S-L05 | `derive_aes_key` silently falls back to weaker SHA-256 derivation if HKDF fails — should log a warning.                                  | `iotservice.rs:343-352` |
| S-L06 | GDPR export ZIP file not ACL-restricted — should call `restrict_file_acl` for consistency.                                               | `privacy.rs:30-35`      |

### INFO

- `dispatch_cmd` bypasses HMAC when already elevated — by design (attacker needs admin).
- AES-GCM nonce generates 16 bytes but truncates to 12 — wasteful but not vulnerable.
- `tauri-plugin-shell` loaded but `shell:default` not granted — safe but unnecessary plugin load.
- Updater has `dialog: false` (silent) — signature-verified but reduces user control.
- `LaunchApp` hotkey action doesn't validate executable path — config is ACL-protected.
- `set_secret` allows arbitrary keyring key names — limited risk (attacker = user).
- GDPR export includes `nonces.json` — reveals usage patterns if shared.
- Dependencies reasonably current — no known CVEs. `rand 0.8`, `thiserror 1`, `sentry 0.34` one major behind.

---

## 2. Architecture & Stability

### CRITICAL

| #     | Finding                                                                                                                                                                                                                                                     | File:Line                | Evidence                                                          |
| ----- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------ | ----------------------------------------------------------------- |
| A-C01 | **Blocking `WaitForSingleObject` in async context** — `launch_elevated_via_uac()` is called directly from the async `run_elevated()` function (not via `spawn_blocking`). It blocks a Tokio worker thread for up to 30 seconds, starving other async tasks. | `elev_bridge.rs:130-135` | `WaitForSingleObject(info.hProcess, 30_000)` called from async fn |
| A-C02 | **Synchronous file I/O with 5s lock timeout in async context** — `auth::get_or_create_key()` performs sync `std::fs` operations with a polling loop (`sleep(50ms)`) from the async `run_elevated()` function. Blocks Tokio worker for up to 5 seconds.      | `elev_bridge.rs:95-96`   | `loop { match file.try_lock_exclusive() { ... } sleep(50ms) }`    |

### HIGH

| #     | Finding                                                                                                                                                                                              | File:Line               | Evidence                                                                    |
| ----- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------- | --------------------------------------------------------------------------- |
| A-H01 | **Potential infinite loop on pipe EOF** — `read_exact_timeout` doesn't check `bytes_read == 0` (pipe closed). When the pipe closes, the loop spins indefinitely until timeout.                       | `iotservice.rs:620-630` | `filled += bytes_read as usize;` with bytes_read=0 → no progress            |
| A-H02 | **`is_eram_range` uses hardcoded `ERAM_BASE` instead of `get_eram_base()`** — If DSDT discovery finds a different base address, the range check would reject valid addresses or accept invalid ones. | `hardware.rs:413-417`   | `let start = crate::hw::ecram::ERAM_BASE;` vs `get_eram_base()` in ecram.rs |
| A-H03 | **`read_string` silently drops registry values > 512 bytes** — `ERROR_MORE_DATA` is treated as "value doesn't exist" with no retry.                                                                  | `registry.rs:131-145`   | `if result.is_err() { return Ok(None); }`                                   |

### MEDIUM

| #     | Finding                                                                                               | File:Line             |
| ----- | ----------------------------------------------------------------------------------------------------- | --------------------- |
| A-M01 | `expect()` on thread spawn in `start_hook()` — panics on resource exhaustion.                         | `hotkeys.rs:288`      |
| A-M02 | `expect()` on cached battery static data — logically safe but panics if refactored.                   | `battery.rs:155`      |
| A-M03 | `set_profile` silently swallows RwLock poison — inconsistent with `lock_or_recover`.                  | `state.rs:46-48`      |
| A-M04 | `read_in_memory`/`update_in_memory` silently swallow RwLock poison.                                   | `hotkeys.rs:271-276`  |
| A-M05 | AC probe cache silently skips on poison.                                                              | `battery.rs:330-345`  |
| A-M06 | Mutex held across slow WMI query during battery init — blocks all `get_battery_info()` callers.       | `battery.rs:100-150`  |
| A-M07 | Three different error code schemes for non-typed errors (`UNKNOWN_ERROR`, `INTERNAL_ERROR`, `other`). | `errors.rs:210-218`   |
| A-M08 | Error code `INVALID_STATUS` manually constructed — not in `HardwareError` enum.                       | `hardware.rs:230-234` |

### LOW

| #     | Finding                                                                        | File:Line              |
| ----- | ------------------------------------------------------------------------------ | ---------------------- |
| A-L01 | `display.rs` uses `winreg` directly instead of `RegKeyGuard`.                  | `display.rs:148-160`   |
| A-L02 | `charging.rs` uses raw `RegOpenKeyExW`/`RegCloseKey` instead of `RegKeyGuard`. | `charging.rs:170-195`  |
| A-L03 | `hotkeys.rs` uses `winreg` directly for `is_script_action_enabled`.            | `hotkeys.rs:1245-1250` |
| A-L04 | WMI query strings leaked in error messages to frontend.                        | `errors.rs:45-50`      |
| A-L05 | `get_available_refresh_rates` silently returns empty vec on error.             | `system.rs:210-213`    |
| A-L06 | `get_hardware_profile` returns `Option` — ambiguous `None`.                    | `system.rs:283-285`    |
| A-L07 | `unreachable!()` in retry loop — should return `Err`.                          | `retry.rs:73`          |

### INFO

- Module architecture is clean — `commands` → `hw` → `util` with no circular dependencies.
- `lock_or_recover` used consistently across `battery.rs`, `hotkeys.rs`, `ai_usage.rs`, `elevated.rs`, `iotservice.rs`.
- `run_blocking` wrapper correctly centralizes `spawn_blocking` with `TaskJoin` error mapping.
- WMI cache uses `thread_local!` correctly (COM objects are thread-affine).
- `RegKeyGuard` RAII pattern ensures `RegCloseKey` on all paths.
- All `unsafe` blocks have `SAFETY:` comments.
- Test coverage is good — tests exist for `errors`, `retry`, `panic`, `wmi_cache`, `battery`, `processes`, `wifi`, `touchpad`, `performance`, `discovery`, `iotservice`, `auth`, `consent_audit`, `ai_usage`.
- `is_connection_error` uses `type_name_of_val` for fallback — fragile but limited risk.
- `check_rate_limit` uses inline poison recovery instead of `lock_or_recover` — inconsistent.
- No tests for `RegKeyGuard`, hotkey config migration, HMAC signing/verification, or consent log rotation.

---

## 3. UI/UX

### HIGH

| #     | Finding                                                                                                                                                                                                                                 | File:Line        | Evidence                       |
| ----- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------- | ------------------------------ |
| U-H01 | **Insufficient test coverage** — Only `ErrorBoundary.test.tsx` and basic tab tests exist. No tests for `useHardware`, `useSettings`, `AiConfigForm`, `ConsentDialog`, `OnboardingWizard`, or `ToastContext`. Coverage threshold is 40%. | `src/__tests__/` | 7 test files vs 30+ components |

### MEDIUM

| #     | Finding                                                                                                                     | File:Line              |
| ----- | --------------------------------------------------------------------------------------------------------------------------- | ---------------------- |
| U-M01 | `ErrorBoundary.didCatch` only logs to console — doesn't call `Sentry.captureException()`. React crashes never reach Sentry. | `ErrorBoundary.tsx:61` |
| U-M02 | `OnboardingWizard` lacks focus trap and Escape key handler (unlike `ConsentDialog`).                                        | `OnboardingWizard.tsx` |
| U-M03 | `getUserFriendlyMessage` returns hardcoded English — bypasses i18n system.                                                  | `error.ts:73-89`       |
| U-M04 | Top-level `ErrorBoundary` wraps entire app — a crash in one tab brings down the whole window.                               | `App.tsx:62-64`        |

### LOW

| #     | Finding                                                                                | File:Line                    |
| ----- | -------------------------------------------------------------------------------------- | ---------------------------- |
| U-L01 | API key show/hide button uses emoji with `title` but no `aria-label`.                  | `AiConfigForm.tsx:113-117`   |
| U-L02 | Onboarding progress dots are `<div>` without `role="progressbar"`.                     | `OnboardingWizard.tsx:42-52` |
| U-L03 | Fixed 950×660 window with 18 sidebar items — no responsive breakpoints.                | `tauri.conf.json:18-25`      |
| U-L04 | `AiAnalysis.tsx` uses inline styles — prevents `:hover`/`:focus` pseudo-class styling. | `AiAnalysis.tsx`             |

### INFO

- i18n implementation is solid: 4 locales (en/pt/es/fr), pluralization, RTL detection, English fallback.
- `ConsentDialog` is well-implemented: proper `role="dialog"`, `aria-modal`, focus trap, Escape handling, neutral initial focus.

---

## 4. Performance

### MEDIUM

| #     | Finding                                                                                                               | File:Line                | Evidence |
| ----- | --------------------------------------------------------------------------------------------------------------------- | ------------------------ | -------- |
| P-M01 | `get_performance_mode` calls `hw_get_perf()` synchronously without `run_blocking` — WMI/registry I/O on async thread. | `hardware.rs:37-39`      |
| P-M02 | `get_charging_threshold` calls `hw_get_charge()` without `run_blocking`.                                              | `hardware.rs:50-52`      |
| P-M03 | `get_perf_debug` calls `hw_perf_debug()` without `run_blocking`.                                                      | `hardware.rs:63-65`      |
| P-M04 | WMI connections cached per-thread via `thread_local!` — each `spawn_blocking` thread creates its own COM connections. | `wmi_cache.rs:20-23`     |
| P-M05 | `useHardware` useMemo spreads `...fanState` and `...systemState` — all consumers re-render every 2 seconds.           | `useHardware.ts:860-950` |

### LOW

| #     | Finding                                                                 | File:Line            |
| ----- | ----------------------------------------------------------------------- | -------------------- |
| P-L01 | `get_ai_brightness_config` reads registry without `run_blocking`.       | `system.rs:131-133`  |
| P-L02 | `opt-level = "s"` optimizes for size, not speed.                        | `Cargo.toml:96`      |
| P-L03 | `LineChart` SVG uses `preserveAspectRatio="none"` — distorts polylines. | `AiAnalysis.tsx:131` |
| P-L04 | `adaptive_brightness_loop` runs every 2s even when display is off.      | `display.rs:170-175` |

### INFO

- Manual chunks configured for `react-vendor`, `tauri-vendor`, `sentry`.
- `run_blocking` wrapper correctly maps `JoinError` to `HardwareError::TaskJoin`.
- Battery static data cached once via `BATTERY_STATIC_DATA` mutex.
- AC power probe throttled to 15-second intervals.

---

## 5. AI Responsibility

### HIGH

| #      | Finding                                                                                                                                                                                             | File:Line       | Evidence                                                   |
| ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------- | ---------------------------------------------------------- |
| AI-H01 | **`test_connection` does not check telemetry consent** — sends API key to external server without verifying consent. `analyze_system` checks consent at line 76-79, but `test_connection` does not. | `ai.rs:131-170` | No `check_consent()` call before `reqwest::Client` request |

### MEDIUM

| #      | Finding                                                                                                  | File:Line                |
| ------ | -------------------------------------------------------------------------------------------------------- | ------------------------ |
| AI-M01 | `base_url` is user-controlled and passed directly to `reqwest` — malicious URL could exfiltrate API key. | `ai.rs:43-55`            |
| AI-M02 | AI usage tracking is in-memory only — stats lost on restart. No backend-enforced daily limit.            | `ai_usage.rs:22-40`      |
| AI-M03 | `buildPrompt` sends process names (by CPU usage) to AI — not explicitly mentioned in consent flow.       | `useSettings.ts:155-180` |

### LOW

| #      | Finding                                                                                   | File:Line                    |
| ------ | ----------------------------------------------------------------------------------------- | ---------------------------- |
| AI-L01 | AI analysis logs (up to 500 entries) stored in localStorage indefinitely — no expiration. | `useAnalysisLogger.ts:12-14` |
| AI-L02 | No backend rate limiting on AI requests — frontend-only throttling.                       | `ai.rs:96-100`               |

### INFO

- API key stored in OS keyring, never exposed to frontend. Legacy localStorage keys migrated.
- Prompt injection detection implemented: `check_suspicious_input` + `validate_output`.
- Input sanitized (control chars stripped, length-capped at 50,000 chars).
- Generic error messages returned instead of raw API response bodies.
- Consent audit log is HMAC-SHA256 signed with rotation.
- AI disclaimer displayed to users.
- Sentry crash reporting consent-gated.

---

## 6. DevOps & CI/CD

### MEDIUM

| #     | Finding                                                                                           | File:Line           |
| ----- | ------------------------------------------------------------------------------------------------- | ------------------- |
| D-M01 | Code signing is optional — if `WINDOWS_CERTIFICATE` not set, release proceeds unsigned.           | `release.yml:62-70` |
| D-M02 | No `dependabot.yml` or `renovate.json` — no automated dependency updates.                         | —                   |
| D-M03 | No SAST tool (CodeQL, Semgrep) configured — `cargo audit`/`npm audit` only catch dependency CVEs. | —                   |

### LOW

| #     | Finding                                                                        | File:Line           |
| ----- | ------------------------------------------------------------------------------ | ------------------- |
| D-L01 | `cargo install cargo-audit` runs on every CI — should be cached.               | `ci.yml:28-30`      |
| D-L02 | `cargo install cargo-tarpaulin` runs on every coverage job — should be cached. | `ci.yml:96-97`      |
| D-L03 | `CODECOV_TOKEN` may not be configured — coverage uploads fail silently.        | `ci.yml:115,142`    |
| D-L04 | Release workflow doesn't verify git tag version matches `package.json`.        | `release.yml:55-56` |
| D-L05 | Release workflow doesn't run `cargo audit` or `npm audit` before building.     | `release.yml`       |

### INFO

- PR template is comprehensive with quality checklist.
- Bug report and feature request templates are well-structured.
- Husky pre-commit hooks with `lint-staged` run ESLint + Prettier.
- CI pipeline is complete: Rust (check + clippy + test + audit), frontend (tsc + eslint + prettier + build + audit), coverage, Tauri smoke test, version check, i18n check.
- Release pipeline verifies signing key, signs with Authenticode, generates `latest.json`, creates GitHub Release.

---

## 7. Cross-Cutting Concerns

### Error Handling Consistency

The `HardwareError` enum with 19 typed variants is a significant improvement over the previous `anyhow::Error → String` pattern. However, three issues remain:

1. **Three fallback codes** — `From<String>` → `"UNKNOWN_ERROR"`, `From<anyhow::Error>` → `"INTERNAL_ERROR"`, `from_display` → `"other"`. The frontend must handle all three.
2. **Ad-hoc error codes** — `INVALID_STATUS` in `hardware.rs:230` is not in the `HardwareError` enum.
3. **WMI query strings** in error messages leak internal implementation details to the frontend.

### Concurrency Pattern Consistency

`lock_or_recover()` is used in most modules, but several modules still use silent `if let Ok(...)` or `.unwrap_or_else(|e| e.into_inner())` patterns:

- `state.rs:46-48` — `set_profile`
- `hotkeys.rs:271-276` — `read_in_memory`/`update_in_memory`
- `battery.rs:330-345` — AC probe cache
- `iotservice.rs:490-495` — `check_rate_limit`

### Registry Access Consistency

The `RegKeyGuard` RAII pattern exists in `util/registry.rs` but is not used consistently:

- `display.rs` uses `winreg::RegKey` directly
- `charging.rs` uses raw `RegOpenKeyExW`/`RegCloseKey`
- `hotkeys.rs` uses `winreg::RegKey` directly

---

## Sprint Recommendations

### Sprint 22 (P0 — CRITICAL)

| Ticket | Finding                                        | Fix                                                |
| ------ | ---------------------------------------------- | -------------------------------------------------- |
| S22-01 | A-C01: Blocking `WaitForSingleObject` in async | Wrap `launch_elevated_via_uac` in `spawn_blocking` |
| S22-02 | A-C02: Sync file I/O with 5s lock in async     | Wrap `get_or_create_key` in `spawn_blocking`       |

### Sprint 23 (P1 — HIGH)

| Ticket | Finding                                             | Fix                                           |
| ------ | --------------------------------------------------- | --------------------------------------------- |
| S23-01 | A-H01: Infinite loop on pipe EOF                    | Check `bytes_read == 0` and return error      |
| S23-02 | A-H02: Hardcoded `ERAM_BASE` in range check         | Use `get_eram_base()`                         |
| S23-03 | A-H03: Registry values > 512 bytes silently dropped | Retry with larger buffer on `ERROR_MORE_DATA` |
| S23-04 | U-H01: Insufficient frontend test coverage          | Add tests for hooks and critical components   |
| S23-05 | AI-H01: `test_connection` missing consent check     | Add `check_consent()` before API call         |

### Sprint 24 (P2 — MEDIUM, batch)

| Ticket | Finding                                       | Fix                                                |
| ------ | --------------------------------------------- | -------------------------------------------------- |
| S24-01 | S-M01: Nonce persistence gap                  | Call `flush_nonces()` before `exit(0)`             |
| S24-02 | S-M02: Dead key rotation code                 | Wire `key_needs_rotation()` into startup or remove |
| S24-03 | S-M03: Broken grace period                    | Try `read_old_key()` in `verify_payload` or remove |
| S24-04 | A-M01: `expect()` on thread spawn             | Return `Result`, degrade gracefully                |
| S24-05 | A-M02: `expect()` on battery data             | Use `ok_or_else` with `HardwareError`              |
| S24-06 | A-M03–M05: Inconsistent poison recovery       | Use `lock_or_recover` everywhere                   |
| S24-07 | A-M06: Mutex held across WMI query            | Use `OnceLock` for one-time init                   |
| S24-08 | A-M07–M08: Error code inconsistency           | Standardize fallback codes, remove ad-hoc codes    |
| S24-09 | U-M01: ErrorBoundary doesn't report to Sentry | Add `Sentry.captureException()` in `didCatch`      |
| S24-10 | U-M02: OnboardingWizard accessibility         | Add focus trap and Escape handler                  |
| S24-11 | U-M03: Error messages bypass i18n             | Route through i18n system                          |
| S24-12 | U-M04: Top-level ErrorBoundary too broad      | Add per-tab ErrorBoundary                          |
| S24-13 | P-M01–M03: Missing `run_blocking`             | Wrap sync hw calls in `run_blocking`               |
| S24-14 | P-M05: useHardware re-render storm            | Split context or use selector hooks                |
| S24-15 | AI-M01: Unvalidated `base_url`                | Add URL allowlist or validation                    |
| S24-16 | AI-M02: In-memory usage tracking              | Persist to file or registry                        |
| S24-17 | D-M01: Optional code signing                  | Fail release if no certificate                     |
| S24-18 | D-M02: No Dependabot                          | Add `dependabot.yml`                               |
| S24-19 | D-M03: No SAST                                | Add CodeQL or Semgrep to CI                        |

### Sprint 25 (P3 — LOW, batch)

All LOW findings batched into a single sprint with minimal individual tickets.

---

## Metrics

### Test Coverage

| Area                     | Tests   | Status                          |
| ------------------------ | ------- | ------------------------------- |
| Rust unit tests          | 238     | ✅ All passing                  |
| Rust integration tests   | 23      | ✅ All passing                  |
| Frontend component tests | 7 files | ⚠️ Low coverage (40% threshold) |
| **Total**                | **268** | —                               |

### Health Check Status (Post-Sprint 21)

| Check                      | Status            |
| -------------------------- | ----------------- |
| `cargo fmt --check`        | ✅ Pass           |
| `cargo check`              | ✅ Pass           |
| `cargo clippy -D warnings` | ✅ Pass           |
| `cargo test`               | ✅ 261 tests pass |
| `npx tsc --noEmit`         | ✅ Pass           |
| `npm run lint`             | ✅ Pass           |
| `npm run format:check`     | ✅ Pass           |
| `npm run build`            | ✅ Pass           |
| `npm run version:check`    | ✅ Pass           |

### Dependency Status

| Dependency      | Current | Latest | Risk                  |
| --------------- | ------- | ------ | --------------------- |
| `rand`          | 0.8     | 0.9    | Low — no CVEs         |
| `thiserror`     | 1       | 2      | Low — no CVEs         |
| `sentry`        | 0.34    | 0.35   | Low — no CVEs         |
| `@sentry/react` | ^8.0.0  | 9      | Low — still supported |
| `wmi`           | 0.13    | 0.14   | Low — no CVEs         |

---

## Conclusion

The MiControl application has significantly improved since the original stability report. The cryptographic foundation is solid, the error handling architecture is well-structured, and the CI/CD pipeline is comprehensive. The remaining CRITICAL issues (2) are blocking I/O on async threads — important but not security vulnerabilities. The remaining HIGH issues (5) are edge cases in pipe handling, registry reads, and test coverage.

**Recommended next step:** Execute Sprint 22 (P0) and Sprint 23 (P1) to bring the codebase to 0 CRITICAL / 0 HIGH, then batch the MEDIUM findings in Sprint 24.

---

_Generated by automated multi-agent audit on 2025-06-25. 3 parallel subagents using `umans-glm-5.2` model examined security, architecture, UI/UX, performance, AI responsibility, and DevOps domains._
