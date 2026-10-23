# Sprint 22 — P0 CRITICAL: Async Blocking I/O (Post-Audit v2)

## Sprint Metadata

| Field                 | Value                                                                                      |
| --------------------- | ------------------------------------------------------------------------------------------ |
| **Sprint Name**       | Async Blocking I/O                                                                         |
| **Sprint Goal**       | Eliminate Tokio worker thread starvation from synchronous blocking calls in async contexts |
| **Duration Estimate** | ~1 day                                                                                     |
| **Priority**          | P0 — Critical, blocks stability                                                            |
| **Sprint Type**       | Architecture / Concurrency                                                                 |
| **Primary Owner**     | Backend engineer                                                                           |
| **Source**            | `docs/STABILITY_REPORT_v2.md` — Findings A-C01, A-C02                                      |
| **Depends On**        | Sprint 21 (post-audit v2 baseline)                                                         |

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **OBRIGATÓRIO: 100% dos tickets desta sprint devem ser concluídos. A sprint não será aceita como entregue se qualquer ticket permanecer incompleto.**
>
> **MANDATORY: 100% of the tickets in this sprint MUST be completed. The sprint will NOT be accepted as delivered if any ticket remains incomplete.**

Every ticket must pass its acceptance criteria AND the full health check suite (9/9) before the sprint commit is made.

---

## Sprint Goal Statement

The post-sprint-21 stability audit (v2) identified 2 CRITICAL findings in `elev_bridge.rs`. Both are synchronous blocking calls executed directly on Tokio async worker threads:

1. **A-C01**: `WaitForSingleObject(info.hProcess, 30_000)` in `launch_elevated_via_uac()` blocks a Tokio worker for up to 30 seconds while waiting for the elevated process to exit.
2. **A-C02**: `auth::get_or_create_key()` performs synchronous file I/O with a 5-second polling loop from an async context.

Both calls can starve the Tokio runtime, causing the entire application to freeze. The fix is mechanical: wrap each blocking call in `tokio::task::spawn_blocking`.

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

## Tickets

### S22-001 — Wrap `launch_elevated_via_uac` in `spawn_blocking`

| Field                | Value                                                                            |
| -------------------- | -------------------------------------------------------------------------------- |
| **Ticket ID**        | S22-001                                                                          |
| **Title**            | Wrap `launch_elevated_via_uac()` in `spawn_blocking` to prevent async starvation |
| **Priority**         | P0                                                                               |
| **Type**             | Architecture / Concurrency                                                       |
| **Estimated Effort** | S                                                                                |
| **Source Finding**   | A-C01 (CRITICAL)                                                                 |

#### Context

In `elev_bridge.rs`, the `run_elevated()` async function calls `launch_elevated_via_uac(&request_id)` directly when the scheduled task is unavailable (dev mode fallback). This function internally calls `WaitForSingleObject(info.hProcess, 30_000)` which blocks the calling thread for up to 30 seconds. Since `run_elevated()` is `async`, this blocks a Tokio worker thread, potentially starving the runtime and freezing the entire application.

The current code at approximately line 130:

```rust
if let Err(e) = launch_elevated_via_uac(&request_id) {
    let _ = tokio::fs::remove_file(&cmd_path).await;
    return Err(format!(
        "Scheduled task '{}' not found AND UAC fallback failed: {e}. \
         Reinstall MiControl to register the scheduled task.",
        TASK_NAME
    ));
}
```

#### Acceptance Criteria

- [x] `launch_elevated_via_uac(&request_id)` is wrapped in `tokio::task::spawn_blocking`
- [x] The `request_id` is moved into the closure (owned `String`, not `&str`)
- [x] `JoinError` from `spawn_blocking` is mapped to a descriptive error string
- [x] The inner `Result<(), String>` from `launch_elevated_via_uac` is propagated correctly (double `?` or explicit match)
- [x] `cargo check --manifest-path src-tauri/Cargo.toml` passes
- [x] `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` passes
- [x] `cargo test --manifest-path src-tauri/Cargo.toml` passes
- [x] No behavior change — the UAC fallback still works identically, just off the async thread

---

### S22-002 — Wrap `get_or_create_key` in `spawn_blocking`

| Field                | Value                                                                            |
| -------------------- | -------------------------------------------------------------------------------- |
| **Ticket ID**        | S22-002                                                                          |
| **Title**            | Wrap `auth::get_or_create_key()` in `spawn_blocking` to prevent async starvation |
| **Priority**         | P0                                                                               |
| **Type**             | Architecture / Concurrency                                                       |
| **Estimated Effort** | S                                                                                |
| **Source Finding**   | A-C02 (CRITICAL)                                                                 |

#### Context

In `elev_bridge.rs`, the `run_elevated()` async function calls `auth::get_or_create_key()` directly at approximately line 95. This function performs synchronous file I/O (reading/writing the HMAC key file) and contains a 5-second polling loop to wait for key availability. Running this on a Tokio worker thread blocks the async runtime.

The current code:

```rust
let key = auth::get_or_create_key().map_err(|e| format!("Cannot obtain HMAC key: {e}"))?;
```

#### Acceptance Criteria

- [x] `auth::get_or_create_key()` is wrapped in `tokio::task::spawn_blocking`
- [x] `JoinError` from `spawn_blocking` is mapped to a descriptive error string
- [x] The inner `Result<_, String>` from `get_or_create_key` is propagated correctly
- [x] The resulting `key` variable has the same type (`[u8; N]` or `Vec<u8>`) as before
- [x] `cargo check --manifest-path src-tauri/Cargo.toml` passes
- [x] `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` passes
- [x] `cargo test --manifest-path src-tauri/Cargo.toml` passes
- [x] No behavior change — key creation/retrieval works identically

---

## Sprint Commit

```bash
git add -A
git commit -m "fix(sprint-22): wrap blocking I/O in spawn_blocking to prevent async starvation (P0)

- S22-001: Wrap launch_elevated_via_uac in spawn_blocking (A-C01)
- S22-002: Wrap auth::get_or_create_key in spawn_blocking (A-C02)"
```
