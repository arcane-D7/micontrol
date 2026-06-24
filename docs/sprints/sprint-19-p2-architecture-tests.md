# Sprint 19 — P2 Architecture, Code Quality & Test Coverage

**Sprint ID:** S19
**Priority:** P2 — MEDIUM (Next sprint cycle)
**Estimated tickets:** 18
**Estimated effort:** 3–4 days
**Base branch:** `master` (after S18 merge)
**Source:** `docs/stability-report-2026-06-24-post-sprints-13-15.md` — P2 Recommendations #19–#30 + Architecture/Code Quality findings

---

## Sprint Goal

Address medium-term architectural debt: extract shared WMI utilities, split the `touchpad.rs` god-module, add comprehensive test coverage (Rust + frontend), remove blanket error conversions, clean up dead code, and improve code quality across the board.

---

## Tickets

### S19-01: Create shared WMI field extraction utility

**Severity:** HIGH (architectural)
**Finding:** A2 — Duplicated across 8 modules
**Files:** `src-tauri/src/util/wmi_extract.rs` (new), `src-tauri/src/hw/{battery,display,fan,performance,processes,system_info,update,wmi_cache}.rs`
**Tasks:**

- [ ] Create `util/wmi_extract.rs` with helper functions:
  - `extract_u32(map: &HashMap<String, wmi::Variant>, key: &str) -> Option<u32>`
  - `extract_i32(map: &HashMap<String, wmi::Variant>, key: &str) -> Option<i32>`
  - `extract_u64(map: &HashMap<String, wmi::Variant>, key: &str) -> Option<u64>`
  - `extract_string(map: &HashMap<String, wmi::Variant>, key: &str) -> Option<String>`
  - `extract_bool(map: &HashMap<String, wmi::Variant>, key: &str) -> Option<bool>`
  - `extract_u32_or(map: &HashMap<String, wmi::Variant>, key: &str, default: u32) -> u32`
  - `extract_string_or(map: &HashMap<String, wmi::Variant>, key: &str, default: &str) -> String`
- [ ] Add unit tests for each helper
- [ ] Refactor `battery.rs` to use the new utilities
- [ ] Refactor `display.rs` to use the new utilities
- [ ] Refactor `fan.rs` to use the new utilities
- [ ] Refactor `performance.rs` to use the new utilities
- [ ] Refactor `processes.rs` to use the new utilities
- [ ] Refactor `system_info.rs` to use the new utilities
- [ ] Refactor `update.rs` to use the new utilities
- [ ] Refactor `wmi_cache.rs` to use the new utilities
      **Acceptance:** No duplicated WMI field extraction logic. All hw modules use shared utilities.

---

### S19-02: Split touchpad.rs into module directory

**Severity:** HIGH (architectural)
**Finding:** A1 — God-module mixing 6 concerns
**Files:** `src-tauri/src/hw/touchpad/` (new directory)
**Tasks:**

- [ ] Create `hw/touchpad/mod.rs` — public API and re-exports
- [ ] Create `hw/touchpad/hid.rs` — HID device communication
- [ ] Create `hw/touchpad/gestures.rs` — gesture processing (swipe, edge-slide)
- [ ] Create `hw/touchpad/hook.rs` — WH_MOUSE_LL low-level hook
- [ ] Create `hw/touchpad/charger.rs` — charger detection
- [ ] Create `hw/touchpad/registry.rs` — registry persistence
- [ ] Create `hw/touchpad/state.rs` — global statics and state management
- [ ] Move all code from `touchpad.rs` into appropriate submodules
- [ ] Update imports in `commands/` and `lib.rs`
- [ ] Verify all tests still pass
      **Acceptance:** `touchpad.rs` is split into focused submodules. No behavior changes.

---

### S19-03: Remove blanket From<String>/From<&str> for HardwareError

**Severity:** MEDIUM
**Finding:** Q4, E6 — Type erosion
**Files:** `src-tauri/src/hw/errors.rs`
**Tasks:**

