# Sprint 29 — P0 CRITICAL+HIGH: Pre-Release Security & GDPR Fixes

## Sprint Metadata

| Field                 | Value                                                                    |
| --------------------- | ------------------------------------------------------------------------ |
| **Sprint Name**       | Pre-Release Security & GDPR Fixes                                        |
| **Sprint Goal**       | Fix 1 CRITICAL + 2 HIGH findings from Stability Report v4 before release |
| **Duration Estimate** | ~1–2 days                                                                |
| **Priority**          | P0 — Critical                                                            |
| **Sprint Type**       | Backend (Rust) + Frontend (TypeScript)                                   |
| **Primary Owner**     | Full-stack engineer                                                      |
| **Source**            | `docs/STABILITY_REPORT_v4.md` — SEC-001, SEC-002, SEC-003                |
| **Depends On**        | Sprint 28 (commit `80872b5`)                                             |

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **OBRIGATÓRIO: 100% dos tickets desta sprint devem ser concluídos. A sprint não será aceita como entregue se qualquer ticket permanecer incompleto.**
>
> **MANDATORY: 100% of the tickets in this sprint MUST be completed. The sprint will NOT be accepted as delivered if any ticket remains incomplete.**

Every ticket must pass its acceptance criteria AND the full health check suite (9/9) before the sprint commit is made.

---

## Sprint Goal Statement

The Stability Report v4 uncovered 1 CRITICAL and 2 HIGH findings that were not visible in the v3 audit scope. All three are pre-release blockers:

1. **SEC-001 (CRITICAL):** `grant_consent()` is `#[cfg(test)]` only — Script hotkey action permanently unusable in production
2. **SEC-002 (HIGH):** Path mismatch `app_data_dir()` vs `%LOCALAPPDATA%\MiControl` — incomplete GDPR data deletion
3. **SEC-003 (HIGH):** `ai_usage.json` excluded from GDPR export — data portability violation

These 3 tickets fix the Script hotkey consent dead-lock and ensure full GDPR Art.17 (right to erasure) and Art.20 (data portability) compliance.

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

## S29-001 — Remove `#[cfg(test)]` from `grant_consent()` and expose Tauri command (SEC-001 CRITICAL)

| Field         | Value                                                                                          |
| ------------- | ---------------------------------------------------------------------------------------------- |
| **Ticket ID** | S29-001                                                                                        |
| **Title**     | Remove `#[cfg(test)]` from `grant_consent()` and expose Tauri command                          |
| **Priority**  | P0 — Critical                                                                                  |
| **Source**    | SEC-001 (v4 report)                                                                            |
| **Files**     | `src-tauri/src/hw/hotkeys/mod.rs`, `src-tauri/src/commands/hotkeys.rs`, `src-tauri/src/lib.rs` |
| **Effort**    | ~2 hours                                                                                       |

### Problem

The `grant_consent()` function in `hotkeys/mod.rs:1356` — the **only** way to write `true` into `hotkey_consent.json` — is annotated `#[cfg(test)]`:

```rust
#[cfg(test)]
fn grant_consent(interpreter: &str, path: &str, args: &[String]) -> Result<(), String> {
```

Meanwhile, `check_script_action()` (line 1407) requires `has_consent()` to return `true` before any `Script` action executes. There is **no Tauri command** registered in `lib.rs` that calls `grant_consent()` in production builds. This means:

1. **No script can ever run** in production — the consent file can never be populated
2. Every `Script` action hits `ScriptCheckResult::ConsentRequired` and is silently skipped
3. Users who configure script hotkeys see silent failures with only log warnings

### Solution

1. Remove `#[cfg(test)]` from `grant_consent()` in `hotkeys/mod.rs`
2. Make `grant_consent()` `pub` (or `pub(crate)`) so it can be called from the commands module
3. Create a new Tauri command `grant_script_consent` in `commands/hotkeys.rs`:

```rust
#[tauri::command]
pub async fn grant_script_consent(
    interpreter: String,
    path: String,
    args: Vec<String>,
) -> Result<(), String> {
    crate::util::blocking::run_blocking(move || {
        crate::hw::hotkeys::grant_consent(&interpreter, &path, &args)
    })
    .await
    .map_err(|e| e.to_string())
}
```

4. Register `grant_script_consent` in the `invoke_handler!` macro in `lib.rs`
5. Add a unit test that calls `grant_consent()` directly (not behind `#[cfg(test)]`) to verify it works in production builds

### Acceptance Criteria

