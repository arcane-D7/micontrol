# Sprint 26 — P2 MEDIUM: Residual Blocking I/O, ACL Gaps, Rate Limiting (Post-Audit v3)

## Sprint Metadata

| Field                 | Value                                               |
| --------------------- | --------------------------------------------------- |
| **Sprint Name**       | Residual Blocking I/O, ACL Gaps, Rate Limiting      |
| **Sprint Goal**       | Fix all 7 MEDIUM findings from Stability Report v3  |
| **Duration Estimate** | ~3 days                                             |
| **Priority**          | P2 — Medium                                         |
| **Sprint Type**       | Multi-domain (Backend, Security, DevOps)            |
| **Primary Owner**     | Full-stack engineer                                 |
| **Source**            | `docs/STABILITY_REPORT_v3.md` — All MEDIUM findings |
| **Depends On**        | Sprint 25                                           |

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **OBRIGATÓRIO: 100% dos tickets desta sprint devem ser concluídos. A sprint não será aceita como entregue se qualquer ticket permanecer incompleto.**
>
> **MANDATORY: 100% of the tickets in this sprint MUST be completed. The sprint will NOT be accepted as delivered if any ticket remains incomplete.**

Every ticket must pass its acceptance criteria AND the full health check suite (9/9) before the sprint commit is made.

---

## Sprint Goal Statement

The post-sprint-25 stability audit (v3) identified 7 MEDIUM findings across 3 domains. These are residual blocking I/O patterns that escaped the S24-013 sweep, ACL restriction gaps on sensitive files, a missing rate limit on `test_connection`, an unimplemented key rotation handler, and a DevOps shell compatibility issue. This sprint batches them into 2 execution groups:

- **Batch A (Rust Backend):** Blocking I/O residuals, ACL gaps, rate limiting, key rotation
- **Batch B (DevOps):** Release workflow shell compatibility

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

## Batch A — Rust Backend (S26-001 through S26-007)

### S26-001 — Add rate limiting to `test_connection`

| Field                | Value                                                              |
| -------------------- | ------------------------------------------------------------------ |
| **Ticket ID**        | S26-001                                                            |
| **Title**            | Add `check_daily_limit()` or cooldown to `test_connection` command |
| **Priority**         | P2                                                                 |
| **Type**             | Security / AI Responsibility                                       |
| **Estimated Effort** | S                                                                  |
| **Source Finding**   | S-001, AI-001 (MEDIUM)                                             |

#### Context

In `src-tauri/src/commands/ai.rs:206-260`, `test_connection` performs a live API call (sending the bearer token to the user-configured `base_url`) without any rate limiting. The `analyze_system` command correctly calls `check_daily_limit(ai_daily_analyses)` before API requests, but `test_connection` does not. An attacker or buggy frontend could call `test_connection` in a tight loop, causing excessive API usage/cost.

#### Acceptance Criteria

- [ ] `test_connection` calls `check_daily_limit()` or a separate cooldown mechanism before the API request
- [ ] If rate limit exceeded, returns a descriptive error (e.g., `"rate_limit_exceeded"`)
- [ ] Alternatively, track `test_connection` calls in the usage counter
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S26-002 — Add ACL restriction to `ai_usage.json`

| Field                | Value                                                     |
| -------------------- | --------------------------------------------------------- |
| **Ticket ID**        | S26-002                                                   |
| **Title**            | Call `restrict_file_acl` on `ai_usage.json` after writing |
| **Priority**         | P2                                                        |
| **Type**             | Security / Privacy                                        |
| **Estimated Effort** | XS                                                        |
| **Source Finding**   | S-002 (MEDIUM)                                            |

#### Context