- [ ] Remove `impl From<String> for HardwareError`
- [ ] Remove `impl From<&str> for HardwareError`
- [ ] Update all call sites that relied on blanket conversion
- [ ] Replace with explicit `HardwareError::Generic(format!(...))` or appropriate variant
- [ ] Fix `ai.rs` and `hotkeys.rs` to return `HardwareResult<T>` instead of `Result<T, String>`
- [ ] Add `HardwareError::AiRequest` and `HardwareError::HotkeyConfig` variants if needed
      **Acceptance:** No blanket string-to-error conversions. All command functions return `HardwareResult<T>`.

---

### S19-04: Migrate ai.rs to HardwareResult

**Severity:** MEDIUM
**Finding:** A4
**Files:** `src-tauri/src/commands/ai.rs`
**Tasks:**

- [ ] Change `analyze_system` return type from `Result<String, String>` to `HardwareResult<String>`
- [ ] Change `test_connection` return type from `Result<String, String>` to `HardwareResult<String>`
- [ ] Add `HardwareError::AiConsentDenied`, `HardwareError::AiRequestFailed`, `HardwareError::AiResponseInvalid` variants
- [ ] Update frontend to handle new error types
- [ ] Add tests for each error path
      **Acceptance:** `ai.rs` uses `HardwareResult<T>`. No `String` error returns.

---

### S19-05: Migrate hotkeys.rs commands to HardwareResult

**Severity:** MEDIUM
**Finding:** A4
**Files:** `src-tauri/src/commands/hotkeys.rs`
**Tasks:**

- [ ] Change all command return types from `Result<T, String>` to `HardwareResult<T>`
- [ ] Add `HardwareError::HotkeyConfigInvalid`, `HardwareError::HotkeyActionBlocked` variants if needed
- [ ] Update frontend error handling
- [ ] Add tests for error paths
      **Acceptance:** `hotkeys.rs` commands use `HardwareResult<T>`.

---

### S19-06: Extract spawn_blocking boilerplate into helper

**Severity:** MEDIUM
**Finding:** Q9 — Repeated 25+ times
**Files:** `src-tauri/src/util/blocking.rs` (new), `src-tauri/src/commands/{system,hardware}.rs`
**Tasks:**

- [ ] Create `util/blocking.rs` with:
  ```rust
  pub async fn run_blocking<T, F>(f: F) -> HardwareResult<T>
  where
      F: FnOnce() -> HardwareResult<T> + Send + 'static,
      T: Send + 'static,
  {
      tokio::task::spawn_blocking(f)
          .await
          .map_err(|e| HardwareError::TaskJoin(e.to_string()))?
  }
  ```
- [ ] Add `HardwareError::TaskJoin(String)` variant
- [ ] Refactor `commands/system.rs` to use `run_blocking()`
- [ ] Refactor `commands/hardware.rs` to use `run_blocking()`
- [ ] Reduce boilerplate from ~25 sites to single-line calls
      **Acceptance:** `spawn_blocking` boilerplate is extracted. Command functions are cleaner.

---

### S19-07: Add Rust unit tests for wmi_cache.rs

**Severity:** MEDIUM
**Finding:** T4
**Files:** `src-tauri/src/hw/wmi_cache.rs`
**Tasks:**

- [ ] Add test for `with_wmi()` success path (mock WMI connection)
- [ ] Add test for `with_wmi()` failure path
- [ ] Add test for cache invalidation
- [ ] Add test for `is_connection_error()` with various error types
- [ ] Add test for thread_local initialization
      **Acceptance:** `wmi_cache.rs` has ≥5 unit tests.

---

### S19-08: Add Rust unit tests for elev_bridge.rs

**Severity:** MEDIUM
**Finding:** T5
**Files:** `src-tauri/src/elev_bridge.rs` (or `elevated.rs`)
**Tasks:**

