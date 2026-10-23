# Sprint 25 — P3 LOW: Polish & Consistency (Post-Audit v2)

## Sprint Metadata

| Field                 | Value                                                                    |
| --------------------- | ------------------------------------------------------------------------ |
| **Sprint Name**       | Polish & Consistency                                                     |
| **Sprint Goal**       | Batch-fix all LOW findings for consistency, polish, and defense-in-depth |
| **Duration Estimate** | ~3 days                                                                  |
| **Priority**          | P3 — Low                                                                 |
| **Sprint Type**       | Multi-domain (Backend, Frontend, Config, DevOps)                         |
| **Primary Owner**     | Full-stack engineer                                                      |
| **Source**            | `docs/STABILITY_REPORT_v2.md` — All LOW findings                         |
| **Depends On**        | Sprint 24                                                                |

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **OBRIGATÓRIO: 100% dos tickets desta sprint devem ser concluídos. A sprint não será aceita como entregue se qualquer ticket permanecer incompleto.**
>
> **MANDATORY: 100% of the tickets in this sprint MUST be completed. The sprint will NOT be accepted as delivered if any ticket remains incomplete.**

Every ticket must pass its acceptance criteria AND the full health check suite (9/9) before the sprint commit is made.

---

## Sprint Goal Statement

The post-sprint-21 stability audit (v2) identified 18 LOW findings across 6 domains. These are not bugs or security vulnerabilities — they are consistency issues, minor polish items, and defense-in-depth improvements. This sprint batches them into 2 execution groups:

- **Batch A (Rust Backend):** Nonce atomicity, Sentry PII, HKDF fallback logging, export ACL, registry consistency, error message leaks, silent fallbacks, unreachable panics
- **Batch B (Frontend & Config):** CSP directives, accessibility, inline styles, optimization level, SVG distortion, display polling, CI caching, release verification, AI log expiration

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

## Batch A — Rust Backend (S25-001 through S25-010)

### S25-001 — Make nonce save atomic (temp file + rename)

| Field                | Value                                                               |
| -------------------- | ------------------------------------------------------------------- |
| **Ticket ID**        | S25-001                                                             |
| **Title**            | Use temp file + rename pattern for `save_nonces()` in `elevated.rs` |
| **Priority**         | P3                                                                  |
| **Type**             | Security / Data Integrity                                           |
| **Estimated Effort** | XS                                                                  |
| **Source Finding**   | S-L01 (LOW)                                                         |

#### Context

In `src-tauri/src/elevated.rs:108-117`, `save_nonces()` writes directly to the nonce file. If the process crashes mid-write, the file is corrupted and all nonces are lost. The `elev_bridge.rs` already uses the temp-file-then-rename pattern for command files.

#### Acceptance Criteria

- [x] `save_nonces()` writes to a temp file first, then renames to the final path
- [x] Temp file is in the same directory (for atomic rename on same filesystem)
- [x] Temp file is cleaned up on error
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S25-002 — Improve Sentry PII redaction

| Field                | Value                                                                                |
| -------------------- | ------------------------------------------------------------------------------------ |
| **Ticket ID**        | S25-002                                                                              |
| **Title**            | Add IPv6 detection, UNC path redaction, and other drive letters to Sentry PII filter |
| **Priority**         | P3                                                                                   |
| **Type**             | Privacy / Security                                                                   |
| **Estimated Effort** | S                                                                                    |
| **Source Finding**   | S-L02 (LOW)                                                                          |

#### Context

