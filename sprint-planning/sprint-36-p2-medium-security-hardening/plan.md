# Sprint 36 — P2 MEDIUM: Security Hardening & Defense in Depth

> **Date:** 2026-07-19
> **Sprint:** 36
> **Theme:** Fix 4 security issues — atomic writes, directory ACL, diag_ps gate, USERNAME spoofing
> **Duration:** ~2–3 days
> **Dependencies:** Sprint 34 (Auth Bridge fixes must be completed first, as they modify the same files)
> **Status:** ✅ Complete
> **Commit:** `0a67897` — `fix(s36): harden elevated bridge with atomic writes, ACL, diag_ps gate, and GetUserNameW`
> **Audit Reference:** `C:\Users\mafsc\Documents\Audit_Report_miControl.md` (Bug A, B, C, D)

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **OBRIGATÓRIO: 100% dos tickets desta sprint devem ser concluídos. A sprint não será aceita como entregue se qualquer ticket permanecer incompleto.**
>
> **MANDATORY: 100% of the tickets in this sprint MUST be completed. The sprint will NOT be accepted as delivered if any ticket remains incomplete.**

---

## Health Check Commands (must pass before commit)

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

## Executive Summary

This sprint addresses 4 security issues discovered during the audit. While none are the root cause of the user-reported bugs, they represent defense-in-depth gaps that should be fixed in the same release cycle. All issues are in the elevated bridge subsystem and modify files already touched by Sprint 34.

1. **S36-001:** Make elevated result file write atomic (temp+rename pattern)
2. **S36-002:** Apply `restrict_file_acl` to the `elev_dir()` directory itself
3. **S36-003:** Gate `diag_ps` command behind `#[cfg(feature = "diag")]` cargo feature
4. **S36-004:** Replace spoofable `USERNAME` env var with `GetUserNameW` Win32 API in `restrict_file_acl`

---

## Goals

| #   | Goal                                                | KPI                                             | Audit Reference |
| --- | --------------------------------------------------- | ----------------------------------------------- | --------------- |
| 1   | Result file writes are atomic (no partial reads)    | temp+rename pattern in `elevated.rs`            | Bug C           |
| 2   | `elev_dir()` directory has restricted ACL           | `restrict_file_acl(&dir)` called after creation | Bug D           |
| 3   | `diag_ps` not available in production builds        | `#[cfg(feature = "diag")]` gate                 | Bug A           |
| 4   | `restrict_file_acl` uses process token, not env var | `GetUserNameW` instead of `std::env::var`       | Bug B           |

---

## Technical Specs

### S36-001: Make elevated result file write atomic (Bug C)

| Field         | Value                                                     |
| ------------- | --------------------------------------------------------- |
| **Ticket ID** | S36-001                                                   |
| **Title**     | Use temp+rename pattern for writing elevated result files |
| **Priority**  | P2 — Medium                                               |
| **Source**    | Bug C (`Audit_Report_miControl.md`)                       |
| **Files**     | `src-tauri/src/elevated.rs` (lines 142–149)               |
| **Effort**    | ~20 minutes                                               |
| **Type**      | Backend (Rust)                                            |

#### Problem

`std::fs::write` truncates then writes. If the elevated helper is killed mid-write (by `StopExisting` — fixed in S34-002 — or `ExecutionTimeLimit`), the result file exists but is partial/empty. The main process polls, sees the file exists, reads partial JSON, and fails with `Invalid result JSON`.

The command-file write in `elev_bridge.rs:99–105` already uses the atomic temp+rename pattern. The nonce store in `elevated.rs:175–187` also uses it. But the result file write does NOT.

#### Current Code

**`elevated.rs` lines 142–149:**

```rust
let _ = std::fs::write(&pending.result_path, json);
if let Err(e) = auth::restrict_file_acl(&pending.result_path) {
    log::warn!("Failed to restrict ACL on result file: {e}");
}
// S24-001: Flush nonces before exit to prevent nonce loss.
flush_nonces();
std::process::exit(0);
```

