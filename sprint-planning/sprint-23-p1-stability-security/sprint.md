# Sprint 23 — P1 HIGH: Stability & Security Edge Cases (Post-Audit v2)

## Sprint Metadata

| Field                 | Value                                                                                                                    |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| **Sprint Name**       | Stability & Security Edge Cases                                                                                          |
| **Sprint Goal**       | Fix infinite loop on pipe EOF, hardcoded ERAM address, registry data loss, frontend test coverage, and AI consent bypass |
| **Duration Estimate** | ~3 days                                                                                                                  |
| **Priority**          | P1 — High                                                                                                                |
| **Sprint Type**       | Multi-domain (Backend, Frontend, AI Responsibility)                                                                      |
| **Primary Owner**     | Full-stack engineer                                                                                                      |
| **Source**            | `docs/STABILITY_REPORT_v2.md` — Findings A-H01, A-H02, A-H03, U-H01, AI-H01                                              |
| **Depends On**        | Sprint 22                                                                                                                |

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **OBRIGATÓRIO: 100% dos tickets desta sprint devem ser concluídos. A sprint não será aceita como entregue se qualquer ticket permanecer incompleto.**
>
> **MANDATORY: 100% of the tickets in this sprint MUST be completed. The sprint will NOT be accepted as delivered if any ticket remains incomplete.**

Every ticket must pass its acceptance criteria AND the full health check suite (9/9) before the sprint commit is made.

---

## Sprint Goal Statement

The post-sprint-21 stability audit (v2) identified 5 HIGH findings across backend stability, frontend test coverage, and AI responsibility. These are not runtime crashes but edge cases that can cause infinite loops, silent data loss, or consent bypass:

1. **A-H01**: `read_exact_timeout` in `iotservice.rs` doesn't check `bytes_read == 0`, causing an infinite spin when the pipe closes.
2. **A-H02**: `is_eram_range` in `hardware.rs` uses compile-time `ERAM_BASE` constant instead of DSDT-discovered `get_eram_base()`.
3. **A-H03**: `read_string` in `registry.rs` silently drops values > 512 bytes — `ERROR_MORE_DATA` is treated as missing.
4. **U-H01**: Only 7 frontend test files vs 30+ components, coverage threshold at 40%.
5. **AI-H01**: `test_connection` in `ai.rs` sends API key to external server without verifying telemetry consent.

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

### S23-001 — Fix infinite loop on pipe EOF in `read_exact_timeout`

| Field                | Value                                                              |
| -------------------- | ------------------------------------------------------------------ |
| **Ticket ID**        | S23-001                                                            |
| **Title**            | Check `bytes_read == 0` in `read_exact_timeout` to detect pipe EOF |
| **Priority**         | P1                                                                 |
| **Type**             | Stability / Bug Fix                                                |
| **Estimated Effort** | S                                                                  |
| **Source Finding**   | A-H01 (HIGH)                                                       |

#### Context

In `src-tauri/src/hw/iotservice.rs:629-730`, the `read_exact_timeout` function uses overlapped I/O to read from a named pipe. After `ReadFile` completes (either synchronously or via `GetOverlappedResult`), it adds `bytes_read` to `filled` and loops. However, if the pipe is closed by the remote end, `ReadFile` returns success with `bytes_read == 0`. The loop never progresses (`filled` stays the same) and never terminates, causing an infinite spin that consumes 100% CPU.

The current code after synchronous completion:

```rust
if result.is_ok() {
    // Completed synchronously
    filled += bytes_read as usize;
} else {
    // ... overlapped wait path ...
    filled += bytes_read as usize;
}
```

No check for `bytes_read == 0` exists in either path.

#### Acceptance Criteria

- [x] After both the synchronous and overlapped completion paths, check if `bytes_read == 0`
- [x] If `bytes_read == 0`, return `Err` with a descriptive message (e.g., `"Pipe closed by remote end (EOF)"`)
- [x] Use `anyhow::bail!` or return `Err(anyhow::anyhow!(...))` consistent with the function's error handling
- [x] Add a unit test that verifies the function returns an error when reading from a closed/empty pipe (similar to the existing `test_read_exact_timeout_zero_length` test)
- [x] `cargo check --manifest-path src-tauri/Cargo.toml` passes
- [x] `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` passes
- [x] `cargo test --manifest-path src-tauri/Cargo.toml` passes