In `src-tauri/src/lib.rs:103-140`, the Sentry PII redaction covers IPv4 and `C:\Users\` paths but misses IPv6 addresses, UNC paths (`\\server\share`), and drives other than `C:`.

#### Acceptance Criteria

- [x] IPv6 addresses are redacted (e.g., `2001:db8::1` → `[REDACTED_IP]`)
- [x] UNC paths are redacted (e.g., `\\server\share\` → `[REDACTED_PATH]`)
- [x] Drive letters `D:` through `Z:` are redacted (not just `C:`)
- [x] Unit test verifies all patterns are redacted
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S25-003 — Log warning on HKDF fallback in `derive_aes_key`

| Field                | Value                                                                          |
| -------------------- | ------------------------------------------------------------------------------ |
| **Ticket ID**        | S25-003                                                                        |
| **Title**            | Add `log::warn!` when HKDF fails and falls back to SHA-256 in `derive_aes_key` |
| **Priority**         | P3                                                                             |
| **Type**             | Observability / Security                                                       |
| **Estimated Effort** | XS                                                                             |
| **Source Finding**   | S-L05 (LOW)                                                                    |

#### Context

In `src-tauri/src/hw/iotservice.rs:343-352`, `derive_aes_key()` silently falls back to legacy SHA-256 derivation when HKDF fails. This should be logged for debugging.

#### Acceptance Criteria

- [x] `log::warn!("HKDF key derivation failed, falling back to legacy SHA-256: {e}")` added
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S25-004 — Restrict ACL on export ZIP file

| Field                | Value                                                 |
| -------------------- | ----------------------------------------------------- |
| **Ticket ID**        | S25-004                                               |
| **Title**            | Call `restrict_file_acl` on export ZIP after creation |
| **Priority**         | P3                                                    |
| **Type**             | Security / Privacy                                    |
| **Estimated Effort** | XS                                                    |
| **Source Finding**   | S-L04 (LOW)                                           |

#### Context

In `src-tauri/src/util/privacy.rs:30-35`, the export ZIP file is created without ACL restriction. Any process can read the exported data.

#### Acceptance Criteria

- [x] `restrict_file_acl` is called after the ZIP file is created
- [x] Log warning if ACL restriction fails (but don't crash)
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S25-005 — Migrate `display.rs` to use `RegKeyGuard`

| Field                | Value                                                             |
| -------------------- | ----------------------------------------------------------------- |
| **Ticket ID**        | S25-005                                                           |
| **Title**            | Replace `winreg::RegKey` usage in `display.rs` with `RegKeyGuard` |
| **Priority**         | P3                                                                |
| **Type**             | Architecture / Consistency                                        |
| **Estimated Effort** | S                                                                 |
| **Source Finding**   | A-L01 (LOW)                                                       |

#### Context

In `src-tauri/src/hw/display.rs:148-160`, `winreg::RegKey` is used directly instead of the `RegKeyGuard` RAII pattern from `util/registry.rs`.

#### Acceptance Criteria

- [x] All `winreg::RegKey` usage in `display.rs` replaced with `RegKeyGuard::open_read` / `RegKeyGuard::create_write`
- [x] No direct `winreg` imports remain in `display.rs`
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S25-006 — Migrate `charging.rs` to use `RegKeyGuard`

| Field                | Value                                                                         |
| -------------------- | ----------------------------------------------------------------------------- |
| **Ticket ID**        | S25-006                                                                       |
| **Title**            | Replace raw `RegOpenKeyExW`/`RegCloseKey` in `charging.rs` with `RegKeyGuard` |
| **Priority**         | P3                                                                            |
| **Type**             | Architecture / Consistency                                                    |
| **Estimated Effort** | S                                                                             |
| **Source Finding**   | A-L02 (LOW)                                                                   |

#### Context

In `src-tauri/src/hw/charging.rs:170-195`, raw `RegOpenKeyExW`/`RegCloseKey` Win32 calls are used instead of the `RegKeyGuard` RAII pattern.

#### Acceptance Criteria

- [x] All raw registry API calls in `charging.rs` replaced with `RegKeyGuard`
- [x] No direct `RegOpenKeyExW`/`RegCloseKey` calls remain in `charging.rs`
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S25-007 — Migrate `hotkeys.rs` to use `RegKeyGuard`

| Field                | Value                                                             |
| -------------------- | ----------------------------------------------------------------- |
| **Ticket ID**        | S25-007                                                           |
| **Title**            | Replace `winreg::RegKey` usage in `hotkeys.rs` with `RegKeyGuard` |
| **Priority**         | P3                                                                |
| **Type**             | Architecture / Consistency                                        |
| **Estimated Effort** | XS                                                                |
| **Source Finding**   | A-L03 (LOW)                                                       |

#### Context

In `src-tauri/src/hw/hotkeys.rs:1245-1250`, `winreg::RegKey` is used directly for `is_script_action_enabled`.

#### Acceptance Criteria

- [x] `winreg::RegKey` usage in `hotkeys.rs` replaced with `RegKeyGuard::open_read`
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S25-008 — Remove WMI query strings from error messages

| Field                | Value                                                         |
| -------------------- | ------------------------------------------------------------- |
| **Ticket ID**        | S25-008                                                       |
| **Title**            | Remove WMI query strings from `Display` impl, keep in `Debug` |
| **Priority**         | P3                                                            |
| **Type**             | Security / Information Disclosure                             |
| **Estimated Effort** | XS                                                            |
| **Source Finding**   | A-L04 (LOW)                                                   |

#### Context

In `src-tauri/src/hw/errors.rs:45-50`, WMI query strings are included in the `Display` implementation, which means they are sent to the frontend. This leaks internal implementation details.

#### Acceptance Criteria

- [x] WMI query strings removed from `Display` impl (user-facing)
- [x] WMI query strings kept in `Debug` impl (developer-facing)
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S25-009 — Return error instead of empty vec on refresh rate failure

| Field                | Value                                                                               |
| -------------------- | ----------------------------------------------------------------------------------- |
| **Ticket ID**        | S25-009                                                                             |
| **Title**            | Change `get_available_refresh_rates` to return `Result` instead of silent empty vec |
| **Priority**         | P3                                                                                  |
| **Type**             | Error Handling                                                                      |
| **Estimated Effort** | XS                                                                                  |
| **Source Finding**   | A-L05 (LOW)                                                                         |

#### Context

In `src-tauri/src/hw/system.rs:210-213`, `get_available_refresh_rates` silently returns an empty `Vec` on error, making it impossible to distinguish "no refresh rates" from "query failed".

#### Acceptance Criteria

- [x] Return type changed from `Vec<u32>` to `Result<Vec<u32>, ErrorResponse>`
- [x] Callers updated to handle the error
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S25-010 — Replace `unreachable!()` with `return Err` in retry loop

| Field                | Value                                                         |
| -------------------- | ------------------------------------------------------------- |
| **Ticket ID**        | S25-010                                                       |
| **Title**            | Replace `unreachable!()` in `retry.rs` with `return Err(...)` |
| **Priority**         | P3                                                            |
| **Type**             | Stability / Error Handling                                    |
| **Estimated Effort** | XS                                                            |
| **Source Finding**   | A-L07 (LOW)                                                   |

#### Context

In `src-tauri/src/util/retry.rs:73`, `unreachable!()` is used in a retry loop. If the code path is actually reached (e.g., due to a logic error), it panics instead of returning an error.

#### Acceptance Criteria

- [x] `unreachable!()` replaced with `return Err(anyhow::anyhow!("..."))` or appropriate error
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

## Batch B — Frontend & Config (S25-011 through S25-018)

### S25-011 — Add `form-action 'self'` to CSP

| Field                | Value                                           |
| -------------------- | ----------------------------------------------- |
| **Ticket ID**        | S25-011                                         |
| **Title**            | Add `form-action 'self'` directive to Tauri CSP |
| **Priority**         | P3                                              |
| **Type**             | Security                                        |
| **Estimated Effort** | XS                                              |
| **Source Finding**   | S-L03 (LOW)                                     |

#### Context

In `tauri.conf.json:25`, the CSP is missing `form-action 'self'`, allowing forms to submit to arbitrary origins.

#### Acceptance Criteria

- [x] `form-action 'self'` added to CSP in `tauri.conf.json`
- [x] `npm run build` passes

---

### S25-012 — Remove unnecessary `api.openai.com` from CSP

| Field                | Value                                                  |
| -------------------- | ------------------------------------------------------ |
| **Ticket ID**        | S25-012                                                |
| **Title**            | Remove `https://api.openai.com` from CSP `connect-src` |
| **Priority**         | P3                                                     |
| **Type**             | Security / Attack Surface                              |
| **Estimated Effort** | XS                                                     |
| **Source Finding**   | S-L04 (LOW)                                            |

