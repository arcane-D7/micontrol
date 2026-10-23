# Sprint 16 — P0 Critical Fixes (Pre-Release Blockers)

**Sprint ID:** S16
**Priority:** P0 — CRITICAL (Must fix before release)
**Estimated tickets:** 15
**Estimated effort:** 2–3 days
**Base branch:** `master`
**Source:** `docs/stability-report-2026-06-24-post-sprints-13-15.md` — Top 10 Priorities + P0 Recommendations

---

## Sprint Goal

Fix all 10 CRITICAL findings from the post-Sprints-13-15 stability report. These are pre-release blockers that affect security, privacy, performance, and functionality. No release should ship until all P0 items are resolved.

---

## Tickets

### S16-01: Fix incomplete data deletion (GDPR Art. 17)

**Severity:** CRITICAL
**Finding:** V1, S4 — `delete_all_user_data` misses 6 data stores
**Files:** `src-tauri/src/util/data_deletion.rs`
**Tasks:**

- [x] Add deletion of `hardware_profile.json`
- [x] Add deletion of `hotkeys.json`
- [x] Add deletion of `nonces.json`
- [x] Add deletion of `elev_key.bin`
- [x] Add deletion of `elev_key.bin.old`
- [x] Add deletion of `ai_config.json` (if exists)
- [x] Add Tauri command/event to clear `localStorage` on the frontend
- [x] Update `DeleteDataReport` struct with new fields
- [x] Add unit tests for each deleted file
      **Acceptance:** `delete_all_user_data` removes all 12+ data stores. Tests verify each file is deleted.

---

### S16-02: Fix KEYRING_SERVICE mismatch in ai.rs

**Severity:** CRITICAL
**Finding:** R1 — AI feature is completely non-functional
**Files:** `src-tauri/src/commands/ai.rs`
**Tasks:**

- [x] Change `KEYRING_SERVICE` from `"micontrol"` to `"com.mipc.micontrol"` (matching `credentials.rs`)
- [x] Also fix `data_deletion.rs` which uses `"micontrol"` for keyring entries
- [x] Verify `test_connection` command also uses correct service name
- [x] Add integration test that stores consent via `set_secret` and reads it via `get_telemetry_consent`
      **Acceptance:** AI analysis feature works end-to-end after user grants consent.

---

### S16-03: Replace OnceLock + .expect() panic in battery.rs

**Severity:** CRITICAL
**Finding:** P1, E1, T1 — Battery module permanently poisoned on WMI failure
**Files:** `src-tauri/src/hw/battery.rs`
**Tasks:**

- [x] Change `BATTERY_STATIC_DATA` from `OnceLock<BatteryStaticData>` to `OnceLock<Result<BatteryStaticData, anyhow::Error>>`
- [x] Replace `.expect("static battery data init failed")` with proper error propagation
- [x] On init failure, return `HardwareError::WmiQuery` instead of panicking
- [x] Allow retry: if first init failed, subsequent calls should retry (use `OnceLock<Result<...>>` with `Err` = retry)
- [x] Alternative: use `Mutex<Option<BatteryStaticData>>` instead of `OnceLock` to allow retry
- [x] Add test for WMI failure scenario (mock or simulate)
      **Acceptance:** Battery module does not panic on transient WMI failure. Retries on next call.

---

### S16-04: Remove Google Fonts CDN, use local fonts only

**Severity:** CRITICAL
**Finding:** P2 — Double font loading (CDN + local), breaks offline
**Files:** `index.html`, `src/styles/globals.css`
**Tasks:**

- [x] Remove `<link rel="preconnect">` for Google Fonts
- [x] Remove `<link rel="stylesheet">` for Google Fonts CSS
- [x] Add `<link rel="preload" as="font" type="font/woff2" crossorigin>` for Outfit and JetBrains Mono
- [x] Verify `@font-face` in `globals.css` covers all weights (300, 400, 500, 600, 700 for Outfit; 400, 600 for JetBrains Mono)
- [x] Test offline mode — fonts should load from local files
- [x] Verify no FOUT (flash of unstyled text) on cold start
      **Acceptance:** No external font requests. All fonts load from local woff2 files. Offline mode works.

---

### S16-05: Add lint-staged configuration

**Severity:** CRITICAL
**Finding:** D1 — Pre-commit enforcement is imaginary
**Files:** `package.json`
**Tasks:**

