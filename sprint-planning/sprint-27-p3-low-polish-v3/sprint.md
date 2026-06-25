# Sprint 27 — P3 LOW: Polish, Defense-in-Depth, Test Gaps (Post-Audit v3)

## Sprint Metadata

| Field                 | Value                                            |
| --------------------- | ------------------------------------------------ |
| **Sprint Name**       | Polish, Defense-in-Depth, Test Gaps              |
| **Sprint Goal**       | Fix all 12 LOW findings from Stability Report v3 |
| **Duration Estimate** | ~2 days                                          |
| **Priority**          | P3 — Low                                         |
| **Sprint Type**       | Multi-domain (Backend, Frontend, DevOps)         |
| **Primary Owner**     | Full-stack engineer                              |
| **Source**            | `docs/STABILITY_REPORT_v3.md` — All LOW findings |
| **Depends On**        | Sprint 26                                        |

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **OBRIGATÓRIO: 100% dos tickets desta sprint devem ser concluídos. A sprint não será aceita como entregue se qualquer ticket permanecer incompleto.**
>
> **MANDATORY: 100% of the tickets in this sprint MUST be completed. The sprint will NOT be accepted as delivered if any ticket remains incomplete.**

Every ticket must pass its acceptance criteria AND the full health check suite (9/9) before the sprint commit is made.

---

## Sprint Goal Statement

The post-sprint-25 stability audit (v3) identified 12 LOW findings across 4 domains. These are defense-in-depth gaps, minor hardening opportunities, test coverage gaps, and DevOps improvements. This sprint batches them into 2 execution groups:

- **Batch A (Rust Backend):** PII redaction multi-occurrence, TOCTOU fix, GDPR export cleanup, keyring allowlist, blocking I/O residuals, OSD graceful degradation
- **Batch B (Frontend & DevOps):** Accessibility i18n, test coverage, CodeQL Rust, release caching, release frontend checks

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

## Batch A — Rust Backend (S27-001 through S27-006)

### S27-001 — Fix PII redaction to handle multiple occurrences

| Field                | Value                                                                      |
| -------------------- | -------------------------------------------------------------------------- |
| **Ticket ID**        | S27-001                                                                    |
| **Title**            | Fix `redact_unc_path` and `redact_path_username` to redact ALL occurrences |
| **Priority**         | P3                                                                         |
| **Type**             | Privacy / Security                                                         |
| **Estimated Effort** | S                                                                          |
| **Source Finding**   | S-005, S-006 (LOW)                                                         |

#### Context

In `src-tauri/src/lib.rs:785-830`, `redact_unc_path` uses `result.find("\\\\")` which finds only the first occurrence. `redact_path_username` returns immediately after the first matching drive prefix. If a string contains multiple UNC paths or multiple user paths (e.g., in a stacktrace), only the first is redacted.

#### Acceptance Criteria

- [ ] `redact_unc_path` redacts ALL UNC path occurrences (use loop or regex)
- [ ] `redact_path_username` redacts ALL drive-letter path occurrences across all drives
- [ ] Unit test verifies multiple occurrences are redacted
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S27-002 — Pass canonical path to `explorer.exe` in `reveal_in_explorer`

| Field                | Value                                                                           |
| -------------------- | ------------------------------------------------------------------------------- |
| **Ticket ID**        | S27-002                                                                         |
| **Title**            | Pass `canonical.to_string_lossy()` to `explorer.exe` instead of original `path` |
| **Priority**         | P3                                                                              |
| **Type**             | Security / TOCTOU                                                               |
| **Estimated Effort** | XS                                                                              |
| **Source Finding**   | S-007 (LOW)                                                                     |

#### Context

In `src-tauri/src/commands/privacy.rs:128-145`, `reveal_in_explorer` validates that the canonical path is within allowed directories, but then passes the original unvalidated `path` string to `explorer.exe`. There's a TOCTOU gap: the original `path` could be a symlink or junction that resolves differently between `canonicalize()` and `explorer.exe`'s resolution.

#### Acceptance Criteria

- [ ] `explorer.exe` receives `canonical.to_string_lossy()` instead of original `path`
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S27-003 — Remove `nonces.json` from GDPR export

| Field                | Value                                                             |
| -------------------- | ----------------------------------------------------------------- |
| **Ticket ID**        | S27-003                                                           |
| **Title**            | Remove `nonces.json` from `USER_DATA_FILES` in `export_user_data` |
| **Priority**         | P3                                                                |
| **Type**             | Security / Privacy                                                |
| **Estimated Effort** | XS                                                                |
| **Source Finding**   | S-008 (LOW)                                                       |

#### Context