- [ ] `#[cfg(test)]` removed from `grant_consent()` function
- [ ] `grant_consent()` is `pub(crate)` or `pub`
- [ ] New `grant_script_consent` Tauri command created in `commands/hotkeys.rs`
- [ ] `grant_script_consent` registered in `invoke_handler!` in `lib.rs`
- [ ] `grant_script_consent` uses `run_blocking` for file I/O
- [ ] Unit test verifies `grant_consent()` works outside `#[cfg(test)]` context
- [ ] `cargo check` passes
- [ ] `cargo clippy -D warnings` passes
- [ ] `cargo test` passes
- [ ] `cargo fmt --check` passes

---

## S29-002 — Unify path resolution and fix GDPR data deletion (SEC-002 HIGH)

| Field         | Value                                                                                  |
| ------------- | -------------------------------------------------------------------------------------- |
| **Ticket ID** | S29-002                                                                                |
| **Title**     | Unify path resolution — fix `delete_all_user_data()` to use `%LOCALAPPDATA%\MiControl` |
| **Priority**  | P0 — High                                                                              |
| **Source**    | SEC-002 (v4 report)                                                                    |
| **Files**     | `src-tauri/src/util/data_deletion.rs`, `src-tauri/src/commands/privacy.rs`             |
| **Effort**    | ~3 hours                                                                               |

### Problem

`delete_all_user_data()` uses Tauri's `app.path().app_data_dir()` to locate files. Tauri's `app_data_dir()` for identifier `com.micontrol.app` resolves to `%APPDATA%\com.micontrol.app` (Roaming) on Windows — **not** `%LOCALAPPDATA%\MiControl`.

However, the following security-sensitive files are written to `%LOCALAPPDATA%\MiControl` via direct `LOCALAPPDATA` env var reads (using `elev_dir()` or equivalent):

| File                  | Written via                                   | Deleted by `delete_all_user_data`? |
| --------------------- | --------------------------------------------- | ---------------------------------- |
| `elev_key.bin`        | `elev_dir()` → `LOCALAPPDATA\MiControl`       | ❌ Looks in `app_data_dir()`       |
| `elev_key.bin.old`    | `elev_dir()`                                  | ❌                                 |
| `nonces.json`         | `elev_dir()`                                  | ❌                                 |
| `ai_usage.json`       | `ai_usage.rs` → `LOCALAPPDATA\MiControl`      | ❌                                 |
| `hotkey_consent.json` | `hotkeys/mod.rs` → `LOCALAPPDATA\MiControl`   | ❌                                 |
| `hotkeys.json`        | `hotkeys/mod.rs` → `LOCALAPPDATA\MiControl`   | ❌                                 |
| `consent_audit.log`   | `consent_audit.rs` → `LOCALAPPDATA\MiControl` | ✅ (via `purge_audit_log()`)       |
| `ai_config.json`      | `ai_usage.rs` → `LOCALAPPDATA\MiControl`      | ❌                                 |

**Impact:** GDPR Art.17 (right to erasure) is **violated** — the HMAC key, key backup, nonce store, AI usage stats, hotkey consent, hotkey config, and AI config survive a "delete all user data" operation. An attacker who recovers a "deleted" machine can extract the HMAC key and forge elevated commands.

### Solution

1. Add a helper function `local_data_dir()` in `data_deletion.rs` (or reuse `elevated::elev_dir()`) that returns `%LOCALAPPDATA%\MiControl`:

```rust
/// Returns the local data directory: `%LOCALAPPDATA%\MiControl`.
/// This is where security-sensitive files (HMAC key, nonces, AI usage, etc.)
/// are stored, as opposed to Tauri's `app_data_dir()` which resolves to
/// `%APPDATA%\com.micontrol.app`.
fn local_data_dir() -> Result<PathBuf, String> {
    let base = std::env::var("LOCALAPPDATA")
        .map_err(|e| format!("LOCALAPPDATA not set: {e}"))?;
    Ok(PathBuf::from(base).join("MiControl"))
}
```

2. Update `delete_all_user_data()` to delete files from **both** locations:
   - Keep existing `app_data_dir()` deletions (for `schedule.json`, `consent.json`, `hardware_profile.json` — these may be in either location depending on when they were created)
   - Add deletions from `local_data_dir()` for: `elev_key.bin`, `elev_key.bin.old`, `nonces.json`, `ai_usage.json`, `hotkey_consent.json`, `hotkeys.json`, `ai_config.json`, `consent_audit.log`