#### Solution

Use the same temp+rename pattern already used for command files:

```rust
let tmp_path = pending.result_path.with_extension("json.tmp");
match std::fs::write(&tmp_path, &json) {
    Ok(()) => {
        if let Err(e) = std::fs::rename(&tmp_path, &pending.result_path) {
            let _ = std::fs::remove_file(&tmp_path);
            log::warn!("Failed to atomically rename result file: {e}");
        } else if let Err(e) = auth::restrict_file_acl(&pending.result_path) {
            log::warn!("Failed to restrict ACL on result file: {e}");
        }
    }
    Err(e) => {
        log::warn!("Failed to write result file: {e}");
    }
}
// S24-001: Flush nonces before exit to prevent nonce loss.
flush_nonces();
std::process::exit(0);
```

**Note:** On Windows, `std::fs::rename` is atomic within the same volume (which it is — both files are in `%LOCALAPPDATA%\MiControl`).

#### Acceptance Criteria

- [ ] Result file write uses temp+rename pattern
- [ ] Temp file is cleaned up on rename failure
- [ ] `restrict_file_acl` called only after successful rename
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes

---

### S36-002: Apply `restrict_file_acl` to `elev_dir()` directory (Bug D)

| Field         | Value                                                     |
| ------------- | --------------------------------------------------------- |
| **Ticket ID** | S36-002                                                   |
| **Title**     | Add `restrict_file_acl` call to the `elev_dir()` function |
| **Priority**  | P2 — Medium                                               |
| **Source**    | Bug D (`Audit_Report_miControl.md`)                       |
| **Files**     | `src-tauri/src/elevated.rs` (lines 1317–1325)             |
| **Effort**    | ~10 minutes                                               |
| **Type**      | Backend (Rust)                                            |

#### Problem

`elev_dir()` creates the `%LOCALAPPDATA%\MiControl` directory with `create_dir_all` but does NOT apply `restrict_file_acl` to the directory itself. Individual files (`elev_key.bin`, `elev_cmd_*.json`, `nonces.json`) get ACL'd, but the directory does not.

**Impact:**

- Another user on the same machine can list the directory and see filenames (including `request_id`s in `elev_cmd_<id>.json`)
- An attacker can pre-create files in the directory (if default ACL allows it)
- An attacker can pre-create a symlink at `elev_key.bin` before the legitimate process creates it (TOCTOU symlink attack)

#### Current Code

**`elevated.rs` lines 1317–1325:**

```rust
pub fn elev_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
    let dir = PathBuf::from(base).join("MiControl");
    let _ = std::fs::create_dir_all(&dir);
    dir
}
```

#### Solution

Apply `restrict_file_acl` to the directory after creation:

```rust
pub fn elev_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
    let dir = PathBuf::from(base).join("MiControl");
    let _ = std::fs::create_dir_all(&dir);
    // Lock down the directory itself so other users can't list files or
    // pre-create symlink attacks against elev_key.bin.
    let _ = crate::util::auth::restrict_file_acl(&dir);
    dir
}
```

**Note:** `restrict_file_acl` currently uses `NO_INHERITANCE` (`auth.rs:319`). For a directory, this is sufficient to prevent listing/creation by other users. Each file already calls `restrict_file_acl` explicitly, so inheritance is not needed.

#### Acceptance Criteria

- [ ] `restrict_file_acl(&dir)` called after `create_dir_all` in `elev_dir()`
- [ ] Directory ACL restricts access to current user + SYSTEM only
- [ ] Other users cannot list files in the directory (manual verification)
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes

---

### S36-003: Gate `diag_ps` behind `#[cfg(feature = "diag")]` (Bug A)