- [x] Add `lint-staged` configuration block to `package.json`:
  ```json
  "lint-staged": {
    "src/**/*.{ts,tsx}": ["eslint --fix", "prettier --write"],
    "src/**/*.{json,css,md}": ["prettier --write"],
    "src-tauri/src/**/*.rs": ["rustfmt"]
  }
  ```
- [x] Add `cargo fmt` to `.husky/pre-commit` hook (before `npx tsc --noEmit`)
- [x] Test pre-commit hook triggers on staged files
      **Acceptance:** Pre-commit hook lints and formats staged files. Unformatted code is blocked.

---

### S16-06: i18n — Replace hardcoded English in TrayPopup

**Severity:** CRITICAL
**Finding:** U1 — Hardcoded strings visible in every tray session
**Files:** `src/pages/TrayPopup.tsx`, `src/i18n/{en,pt,es,fr}.json`
**Tasks:**

- [x] Add i18n keys: `tray.crossDevice`, `tray.mute`, `tray.unmute`, `tray.muted`, `tray.on`, `tray.audio`, `tray.fanAuto`, `tray.fanFixed`, `tray.fanOff`
- [x] Replace `'Cross-Device'` → `t('tray.crossDevice')`
- [x] Replace `'Unmute'` / `'Mute'` → `t('tray.unmute')` / `t('tray.mute')`
- [x] Replace `'Muted'` / `'On'` → `t('tray.muted')` / `t('tray.on')`
- [x] Replace `'Auto'` / `'Fixed'` / `'Off'` → `t('tray.fanAuto')` / `t('tray.fanFixed')` / `t('tray.fanOff')`
- [x] Add translations for all 4 locales (en, pt, es, fr)
      **Acceptance:** No hardcoded English in TrayPopup. All strings use `t()`.

---

### S16-07: i18n — Replace hardcoded English theme labels

**Severity:** CRITICAL
**Finding:** U2 — Theme toggle shows English regardless of locale
**Files:** `src/pages/MainWindow.tsx`, `src/i18n/{en,pt,es,fr}.json`
**Tasks:**

- [x] Add i18n keys: `theme.auto`, `theme.light`, `theme.dark`
- [x] Replace `THEME_LABELS` constant with a function that calls `t()`
- [x] Add translations for all 4 locales
- [x] Verify theme toggle updates labels when locale changes
      **Acceptance:** Theme labels are localized. No hardcoded English.

---

### S16-08: Refactor ErrorBoundary to use useI18n instead of static imports

**Severity:** CRITICAL
**Finding:** U3 — Imports all 4 locale JSONs at module level
**Files:** `src/components/ErrorBoundary.tsx`
**Tasks:**

- [x] Remove static imports of `en`, `pt`, `es`, `fr` JSON files
- [x] Remove `LOCALES` constant and `getLocaleStrings()` function
- [x] Use `localStorage.getItem('micontrol_lang')` to determine locale, then dynamically import only the needed locale
- [x] Or: use a simpler approach — import only `en` as fallback, and read the current locale from localStorage to pick the right file
- [x] Add `type="button"` to both buttons
- [x] Add `role="alert"` to the `<pre>` error display
- [x] Fix `APP_VERSION` from `'0.1.0'` to read from `package.json` or `import.meta.env`
- [x] Fix GitHub issues URL from `github.com/mafsc/miPC` to `github.com/Freitas-MA/miPC`
      **Acceptance:** ErrorBoundary does not import all 4 locale files. Version is correct. Buttons have `type="button"`.

---

### S16-09: Add HTTP timeout to AI requests

**Severity:** CRITICAL
**Finding:** R2 — No timeout, UI freezes indefinitely
**Files:** `src-tauri/src/commands/ai.rs`
**Tasks:**

- [x] Replace `reqwest::Client::new()` with `reqwest::Client::builder().timeout(Duration::from_secs(30)).build()`
- [x] Apply to both `analyze_system` and `test_connection` commands
- [x] Add `use std::time::Duration;` import
- [x] Return a user-friendly error message on timeout
      **Acceptance:** AI requests timeout after 30 seconds. UI does not freeze indefinitely.

---

### S16-10: Wire ErrorResponse.code into frontend error handling