#### Context

In `tauri.conf.json:25`, `https://api.openai.com` is in the CSP `connect-src` allowlist, but the app uses user-configured `base_url` for AI requests, not hardcoded OpenAI.

#### Acceptance Criteria

- [x] `https://api.openai.com` removed from CSP `connect-src`
- [x] `npm run build` passes

---

### S25-013 — Add `aria-label` to API key show/hide button

| Field                | Value                                                          |
| -------------------- | -------------------------------------------------------------- |
| **Ticket ID**        | S25-013                                                        |
| **Title**            | Add `aria-label` to API key show/hide button in `AiConfigForm` |
| **Priority**         | P3                                                             |
| **Type**             | Accessibility                                                  |
| **Estimated Effort** | XS                                                             |
| **Source Finding**   | U-L01 (LOW)                                                    |

#### Context

In `src/components/AiConfigForm.tsx:113-117`, the API key show/hide button uses emoji with `title` but no `aria-label`, making it inaccessible to screen readers.

#### Acceptance Criteria

- [x] `aria-label="Show API key"` / `aria-label="Hide API key"` added (dynamic based on state)
- [x] `npx tsc --noEmit` passes, `npm run lint` passes

---

### S25-014 — Add `role="progressbar"` to onboarding progress dots

| Field                | Value                                                                       |
| -------------------- | --------------------------------------------------------------------------- |
| **Ticket ID**        | S25-014                                                                     |
| **Title**            | Add `role="progressbar"` and `aria-label` to OnboardingWizard progress dots |
| **Priority**         | P3                                                                          |
| **Type**             | Accessibility                                                               |
| **Estimated Effort** | XS                                                                          |
| **Source Finding**   | U-L02 (LOW)                                                                 |