---

### S23-002 — Use `get_eram_base()` instead of hardcoded `ERAM_BASE` in `is_eram_range`

| Field                | Value                                                               |
| -------------------- | ------------------------------------------------------------------- |
| **Ticket ID**        | S23-002                                                             |
| **Title**            | Replace hardcoded `ERAM_BASE` with `get_eram_base()` in range check |
| **Priority**         | P1                                                                  |
| **Type**             | Correctness / Bug Fix                                               |
| **Estimated Effort** | XS                                                                  |
| **Source Finding**   | A-H02 (HIGH)                                                        |

#### Context

In `src-tauri/src/commands/hardware.rs:413-417`, the `is_eram_range` function uses the compile-time constant `crate::hw::ecram::ERAM_BASE` to validate EC RAM write addresses. However, the codebase has a `get_eram_base()` function in `ecram.rs:102` that performs DSDT-based auto-discovery of the actual ERAM base address, falling back to `ERAM_BASE_FALLBACK`. Using the hardcoded constant means writes to the DSDT-discovered address range would be incorrectly rejected (or writes to the wrong range incorrectly accepted) on machines where the DSDT address differs from the fallback.

The current code:

```rust
fn is_eram_range(addr: u64, len: usize) -> bool {
    if len == 0 {
        return false;
    }
    let start = crate::hw::ecram::ERAM_BASE;
    let end = start + crate::hw::ecram::ERAM_SIZE as u64;
    let write_end = addr.saturating_add(len as u64);
    addr >= start && write_end <= end
}
```

The same issue exists in `is_known_safe_single_byte_write` which also references `ERAM_BASE`:

```rust
let offset = (addr - crate::hw::ecram::ERAM_BASE) as usize;
```

#### Acceptance Criteria

- [x] `is_eram_range` uses `crate::hw::ecram::get_eram_base()` instead of `ERAM_BASE`
- [x] `is_known_safe_single_byte_write` uses `get_eram_base()` for the offset calculation
- [x] `get_eram_base()` is confirmed `pub` (it already is — `ecram.rs:102`)
- [x] `cargo check --manifest-path src-tauri/Cargo.toml` passes
- [x] `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` passes
- [x] `cargo test --manifest-path src-tauri/Cargo.toml` passes

---

### S23-003 — Retry with larger buffer on `ERROR_MORE_DATA` in `read_string`

| Field                | Value                                                                                |
| -------------------- | ------------------------------------------------------------------------------------ |
| **Ticket ID**        | S23-003                                                                              |
| **Title**            | Handle `ERROR_MORE_DATA` in `RegKeyGuard::read_string` to support values > 512 bytes |
| **Priority**         | P1                                                                                   |
| **Type**             | Bug Fix / Data Loss                                                                  |
| **Estimated Effort** | S                                                                                    |
| **Source Finding**   | A-H03 (HIGH)                                                                         |

#### Context

In `src-tauri/src/util/registry.rs:110-125`, the `read_string` method allocates a 512-byte buffer and calls `RegQueryValueExW`. If the registry value is larger than 512 bytes, Windows returns `ERROR_MORE_DATA` (code 234), but the current code treats any error as "value not found" and returns `Ok(None)`. This silently drops registry values > 512 bytes, which can cause missing configuration data.

The current code:

```rust
let mut buf = [0u16; 256]; // 256 u16 = 512 bytes
let mut buf_len: u32 = buf.len() as u32 * 2; // bytes
// ...
let result = unsafe {
    RegQueryValueExW(
        self.as_raw(),
        PCWSTR(name_w.as_ptr()),
        None,
        Some(&mut value_type),
        Some(buf.as_mut_ptr() as *mut u8),
        Some(&mut buf_len),
    )
};
if result.is_err() {
    return Ok(None);
}
```

#### Acceptance Criteria

- [x] After `RegQueryValueExW` returns an error, check if the error code is `ERROR_MORE_DATA` (234)
- [x] If `ERROR_MORE_DATA`, read the required size from `buf_len`, allocate a new buffer of that size, and retry the query
- [x] Limit total buffer size to 64KB (prevent unbounded allocation from malicious registry values)
- [x] If retry fails or buffer exceeds 64KB, return `Ok(None)` (same as current behavior for missing values)
- [x] Add a unit test (or integration test) that verifies large registry values are read correctly (may require mocking or a test registry key)
- [x] `cargo check --manifest-path src-tauri/Cargo.toml` passes
- [x] `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` passes
- [x] `cargo test --manifest-path src-tauri/Cargo.toml` passes