- [ ] Add test for HMAC signing/verification round-trip
- [ ] Add test for nonce replay detection
- [ ] Add test for invalid HMAC rejection
- [ ] Add test for nonce persistence
- [ ] Mock the subprocess communication for unit testing
      **Acceptance:** `elev_bridge.rs` has ≥5 unit tests.

---

### S19-09: Add Rust unit tests for ai_usage.rs

**Severity:** LOW
**Finding:** T13
**Files:** `src-tauri/src/util/ai_usage.rs`
**Tasks:**

- [ ] Add test for `record_usage()` — verify counters increment
- [ ] Add test for `get_usage()` — verify correct stats returned
- [ ] Add test for `reset_usage()` — verify counters reset to 0
- [ ] Add test for concurrent access (thread safety)
      **Acceptance:** `ai_usage.rs` has ≥4 unit tests.

---

### S19-10: Add Rust unit tests for debug_log.rs

**Severity:** LOW
**Finding:** T14
**Files:** `src-tauri/src/debug_log.rs`
**Tasks:**

- [ ] Add test for log initialization
- [ ] Add test for log level filtering
- [ ] Add test for log file creation
      **Acceptance:** `debug_log.rs` has ≥3 unit tests.

---

### S19-11: Add frontend tests for major components

**Severity:** HIGH
**Finding:** T3 — Only 3 test files for 30+ components
**Files:** `src/__tests__/`
**Tasks:**

- [ ] Add test for `OverviewTab` — renders hardware data, updates on hardware change
- [ ] Add test for `BatteryTab` — displays charge level, health, charging status
- [ ] Add test for `FanTab` — mode switching, fan curve display
- [ ] Add test for `DisplayTab` — brightness slider, HDR toggle
- [ ] Add test for `SettingsTab` — language switching, theme toggle
- [ ] Add test for `ConsentDialog` — grant/deny flow, focus trapping
- [ ] Add test for `OnboardingWizard` — step navigation, completion
- [ ] Add test for `ErrorBoundary` — error catching, reload button
- [ ] Add test for `TrayPopup` — quick actions, volume slider
- [ ] Add test for `WiFiManager` — password input, connection status
      **Acceptance:** Frontend test count ≥ 13 files (3 existing + 10 new). All major components tested.

---

### S19-12: Fix duplicate Props interface in MainWindow.tsx

**Severity:** HIGH
**Finding:** Q2
**Files:** `src/pages/MainWindow.tsx`
**Tasks:**

- [ ] Remove the duplicate `interface Props` declaration (lines 33 and 64)
- [ ] Keep only one `Props` interface
- [ ] Verify TypeScript compilation still passes
      **Acceptance:** Only one `Props` interface in `MainWindow.tsx`.

---

### S19-13: Clean up #[allow(dead_code)] annotations

**Severity:** MEDIUM
**Finding:** Q3 — 21 annotations
**Files:** Multiple
**Tasks:**

- [ ] Audit each `#[allow(dead_code)]` annotation
- [ ] For truly unused code: remove the code
- [ ] For code used only in tests: add `#[cfg(test)]` or `#[allow(dead_code)]` with a comment explaining why
- [ ] For `RegKeyGuard`: either use it or remove it
- [ ] For `from_display()`: either use it or remove it
- [ ] Target: reduce from 21 to ≤5 annotations (with justified comments)
      **Acceptance:** ≤5 `#[allow(dead_code)]` annotations remain, each with a justification comment.

---

### S19-14: Add SAFETY comments to all unsafe blocks

**Severity:** MEDIUM
**Finding:** A8
**Files:** `src-tauri/src/{startup.rs, performance.rs, touchpad.rs (or touchpad/)}`
**Tasks:**

- [ ] Audit all `unsafe` blocks in the codebase
- [ ] Add `// SAFETY:` comment before each `unsafe` block explaining:
  - Why the operation is safe
  - What invariants are being upheld
  - What could go wrong if the unsafe block is misused