In `src-tauri/src/util/ai_usage.rs:73-82`, `save_to_file()` writes with plain `std::fs::write` without calling `restrict_file_acl`. The file contains `total_requests`, token counts, and `estimated_cost_usd`. Other sensitive files in `%LOCALAPPDATA%\MiControl\` (elev_key.bin, nonces.json) get ACL-restricted, but `ai_usage.json` inherits default permissions.

#### Acceptance Criteria

- [ ] `restrict_file_acl` called after `std::fs::write` in `save_to_file()`
- [ ] Log warning if ACL restriction fails (but don't crash)
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S26-003 — Add ACL restriction to `consent_audit.log`

| Field                | Value                                                             |
| -------------------- | ----------------------------------------------------------------- |
| **Ticket ID**        | S26-003                                                           |
| **Title**            | Call `restrict_file_acl` on `consent_audit.log` on first creation |
| **Priority**         | P2                                                                |
| **Type**             | Security / Privacy                                                |
| **Estimated Effort** | XS                                                                |
| **Source Finding**   | S-003 (MEDIUM)                                                    |

#### Context

In `src-tauri/src/util/consent_audit.rs:170-185`, `log_consent_event` opens the file with `OpenOptions::new().create(true).append(true)` without ACL restriction. The file contains consent grant/revoke timestamps and HMAC tags. World-readable by default on Windows.

#### Acceptance Criteria

- [ ] `restrict_file_acl` called when the file is first created (check if exists before creating)
- [ ] Log warning if ACL restriction fails (but don't crash)
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S26-004 — Implement `--rotate-key` handler or auto-rotate

| Field                | Value                                                             |
| -------------------- | ----------------------------------------------------------------- |
| **Ticket ID**        | S26-004                                                           |
| **Title**            | Implement HMAC key rotation CLI handler or auto-rotate at startup |
| **Priority**         | P2                                                                |
| **Type**             | Security / Defense-in-Depth                                       |
| **Estimated Effort** | S                                                                 |
| **Source Finding**   | S-004 (MEDIUM)                                                    |

#### Context

In `src-tauri/src/lib.rs:~395`, `key_needs_rotation()` is called at startup and logs a warning: "HMAC key rotation needed — run with --rotate-key to rotate". However, `main.rs` only handles `--elevated` — there is no `--rotate-key` argument handler. The `rotate_key()` function exists in `auth.rs` but is never called by any code path. The warning message instructs the user to run a flag that doesn't exist.

#### Acceptance Criteria

- [ ] Option A: Implement `--rotate-key` CLI handler in `main.rs` that calls `rotate_key()`
- [ ] Option B: Auto-rotate at startup when `key_needs_rotation()` returns true (in `spawn_blocking`)
- [ ] Option C: Remove the misleading log message and document manual rotation as operational procedure
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S26-005 — Wrap `launch_elevated_via_uac()` timeout fallback in `spawn_blocking`

| Field                | Value                                                                  |
| -------------------- | ---------------------------------------------------------------------- |
| **Ticket ID**        | S26-005                                                                |
| **Title**            | Wrap timeout-path `launch_elevated_via_uac()` call in `spawn_blocking` |
| **Priority**         | P2                                                                     |
| **Type**             | Performance / Concurrency                                              |
| **Estimated Effort** | XS                                                                     |
| **Source Finding**   | A-001 (MEDIUM)                                                         |

#### Context

In `src-tauri/src/elev_bridge.rs:202`, the timeout fallback path of `run_elevated()` calls `launch_elevated_via_uac(&request_id)` directly on the Tokio async runtime worker thread. This function calls `WaitForSingleObject(info.hProcess, 30_000)` — a synchronous 30-second blocking wait. The initial UAC fallback at line 130 correctly wraps this in `spawn_blocking`, but the timeout-path call does not.

#### Acceptance Criteria

- [ ] Timeout-path `launch_elevated_via_uac()` call wrapped in `tokio::task::spawn_blocking`
- [ ] Error handling maps `JoinError` appropriately
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S26-006 — Wrap `cleanup_stale_elev_files()` in `spawn_blocking`

| Field                | Value                                                 |
| -------------------- | ----------------------------------------------------- |
| **Ticket ID**        | S26-006                                               |
| **Title**            | Wrap `cleanup_stale_elev_files()` in `spawn_blocking` |
| **Priority**         | P2                                                    |
| **Type**             | Performance / Concurrency                             |
| **Estimated Effort** | XS                                                    |
| **Source Finding**   | A-002 (MEDIUM)                                        |

#### Context

In `src-tauri/src/elev_bridge.rs:63`, `cleanup_stale_elev_files(&dir)` is called inside the async `run_elevated()` function. It uses `std::fs::read_dir()` — synchronous filesystem I/O on the Tokio worker thread. While typically fast, directory enumeration on a slow disk can take milliseconds to seconds.

#### Acceptance Criteria

- [ ] `cleanup_stale_elev_files()` call wrapped in `tokio::task::spawn_blocking`
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S26-007 — Wrap `hw_get_ai_cfg()` in `set_brightness` with `run_blocking`

| Field                | Value                                                               |
| -------------------- | ------------------------------------------------------------------- |
| **Ticket ID**        | S26-007                                                             |
| **Title**            | Wrap `hw_get_ai_cfg()` call in `set_brightness` with `run_blocking` |
| **Priority**         | P2                                                                  |
| **Type**             | Performance / Concurrency                                           |
| **Estimated Effort** | XS                                                                  |
| **Source Finding**   | A-003 (MEDIUM)                                                      |

#### Context

In `src-tauri/src/commands/system.rs:75`, the `set_brightness` Tauri command calls `hw_get_ai_cfg()` directly without `run_blocking`. The sibling command `get_ai_brightness_config` (line 110) correctly wraps the same call. `hw_get_ai_cfg()` does 4+ synchronous registry reads via `RegKeyGuard::open_read`.

#### Acceptance Criteria

- [ ] `hw_get_ai_cfg()` call in `set_brightness` wrapped in `run_blocking`
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

## Batch B — DevOps (S26-008)

### S26-008 — Fix release workflow bash syntax on Windows

| Field                | Value                                                    |
| -------------------- | -------------------------------------------------------- |
| **Ticket ID**        | S26-008                                                  |
| **Title**            | Add `shell: bash` or rewrite signing check in PowerShell |
| **Priority**         | P2                                                       |
| **Type**             | DevOps / CI                                              |
| **Estimated Effort** | XS                                                       |
| **Source Finding**   | D-001 (MEDIUM)                                           |

#### Context

In `.github/workflows/release.yml`, the code signing verification step uses bash syntax (`if [ -z ... ]`) on a `windows-latest` runner without specifying `shell: bash`. GitHub Actions defaults to PowerShell on Windows runners, which will fail on bash syntax.

#### Acceptance Criteria

- [ ] Either add `shell: bash` to the step, or rewrite the check in PowerShell
- [ ] Verify the logic is correct: fail if `WINDOWS_CERTIFICATE` secret is not set
- [ ] No `continue-on-error` on the signing step

---

## Sprint Commit

```bash
git add -A
git commit -m "fix(s26): residual blocking I/O, ACL gaps, rate limiting, key rotation, DevOps shell fix (P2)

Batch A - Rust Backend:
- S26-001: Rate limit on test_connection (S-001, AI-001)
- S26-002: ACL restriction on ai_usage.json (S-002)
- S26-003: ACL restriction on consent_audit.log (S-003)
- S26-004: Key rotation handler or auto-rotate (S-004)
- S26-005: spawn_blocking for UAC timeout path (A-001)
- S26-006: spawn_blocking for cleanup_stale_elev_files (A-002)
- S26-007: run_blocking for hw_get_ai_cfg in set_brightness (A-003)

Batch B - DevOps:
- S26-008: Fix release workflow bash syntax on Windows (D-001)"
```