| Field         | Value                                                               |
| ------------- | ------------------------------------------------------------------- |
| **Ticket ID** | S36-003                                                             |
| **Title**     | Gate `diag_ps` elevated command behind a cargo feature flag         |
| **Priority**  | P2 — Medium                                                         |
| **Source**    | Bug A (`Audit_Report_miControl.md`)                                 |
| **Files**     | `src-tauri/src/elevated.rs` (lines 613–636), `src-tauri/Cargo.toml` |
| **Effort**    | ~20 minutes                                                         |
| **Type**      | Backend (Rust)                                                      |

#### Problem

The `diag_ps` command allows arbitrary PowerShell execution with elevated privileges. While there are zero callers in the frontend (confirmed via grep), it's a loaded gun — if the HMAC key is compromised, `diag_ps` is a direct privilege escalation to elevated PowerShell.

The command is reachable via the elevated bridge protocol, which requires a valid HMAC. But defense in depth dictates that unused attack surface should be removed from production builds.

#### Current Code

**`elevated.rs` lines 613–636:**

```rust
"diag_ps" => {
    let script = cmd.args["script"].as_str().unwrap_or("");
    if script.is_empty() {
        return make_err("Missing 'script' argument".to_string());
    }
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output();
    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            make_ok(serde_json::json!({
                "stdout": stdout,
                "stderr": stderr,
                "exit_code": out.status.code().unwrap_or(-1),
            }))
        }
        Err(e) => make_err(format!("Failed to run PowerShell: {e}")),
    }
}
```

#### Solution

**Step 1:** Add `diag` feature to `Cargo.toml`:

```toml
[features]
default = []
diag = []
```

**Step 2:** Gate the `diag_ps` match arm:

```rust
#[cfg(feature = "diag")]
"diag_ps" => {
    let script = cmd.args["script"].as_str().unwrap_or("");
    if script.is_empty() {
        return make_err("Missing 'script' argument".to_string());
    }
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output();
    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            make_ok(serde_json::json!({
                "stdout": stdout,
                "stderr": stderr,
                "exit_code": out.status.code().unwrap_or(-1),
            }))
        }
        Err(e) => make_err(format!("Failed to run PowerShell: {e}")),
    }
}

#[cfg(not(feature = "diag"))]
"diag_ps" => {
    make_err("diag_ps is disabled in production builds".to_string())
}
```

**Step 3:** Verify no test binaries rely on `diag_ps` without the feature:

```bash
grep -r "diag_ps" src-tauri/tests/
# If any test uses it, add #[cfg(feature = "diag")] to the test or enable the feature in test builds
```

#### Acceptance Criteria

- [ ] `diag` feature added to `Cargo.toml` `[features]` section
- [ ] `diag_ps` match arm gated behind `#[cfg(feature = "diag")]`
- [ ] Non-diag build returns error message for `diag_ps`
- [ ] `cargo build` (without features) does NOT include `diag_ps` PowerShell execution code
- [ ] `cargo build --features diag` DOES include `diag_ps`
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes

---

### S36-004: Replace `USERNAME` env var with `GetUserNameW` in `restrict_file_acl` (Bug B)

| Field         | Value                                                                |
| ------------- | -------------------------------------------------------------------- |
| **Ticket ID** | S36-004                                                              |
| **Title**     | Use `GetUserNameW` Win32 API instead of spoofable `USERNAME` env var |
| **Priority**  | P2 — Medium                                                          |
| **Source**    | Bug B (`Audit_Report_miControl.md`)                                  |
| **Files**     | `src-tauri/src/util/auth.rs` (line 272)                              |
| **Effort**    | ~30 minutes                                                          |
| **Type**      | Backend (Rust)                                                       |

#### Problem

`restrict_file_acl` uses `std::env::var("USERNAME")` to get the current username for building the ACL. The `USERNAME` environment variable is spoofable — any process can set `USERNAME=Administrator` before launching `micontrol.exe`, and the ACL would be built for the wrong user.

The practical impact is limited because:

- `%LOCALAPPDATA%\MiControl` is already per-user (directory ACL inherits from user profile)
- The elevated helper runs as the same user (InteractiveToken, not SYSTEM)