- [ ] Verify all unsafe blocks have SAFETY comments
      **Acceptance:** 100% of `unsafe` blocks have `// SAFETY:` comments.

---

### S19-15: Fix ErrorBoundary hardcoded APP_VERSION

**Severity:** LOW
**Finding:** Q17, E19 (may already be fixed in S16-08)
**Files:** `src/components/ErrorBoundary.tsx`
**Tasks:**

- [ ] If not already fixed in S16-08, replace `APP_VERSION = '0.1.0'` with:
  - `const APP_VERSION = import.meta.env.PACKAGE_VERSION ?? '1.0.0';`
  - Or read from `package.json` via Vite's `define` config
- [ ] Verify version displays correctly in error report
      **Acceptance:** `APP_VERSION` is dynamically read, not hardcoded.

---

### S19-16: Add data export / portability feature (GDPR Art. 20)

**Severity:** MEDIUM
**Finding:** V4
**Files:** `src-tauri/src/commands/privacy.rs` (new or existing), `src/pages/PrivacySettings.tsx` (new or existing)
**Tasks:**

- [ ] Add `export_user_data` Tauri command that:
  - Collects all user data files (hardware profile, hotkeys, consent, AI logs, settings)
  - Bundles them into a ZIP archive
  - Returns the file path or saves to user-chosen location
- [ ] Add "Download My Data" button in Settings → Privacy
- [ ] Add file save dialog using Tauri's dialog API
- [ ] Add test for export functionality
      **Acceptance:** Users can export all their data as a ZIP file. GDPR Art. 20 compliance.

---

### S19-17: Derive separate sub-keys using HKDF

**Severity:** MEDIUM
**Finding:** V6 — HMAC key reused for 3 purposes
**Files:** `src-tauri/src/util/auth.rs`, `src-tauri/src/util/consent_audit.rs`, `src-tauri/src/hw/iotservice.rs`
**Tasks:**

- [ ] Add `hkdf` crate to `Cargo.toml`
- [ ] In `auth.rs`, add `derive_subkey(purpose: &str) -> [u8; 32]` using HKDF-SHA256
- [ ] Use separate sub-keys for:
  - HMAC signing (elevated bridge)
  - Audit log integrity
  - WiFi password encryption
- [ ] Migrate existing code to use derived sub-keys
- [ ] Add backward compatibility: if old key exists, use it directly (no sub-key derivation)
- [ ] Add tests for key derivation
      **Acceptance:** Each cryptographic purpose uses a separate derived sub-key.

---

### S19-18: Health check verification and commit

**Severity:** N/A (process)
**Tasks:**

- [ ] Run all 9 health checks
- [ ] Fix any failures
- [ ] Commit with message: `feat(sprint-19): architecture, code quality, and test coverage improvements (P2)`
- [ ] Verify test count significantly increased (target: 230+ tests)
      **Acceptance:** 9/9 health checks pass. Test count ≥ 230.

---

## Sprint Exit Criteria

- [ ] Shared WMI extraction utility exists and is used by all hw modules
- [ ] `touchpad.rs` is split into a module directory
- [ ] No blanket `From<String>`/`From<&str>` for `HardwareError`
- [ ] `ai.rs` and `hotkeys.rs` use `HardwareResult<T>`
- [ ] `spawn_blocking` boilerplate extracted into helper
- [ ] `wmi_cache.rs` has ≥5 unit tests
- [ ] `elev_bridge.rs` has ≥5 unit tests
- [ ] `ai_usage.rs` has ≥4 unit tests
- [ ] `debug_log.rs` has ≥3 unit tests
- [ ] Frontend has ≥13 test files
- [ ] No duplicate `Props` interface in `MainWindow.tsx`
- [ ] ≤5 `#[allow(dead_code)]` annotations with justification
- [ ] All `unsafe` blocks have SAFETY comments
- [ ] Data export feature works
- [ ] Separate sub-keys for each cryptographic purpose
- [ ] 9/9 health checks pass
- [ ] Test count ≥ 230