#### Context

In `src/components/OnboardingWizard.tsx:42-52`, progress dots are `<div>` elements without `role="progressbar"`, making them invisible to screen readers.

#### Acceptance Criteria

- [x] Progress dots container has `role="progressbar"` and `aria-label="Onboarding progress"`
- [x] `npx tsc --noEmit` passes, `npm run lint` passes

---

### S25-015 — Extract AiAnalysis inline styles to CSS classes

| Field                | Value                                                                               |
| -------------------- | ----------------------------------------------------------------------------------- |
| **Ticket ID**        | S25-015                                                                             |
| **Title**            | Move inline styles in `AiAnalysis.tsx` to CSS classes for `:hover`/`:focus` support |
| **Priority**         | P3                                                                                  |
| **Type**             | UX / Code Quality                                                                   |
| **Estimated Effort** | S                                                                                   |
| **Source Finding**   | U-L04 (LOW)                                                                         |

#### Context

In `src/components/AiAnalysis.tsx`, inline styles prevent `:hover` and `:focus` pseudo-class styling.

#### Acceptance Criteria

- [x] All inline styles in `AiAnalysis.tsx` extracted to CSS classes
- [x] `:hover` and `:focus` styles added where appropriate
- [x] `npx tsc --noEmit` passes, `npm run lint` passes, `npm run build` passes

---

### S25-016 — Change `opt-level` from `"s"` to `3`

| Field                | Value                                                               |
| -------------------- | ------------------------------------------------------------------- |
| **Ticket ID**        | S25-016                                                             |
| **Title**            | Change `opt-level` from `"s"` (size) to `3` (speed) in `Cargo.toml` |
| **Priority**         | P3                                                                  |
| **Type**             | Performance                                                         |
| **Estimated Effort** | XS                                                                  |
| **Source Finding**   | P-L02 (LOW)                                                         |

#### Context

In `src-tauri/Cargo.toml:96`, `opt-level = "s"` optimizes for binary size, not execution speed. For a hardware control app, speed is more important than size.

#### Acceptance Criteria

- [x] `opt-level` changed to `3` in `[profile.release]`
- [x] `cargo check` passes, `cargo test` passes
- [x] Binary size increase is acceptable (document if significant)

---

### S25-017 — Fix LineChart SVG distortion