In `src-tauri/src/commands/privacy.rs:20-28`, the `USER_DATA_FILES` list includes `nonces.json`. The nonce store is an internal security mechanism (anti-replay cache for the elevated bridge). It is not "user data" in the GDPR sense and has no value to the user. The HMAC key (`elev_key.bin`) is correctly excluded — the same logic should apply to nonces.

#### Acceptance Criteria

- [ ] `nonces.json` removed from `USER_DATA_FILES` array
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S27-004 — Add key allowlist to `get_secret` command

| Field                | Value                                                        |
| -------------------- | ------------------------------------------------------------ |
| **Ticket ID**        | S27-004                                                      |
| **Title**            | Add allowlist of permitted keyring key names to `get_secret` |
| **Priority**         | P3                                                           |
| **Type**             | Security / Defense-in-Depth                                  |
| **Estimated Effort** | XS                                                           |
| **Source Finding**   | S-009 (LOW)                                                  |

#### Context

In `src-tauri/src/commands/credentials.rs:27-34`, the `get_secret` command accepts an arbitrary `key` parameter with no allowlist. Any frontend JavaScript (including injected code if CSP is bypassed) can read any secret stored under the service name, including `openai_api_key` and `telemetry_consent`.

#### Acceptance Criteria

- [ ] Allowlist of permitted keys (e.g., `["openai_api_key", "telemetry_consent"]`)
- [ ] Unknown keys rejected with descriptive error
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S27-005 — Wrap `save_config()` in `set_hotkey_config` with `run_blocking`

| Field                | Value                                                                |
| -------------------- | -------------------------------------------------------------------- |
| **Ticket ID**        | S27-005                                                              |
| **Title**            | Wrap `save_config()` call in `set_hotkey_config` with `run_blocking` |
| **Priority**         | P3                                                                   |
| **Type**             | Performance / Concurrency                                            |
| **Estimated Effort** | XS                                                                   |
| **Source Finding**   | A-004 (LOW)                                                          |

#### Context

In `src-tauri/src/commands/hotkeys.rs:12`, `set_hotkey_config` calls `save_config(&config)` directly on the async runtime. `save_config()` does `std::fs::create_dir_all` + `std::fs::write` + `restrict_file_acl` — all synchronous filesystem operations. This violates the S24-013 pattern.

#### Acceptance Criteria

- [ ] `save_config()` call wrapped in `run_blocking`
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S27-006 — Replace `expect()` on OSD thread spawn with graceful degradation

| Field                | Value                                                            |
| -------------------- | ---------------------------------------------------------------- |
| **Ticket ID**        | S27-006                                                          |
| **Title**            | Replace `expect()` in `osd::init()` with graceful error handling |
| **Priority**         | P3                                                               |
| **Type**             | Stability / Error Handling                                       |
| **Estimated Effort** | XS                                                               |
| **Source Finding**   | A-005 (LOW)                                                      |

#### Context

In `src-tauri/src/hw/osd.rs:113`, `osd::init()` calls `.expect("osd thread spawn failed")` on `thread::spawn()`. This is the same pattern that S24-004 fixed in `hotkeys::start_hook()`. The OSD thread is non-critical (brightness overlay UI), so a spawn failure should degrade gracefully.

#### Acceptance Criteria

- [ ] `.expect()` replaced with `if let Err(e) = ... { log::warn!(...) }`
- [ ] App continues without OSD if thread spawn fails
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

## Batch B — Frontend & DevOps (S27-007 through S27-011)

### S27-007 — Use `t()` for `aria-label` on API key show/hide button

| Field                | Value                                                           |
| -------------------- | --------------------------------------------------------------- |
| **Ticket ID**        | S27-007                                                         |
| **Title**            | Replace hardcoded English `aria-label` with `t()` function call |
| **Priority**         | P3                                                              |
| **Type**             | Accessibility / i18n                                            |
| **Estimated Effort** | XS                                                              |
| **Source Finding**   | U-001 (LOW)                                                     |

#### Context

In `src/components/AiConfigForm.tsx:135`, the `aria-label` on the API key show/hide button is hardcoded in English (`"Show API key"` / `"Hide API key"`) while the `title` attribute uses `t()`. Non-English screen reader users hear English.

#### Acceptance Criteria

- [ ] `aria-label` uses `t('settings.showKey')` / `t('settings.hideKey')` (or appropriate i18n keys)
- [ ] New keys added to all 4 locale files if they don't exist
- [ ] `npx tsc --noEmit` passes, `npm run lint` passes, `npm run build` passes

---

### S27-008 — Add tests for ErrorBoundary compact mode and OnboardingWizard accessibility