3. Add `ai_usage.json` and `hotkey_consent.json` to the deletion list (they are currently missing entirely)

4. Add `ai_config.json` to the deletion list (currently only deleted from `app_data_dir()`, not `local_data_dir()`)

5. Update `DeleteDataReport` struct to include `ai_usage_deleted: bool` and `hotkey_consent_deleted: bool` fields

6. Add unit tests verifying that all files in `%LOCALAPPDATA%\MiControl` are deleted

### Acceptance Criteria

- [ ] `local_data_dir()` helper added to `data_deletion.rs`
- [ ] `delete_all_user_data()` deletes from both `app_data_dir()` and `local_data_dir()`
- [ ] `elev_key.bin`, `elev_key.bin.old`, `nonces.json` deleted from `local_data_dir()`
- [ ] `ai_usage.json` deleted from `local_data_dir()`
- [ ] `hotkey_consent.json` deleted from `local_data_dir()`
- [ ] `hotkeys.json` deleted from `local_data_dir()`
- [ ] `ai_config.json` deleted from `local_data_dir()`
- [ ] `consent_audit.log` deleted from `local_data_dir()` (in addition to `purge_audit_log()`)
- [ ] `DeleteDataReport` struct updated with new fields
- [ ] Unit tests verify all files are deleted
- [ ] `cargo check` passes
- [ ] `cargo clippy -D warnings` passes
- [ ] `cargo test` passes
- [ ] `cargo fmt --check` passes

---

## S29-003 — Add `ai_usage.json` to GDPR export (SEC-003 HIGH)

| Field         | Value                                                            |
| ------------- | ---------------------------------------------------------------- |
| **Ticket ID** | S29-003                                                          |
| **Title**     | Add `ai_usage.json` to `USER_DATA_FILES` in `export_user_data()` |
| **Priority**  | P0 — High                                                        |
| **Source**    | SEC-003 (v4 report)                                              |
| **Files**     | `src-tauri/src/commands/privacy.rs`                              |
| **Effort**    | ~1 hour (depends on S29-002 for path fix)                        |

### Problem

The `USER_DATA_FILES` array in `export_user_data()` includes `hardware_profile.json`, `hotkeys.json`, `consent_audit.log`, `ai_config.json`, `schedule.json`, and `consent.json` — but **omits `ai_usage.json`**.

`ai_usage.json` contains `total_requests`, `total_input_tokens`, `total_output_tokens`, `estimated_cost_usd`, `today_count`, and `model_usage` (per-model request counts). This is personal usage data under GDPR Art.20 (data portability).

Additionally, `export_user_data()` reads from `app_data_dir()`, but `ai_usage.json` is written to `%LOCALAPPDATA%\MiControl` — so even if added to the array, it wouldn't be found without the path fix from S29-002.

### Solution

1. Add `"ai_usage.json"` to the `USER_DATA_FILES` array in `privacy.rs`:

```rust
const USER_DATA_FILES: &[&str] = &[
    "hardware_profile.json",
    "hotkeys.json",
    "consent_audit.log",
    "ai_config.json",
    "schedule.json",
    "consent.json",
    "ai_usage.json", // S29-003: Added for GDPR Art.20 compliance.
    // S27-003: nonces.json excluded — internal anti-replay cache, not user data.
];
```