| Field                | Value                                                                                   |
| -------------------- | --------------------------------------------------------------------------------------- |
| **Ticket ID**        | S25-017                                                                                 |
| **Title**            | Change `preserveAspectRatio` from `"none"` to `"xMidYMid meet"` in AiAnalysis LineChart |
| **Priority**         | P3                                                                                      |
| **Type**             | UX / Visualization                                                                      |
| **Estimated Effort** | XS                                                                                      |
| **Source Finding**   | P-L03 (LOW)                                                                             |

#### Context

In `src/components/AiAnalysis.tsx:131`, the LineChart SVG uses `preserveAspectRatio="none"`, which distorts polylines when the container aspect ratio doesn't match.

#### Acceptance Criteria

- [x] `preserveAspectRatio` changed to `"xMidYMid meet"`
- [x] Chart still renders correctly
- [x] `npm run build` passes

---

### S25-018 — Skip brightness loop when display is off

| Field                | Value                                                                  |
| -------------------- | ---------------------------------------------------------------------- |
| **Ticket ID**        | S25-018                                                                |
| **Title**            | Check display power state before polling in `adaptive_brightness_loop` |
| **Priority**         | P3                                                                     |
| **Type**             | Performance / Power                                                    |
| **Estimated Effort** | XS                                                                     |
| **Source Finding**   | P-L04 (LOW)                                                            |

#### Context

In `src-tauri/src/hw/display.rs:170-175`, `adaptive_brightness_loop` runs every 2 seconds even when the display is off, wasting CPU and power.

#### Acceptance Criteria

- [x] Display power state checked before polling
- [x] If display is off, skip the iteration (sleep and continue)
- [x] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

## Additional LOW Items (no separate ticket — fix in batch)

These items are small enough to be fixed as part of the batch without individual tickets:

| Finding | File                         | Fix                                                                        |
| ------- | ---------------------------- | -------------------------------------------------------------------------- |
| A-L06   | `system.rs:283-285`          | Add `log::warn!` when `get_hardware_profile` returns `None`                |
| D-L01   | `ci.yml:28-30`               | Cache `cargo-audit` install using `actions/cache` or `Swatinem/rust-cache` |
| D-L02   | `ci.yml:96-97`               | Cache `cargo-tarpaulin` install                                            |
| D-L03   | `ci.yml:115,142`             | Verify `CODECOV_TOKEN` is configured (add comment if not)                  |
| D-L04   | `release.yml:55-56`          | Add step to verify git tag version matches `package.json`                  |
| D-L05   | `release.yml`                | Add `cargo audit` and `npm audit` steps before build                       |
| AI-L01  | `useAnalysisLogger.ts:12-14` | Add 30-day expiration to AI analysis logs in localStorage                  |
| AI-L02  | `ai.rs:96-100`               | Add backend rate limiting comment or basic enforcement                     |

---

## Sprint Commit

```bash
git add -A
git commit -m "chore(sprint-25): polish, consistency, and defense-in-depth improvements (P3)

Batch A - Rust Backend:
- S25-001: Atomic nonce save (S-L01)
- S25-002: Improve Sentry PII redaction (S-L02)
- S25-003: Log HKDF fallback warning (S-L05)
- S25-004: Restrict ACL on export ZIP (S-L04)
- S25-005: Migrate display.rs to RegKeyGuard (A-L01)
- S25-006: Migrate charging.rs to RegKeyGuard (A-L02)
- S25-007: Migrate hotkeys.rs to RegKeyGuard (A-L03)
- S25-008: Remove WMI queries from error Display (A-L04)
- S25-009: Return error on refresh rate failure (A-L05)
- S25-010: Replace unreachable!() with return Err (A-L07)

Batch B - Frontend & Config:
- S25-011: Add form-action to CSP (S-L03)
- S25-012: Remove api.openai.com from CSP (S-L04)
- S25-013: Add aria-label to API key button (U-L01)
- S25-014: Add role=progressbar to onboarding dots (U-L02)
- S25-015: Extract AiAnalysis inline styles (U-L04)
- S25-016: Change opt-level to 3 (P-L02)
- S25-017: Fix LineChart SVG distortion (P-L03)
- S25-018: Skip brightness loop when display off (P-L04)

Additional: A-L06, D-L01-L05, AI-L01-L02"
```