| Field                | Value                                                                       |
| -------------------- | --------------------------------------------------------------------------- |
| **Ticket ID**        | S27-008                                                                     |
| **Title**            | Add test coverage for per-tab ErrorBoundary and OnboardingWizard focus trap |
| **Priority**         | P3                                                                          |
| **Type**             | Testing / Quality                                                           |
| **Estimated Effort** | S                                                                           |
| **Source Finding**   | U-002, U-003 (LOW)                                                          |

#### Context

The ErrorBoundary compact mode (S24-012) and OnboardingWizard focus trap/Escape handler (S24-010) have no test coverage. These are accessibility-critical features that should be tested.

#### Acceptance Criteria

- [ ] Test for ErrorBoundary compact mode: renders compact error UI, "Reload tab" button resets state
- [ ] Test for OnboardingWizard: focus trap cycles within modal, Escape key closes wizard
- [ ] `npx tsc --noEmit` passes, `npm run lint` passes, `npm run build` passes

---

### S27-009 — Add `cargo-deny` or Semgrep for Rust SAST

| Field                | Value                                                     |
| -------------------- | --------------------------------------------------------- |
| **Ticket ID**        | S27-009                                                   |
| **Title**            | Add `cargo-deny` to CI for Rust license/advisory checking |
| **Priority**         | P3                                                        |
| **Type**             | DevOps / Security                                         |
| **Estimated Effort** | XS                                                        |
| **Source Finding**   | D-002 (LOW)                                               |

#### Context

CodeQL only covers JavaScript/TypeScript. The Rust backend (where all the crypto, IPC, and hardware access lives) is not analyzed by CodeQL. `cargo-audit` provides dependency CVE scanning but not license checking or code-level analysis.

#### Acceptance Criteria

- [ ] `cargo-deny` added to CI workflow (or Semgrep with Rust rules)
- [ ] Runs on push to `master` and on PRs
- [ ] `deny.toml` configuration file created with license/advisory rules

---

### S27-010 — Cache `cargo-audit` in release workflow

| Field                | Value                                                     |
| -------------------- | --------------------------------------------------------- |
| **Ticket ID**        | S27-010                                                   |
| **Title**            | Add `actions/cache` for `cargo-audit` in release workflow |
| **Priority**         | P3                                                        |
| **Type**             | DevOps / Performance                                      |
| **Estimated Effort** | XS                                                        |
| **Source Finding**   | D-003 (LOW)                                               |

#### Context

The CI workflow caches `cargo-audit` and `cargo-tarpaulin` installs (D-L01, D-L02), but the release workflow installs `cargo-audit` without caching on every release build.

#### Acceptance Criteria

- [ ] `actions/cache@v4` step added to release workflow for `~/.cargo/bin/cargo-audit`
- [ ] Install step only runs if not already cached

---

### S27-011 — Add frontend checks to release workflow

| Field                | Value                                                                |
| -------------------- | -------------------------------------------------------------------- |
| **Ticket ID**        | S27-011                                                              |
| **Title**            | Add `tsc`, `lint`, and `test` steps to release workflow before build |
| **Priority**         | P3                                                                   |
| **Type**             | DevOps / Quality                                                     |
| **Estimated Effort** | XS                                                                   |
| **Source Finding**   | D-004 (LOW)                                                          |

#### Context

The release workflow runs `cargo audit` and `npm audit` (D-L05) but skips `tsc`, `eslint`, and frontend tests before building. A TypeScript or lint error could slip into a release build.

#### Acceptance Criteria

- [ ] `npx tsc --noEmit` step added before build
- [ ] `npm run lint` step added before build
- [ ] `npm run test` step added before build (if test script exists)

---

## Sprint Commit

```bash
git add -A
git commit -m "chore(s27): polish, defense-in-depth, test gaps, DevOps improvements (P3)

Batch A - Rust Backend:
- S27-001: Fix PII redaction for multiple occurrences (S-005, S-006)
- S27-002: Pass canonical path to explorer.exe (S-007)
- S27-003: Remove nonces.json from GDPR export (S-008)
- S27-004: Add key allowlist to get_secret (S-009)
- S27-005: Wrap save_config in run_blocking (A-004)
- S27-006: Graceful OSD thread spawn failure (A-005)

Batch B - Frontend & DevOps:
- S27-007: Use t() for aria-label on API key button (U-001)
- S27-008: Tests for ErrorBoundary compact and OnboardingWizard (U-002, U-003)
- S27-009: Add cargo-deny for Rust SAST (D-002)
- S27-010: Cache cargo-audit in release workflow (D-003)
- S27-011: Add frontend checks to release workflow (D-004)"
```