---

### S23-004 — Add frontend tests for hooks and critical components

| Field                | Value                                                                                               |
| -------------------- | --------------------------------------------------------------------------------------------------- |
| **Ticket ID**        | S23-004                                                                                             |
| **Title**            | Add tests for `useHardware`, `useSettings`, `ConsentDialog`, `AiConfigForm`, and `OnboardingWizard` |
| **Priority**         | P1                                                                                                  |
| **Type**             | Test Coverage                                                                                       |
| **Estimated Effort** | L                                                                                                   |
| **Source Finding**   | U-H01 (HIGH)                                                                                        |

#### Context

The frontend has only 7 test files for 30+ components. The coverage threshold in `vite.config.ts` is set to 40%. Critical hooks (`useHardware`, `useSettings`) and components (`ConsentDialog`, `AiConfigForm`, `OnboardingWizard`) have no tests at all. This means regressions in these components go undetected.

#### Acceptance Criteria

- [x] Create `src/__tests__/useHardware.test.ts` — test hook initialization, error state, polling behavior
- [x] Create `src/__tests__/useSettings.test.ts` — test settings load/save, API key migration
- [x] Create `src/__tests__/ConsentDialog.test.tsx` — test consent grant/deny, focus trap, Escape key
- [x] Create `src/__tests__/AiConfigForm.test.tsx` — test API key input, show/hide toggle, validation
- [x] Create `src/__tests__/OnboardingWizard.test.tsx` — test step navigation, completion, skip
- [x] Mock Tauri `invoke` in all tests using `vi.mock('@tauri-apps/api/core')`
- [x] Raise coverage threshold in `vite.config.ts` from 40% to 50% (lines, functions, branches)
- [x] `npx tsc --noEmit` passes
- [x] `npm run lint` passes
- [x] `npm run build` passes
- [x] All new tests pass

---

### S23-005 — Add consent check to `test_connection` AI command

| Field                | Value                                                                                     |
| -------------------- | ----------------------------------------------------------------------------------------- |
| **Ticket ID**        | S23-005                                                                                   |
| **Title**            | Add `check_consent()` call to `test_connection` before sending API key to external server |
| **Priority**         | P1                                                                                        |
| **Type**             | AI Responsibility / Privacy                                                               |
| **Estimated Effort** | XS                                                                                        |
| **Source Finding**   | AI-H01 (HIGH)                                                                             |

#### Context

In `src-tauri/src/commands/ai.rs:131-170`, the `test_connection` function sends the user's API key to an external AI server (`base_url`) to test connectivity. However, unlike `analyze_system` (which checks consent at line 76-79), `test_connection` does not verify that the user has granted telemetry consent. This means the API key can be sent to an external server without the user's explicit consent, violating GDPR and the app's own consent framework.

The `analyze_system` function has this check:

```rust
if !crate::util::consent_audit::has_consent("telemetry") {
    return Err("Telemetry consent not granted".to_string());
}
```

But `test_connection` at line 131 goes straight to reading the API key and making the HTTP request.

#### Acceptance Criteria

- [x] `test_connection` checks telemetry consent before any network request
- [x] If consent is not granted, returns an error with a descriptive message (same pattern as `analyze_system`)
- [x] The consent check is placed before the `reqwest::Client` builder call
- [x] `cargo check --manifest-path src-tauri/Cargo.toml` passes
- [x] `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` passes
- [x] `cargo test --manifest-path src-tauri/Cargo.toml` passes

---

## Sprint Commit

```bash
git add -A
git commit -m "fix(sprint-23): stability edge cases, test coverage, and AI consent enforcement (P1)

- S23-001: Fix infinite loop on pipe EOF in read_exact_timeout (A-H01)
- S23-002: Use get_eram_base() instead of hardcoded ERAM_BASE (A-H02)
- S23-003: Handle ERROR_MORE_DATA in registry read_string (A-H03)
- S23-004: Add frontend tests for hooks and critical components (U-H01)
- S23-005: Add consent check to test_connection AI command (AI-H01)"
```