But defense in depth dictates using the process token, not an env var.

#### Current Code

**`auth.rs` line 272:**

```rust
let username = std::env::var("USERNAME").map_err(|e| format!("Cannot get USERNAME: {e}"))?;
```

#### Solution

Replace with `GetUserNameW` Win32 API, which reads from the process token (cannot be spoofed by env var manipulation):

```rust
use windows::Win32::System::WindowsProgramming::GetUserNameW;

let mut buf = [0u16; 256];
let mut len = buf.len() as u32;
unsafe {
    GetUserNameW(&mut buf, &mut len)
        .map_err(|e| format!("GetUserNameW failed: {e}"))?;
}
// len includes the null terminator
let username = String::from_utf16_lossy(&buf[..len as usize - 1])
    .trim_end_matches('\0')
    .to_string();
```

**Alternative (more robust):** Use `GetTokenInformation(TokenUser)` to get the SID directly and build the trustee from the SID via `BuildTrusteeWithSidW`. This is immune to username spoofing entirely and doesn't depend on username string matching. This is a larger change but is the correct fix. If time permits, implement this instead.

#### Acceptance Criteria

- [ ] `std::env::var("USERNAME")` replaced with `GetUserNameW` (or SID-based approach)
- [ ] ACL is built for the actual process user, not a spoofable env var
- [ ] `restrict_file_acl` still works correctly (files are accessible by the current user)
- [ ] `cargo check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes

---

## Story Points

| Ticket    | Points | Owner   | Wave                                       |
| --------- | ------ | ------- | ------------------------------------------ |
| S36-001   | 1      | Backend | 1 (elevated.rs — independent)              |
| S36-002   | 1      | Backend | 1 (elevated.rs — independent)              |
| S36-003   | 1      | Backend | 1 (elevated.rs + Cargo.toml — independent) |
| S36-004   | 2      | Backend | 1 (auth.rs — independent)                  |
| **Total** | **5**  |         |                                            |

## Dependency Map

```
Wave 1 (all parallel — 4 independent tickets):
  S36-001: src-tauri/src/elevated.rs (atomic result write)
  S36-002: src-tauri/src/elevated.rs (directory ACL)
  S36-003: src-tauri/src/elevated.rs + Cargo.toml (diag_ps gate)
  S36-004: src-tauri/src/util/auth.rs (GetUserNameW)
```

**Note:** S36-001, S36-002, and S36-003 all modify `elevated.rs`. They should be committed sequentially to avoid merge conflicts. S36-004 modifies `auth.rs` and is fully independent.

## Commit Strategy

One commit per ticket:

1. `fix(s36-001): make elevated result file write atomic using temp+rename`
2. `fix(s36-002): apply restrict_file_acl to elev_dir directory`
3. `security(s36-003): gate diag_ps behind diag cargo feature flag`
4. `security(s36-004): replace spoofable USERNAME env var with GetUserNameW`

## What Was Deferred

| Ticket                             | Reason                                              | Next Action                                   |
| ---------------------------------- | --------------------------------------------------- | --------------------------------------------- |
| SID-based trustee (S36-004 alt)    | More robust but larger change                       | Future sprint if GetUserNameW is insufficient |
| Symlink hardening for elev_key.bin | Requires FILE_CREATE + FILE_FLAG_OPEN_REPARSE_POINT | Future sprint                                 |

---

## Sprint Completion Checklist

After all tickets are committed:

- [ ] All 4 tickets have passing health checks (9/9)
- [ ] All commits pushed to `main`
- [ ] `sprint-overview.md` updated with Sprint 36 status
- [ ] Manual test: Result file is written atomically (no partial JSON on kill)
- [ ] Manual test: `elev_dir()` directory has restricted ACL (verified via `icacls`)
- [ ] Manual test: `diag_ps` returns error in production build (without `--features diag`)
- [ ] Manual test: `restrict_file_acl` works correctly with `GetUserNameW`