**Severity:** CRITICAL
**Finding:** E5 — Typed error system is a dead letter
**Files:** `src/hooks/useHardware.ts`, `src/types/error.ts` (new), `src/components/ErrorMessage.tsx` (new or existing)
**Tasks:**

- [x] Create `src/types/error.ts` with `ErrorResponse` interface matching the Rust enum:
  ```typescript
  interface ErrorResponse {
    code: string;
    message: string;
  }
  ```
- [x] Export stable error code constants (e.g., `ERROR_CODES = { WMI_QUERY: 'WMI_QUERY', ... }`)
- [x] In `useHardware.ts` catch blocks, parse the error and extract `.code`
- [x] Show user-facing toast/notification based on error code
- [x] Replace generic `console.error()` with structured error handling
- [x] Add at least 3 error code mappings (WMI failure, permission denied, device not found)
      **Acceptance:** Frontend reads `ErrorResponse.code` and shows appropriate user-facing messages.

---

### S16-11: Fix keyring service name in data_deletion.rs

**Severity:** CRITICAL (blocks S16-01 and S16-02)
**Finding:** R1 (related) — `data_deletion.rs` also uses wrong service name
**Files:** `src-tauri/src/util/data_deletion.rs`
**Tasks:**

- [x] Change `keyring::Entry::new("micontrol", ...)` to `keyring::Entry::new("com.mipc.micontrol", ...)`
- [x] Apply to both `openai_api_key` and `telemetry_consent` entries
- [x] Verify deletion actually removes credentials
      **Acceptance:** Data deletion removes credentials from the correct keyring service.

---

### S16-12: Add CSP directives (object-src, base-uri)

**Severity:** MEDIUM (promoted for security hardening)
**Finding:** S6 — CSP missing directives
**Files:** `src-tauri/tauri.conf.json`
**Tasks:**

- [x] Add `object-src 'none'` to CSP
- [x] Add `base-uri 'self'` to CSP
- [x] Verify app still functions correctly with stricter CSP
      **Acceptance:** CSP includes `object-src 'none'; base-uri 'self'`.

---

### S16-13: Fix README URLs from placeholder to actual repo

**Severity:** HIGH (promoted for release readiness)
**Finding:** D4 — All links broken
**Files:** `README.md`
**Tasks:**

- [x] Replace `github.com/user/miPC` → `github.com/Freitas-MA/miPC` everywhere
- [x] Fix CI badge URL
- [x] Fix version badge URL
- [x] Fix clone URL
- [x] Fix releases link
- [x] Fix issues link in ErrorBoundary (cross-ref S16-08)
      **Acceptance:** All URLs in README point to `github.com/Freitas-MA/miPC`.

---

### S16-14: Fix Spanish locale typo and French diacritics

**Severity:** LOW (quick win, batched with i18n work)
**Finding:** U19, U14
**Files:** `src/i18n/es.json`, `src/i18n/fr.json`
**Tasks:**

- [x] Fix `"Portátils"` → `"Portátiles"` in `es.json`
- [x] Audit and fix missing diacritics in `fr.json`
- [x] Run `npm run version:check` to verify locale key consistency
      **Acceptance:** No typos in Spanish locale. French locale has proper diacritics.

---

### S16-15: Health check verification and commit

**Severity:** N/A (process)
**Tasks:**

- [x] Run all 9 health checks:
  - `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
  - `cargo check --manifest-path src-tauri/Cargo.toml`
  - `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
  - `cargo test --manifest-path src-tauri/Cargo.toml`
  - `npx tsc --noEmit`
  - `npm run lint`
  - `npm run format:check`
  - `npm run build`
  - `npm run version:check`
- [x] Fix any failures
- [x] Commit with message: `feat(sprint-16): fix all P0 critical findings from stability report`
- [x] Verify test count increased (target: 200+ tests)
      **Acceptance:** 9/9 health checks pass. All P0 findings resolved.

---

## Sprint Exit Criteria

- [x] All 10 CRITICAL findings from stability report are resolved
- [x] 9/9 health checks pass
- [x] No new clippy warnings
- [x] No new TypeScript errors
- [x] Test count ≥ 200
- [x] `delete_all_user_data` deletes all data stores
- [x] AI feature works end-to-end
- [x] Battery module does not panic on WMI failure
- [x] No external font requests
- [x] Pre-commit hook enforces lint/format
- [x] No hardcoded English in TrayPopup or theme labels