2. Update `export_user_data()` to also read from `local_data_dir()` (the `%LOCALAPPDATA%\MiControl` path) in addition to `app_data_dir()`. Files should be searched in both locations, with `local_data_dir()` taking precedence (since that's where security-sensitive files are actually written).

3. Add a comment explaining why `ai_usage.json` is included (user data: usage stats, cost estimates, model usage)

4. Add a unit test verifying `ai_usage.json` appears in the export

### Acceptance Criteria

- [ ] `"ai_usage.json"` added to `USER_DATA_FILES` array
- [ ] `export_user_data()` reads from both `app_data_dir()` and `local_data_dir()`
- [ ] Comment explains inclusion rationale
- [ ] Unit test verifies `ai_usage.json` is included in export
- [ ] `cargo check` passes
- [ ] `cargo clippy -D warnings` passes
- [ ] `cargo test` passes
- [ ] `cargo fmt --check` passes

---

## Backlog — Remaining v4 Findings (Not in This Sprint)

The following findings from the v4 report are **not** included in this sprint. They should be addressed in a future polish sprint (Sprint 30+).

### MEDIUM (4 findings)

| #       | Finding                                           | File:Line                                     | Effort  |
| ------- | ------------------------------------------------- | --------------------------------------------- | ------- |
| SEC-004 | CodeQL covers only JS/TS, not Rust                | `.github/workflows/codeql.yml:18`             | ~1 hour |
| SEC-005 | AES-GCM nonce truncated 16→12 bytes               | `src-tauri/src/hw/iotservice.rs:417-418`      | ~1 hour |
| SEC-006 | `set_secret` has no write allowlist               | `src-tauri/src/commands/credentials.rs:10-18` | ~1 hour |
| SEC-007 | EC RAM safe-write config searched relative to CWD | `src-tauri/src/hw/ecram.rs:611-635`           | ~1 hour |

### LOW (14 findings)

| #        | Finding                                                                   | File:Line                                      | Effort   |
| -------- | ------------------------------------------------------------------------- | ---------------------------------------------- | -------- |
| SEC-008  | `restrict_file_acl` uses spoofable `USERNAME` env var                     | `src-tauri/src/util/auth.rs:265`               | ~1 hour  |
| SEC-009  | Nonce store has no size limit                                             | `src-tauri/src/elevated.rs:137-183`            | ~2 hours |
| SEC-010  | `test_connection` doesn't increment daily counter                         | `src-tauri/src/commands/ai.rs:131`             | ~30 min  |
| ARCH-001 | `hotkeys/mod.rs` is 2,600 lines — no logical decomposition                | `src-tauri/src/hw/hotkeys/mod.rs`              | ~1 day   |
| ARCH-005 | `osd.rs` uses `.lock().unwrap()` (30+ sites) instead of `lock_or_recover` | `src-tauri/src/hw/osd.rs`                      | ~2 hours |
| ARCH-015 | `IotEvent` type not exposed to frontend TypeScript                        | `src/types/hardware.ts`                        | ~30 min  |
| ARCH-016 | `IPC_WRITE_TIMES` is only `static Mutex` not in `OnceLock`                | `src-tauri/src/hw/iotservice.rs:~560`          | ~30 min  |
| ARCH-017 | Duplicate `# Safety` doc comment in `osd.rs` `reposition_osd`             | `src-tauri/src/hw/osd.rs:~345`                 | ~5 min   |
| ARCH-018 | Duplicated layout documentation in `osd.rs` `paint_notification`          | `src-tauri/src/hw/osd.rs:~540`                 | ~5 min   |
| DEV-002  | E2E job uses `continue-on-error: true`                                    | `.github/workflows/ci.yml:236`                 | ~5 min   |
| PERF-001 | `AiUsagePanel` frontend interface omits `model_usage`                     | `src/components/AiAnalysis.tsx:271-276`        | ~1 hour  |
| RAI-001  | AI feedback is stored only locally                                        | `src/components/AiFeedback.tsx:7`              | ~3 hours |
| RAI-002  | AI documentation link points to GitHub                                    | `src/components/AiConfigForm.tsx:198`          | ~30 min  |
| UI-001   | OnboardingWizard test mocks `t()` to return raw keys                      | `src/__tests__/OnboardingWizard.test.tsx:5-10` | ~1 hour  |

### INFO (6 findings)

| #       | Finding                                         | File:Line                              | Effort   |
| ------- | ----------------------------------------------- | -------------------------------------- | -------- |
| SEC-011 | `ai_usage.json` ACL restriction is warn-only    | `src-tauri/src/util/ai_usage.rs:73-75` | ~30 min  |
| SEC-012 | Updater public key in config (by design)        | `src-tauri/tauri.conf.json:62`         | N/A      |
| DEV-003 | cargo-deny not cached in CI                     | `.github/workflows/ci.yml:73-77`       | ~30 min  |
| DEV-004 | Pre-commit hook runs slow `tsc --noEmit`        | `.husky/pre-commit:4-5`                | ~15 min  |
| UI-002  | EcrDebugPanel uses `window.confirm()`           | `src/components/EcrDebugPanel.tsx:95`  | ~2 hours |
| RAI-003 | Process names sent to AI provider in plain text | `docs/ai-features.md:36`               | ~2 hours |

---

## Sprint Commit Message

```
fix(s29): pre-release security & GDPR fixes (P0)

- S29-001: Remove #[cfg(test)] from grant_consent(), expose Tauri command (SEC-001 CRITICAL)
- S29-002: Unify path resolution, fix delete_all_user_data() to use %LOCALAPPDATA%\MiControl (SEC-002 HIGH)
- S29-003: Add ai_usage.json to GDPR export (SEC-003 HIGH)
```
