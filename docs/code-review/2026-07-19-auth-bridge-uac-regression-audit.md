# Code Review: miControl Auth Bridge — UAC Regression & Security Audit

**Date:** 2026-07-19
**Component:** Auth Bridge (Main ↔ Elevated helper IPC)
**Scope:** UAC prompt regression + full security review of the HMAC-based elevation protocol
**Ready for Production:** ⛔ **NO** — critical UAC regression must be fixed first
**Critical Issues:** 3

---

## Executive Summary

The user reports that **every configuration change now triggers a UAC prompt**. This is a regression — the Auth Bridge is designed to elevate silently via the `MiControlElevated` scheduled task (registered at install time with `RunLevel=HighestAvailable`), so no UAC prompt should ever appear during normal operation.

After a thorough review of `elev_bridge.rs`, `elevated.rs`, `util/auth.rs`, `installer-hooks.nsi`, and the stability reports, I identified **three critical issues** that together fully explain the regression, plus several protocol-level weaknesses that should be hardened.

### Root Cause Summary

The UAC prompt is triggered by **two independent bugs** that reinforce each other:

1. **CRITICAL-1 — `ensure_task_correct_path()` return-value semantic bug** (`elev_bridge.rs:460-540`): The function returns `false` both when healing _failed_ AND when healing was _not needed_ (path already correct). The caller treats `false` as "healing failed" and falls back to UAC. **This means: even when the scheduled task is perfectly healthy, any transient `schtasks /run` failure (e.g. the task is briefly in `Running` state from a previous call, or a 1ms race) triggers a UAC prompt.**

2. **CRITICAL-2 — 15-second timeout is too short for cold-start hardware operations** (`elev_bridge.rs:30, 209`): `ELEV_TIMEOUT_SECS = 15` is insufficient for commands like `run_hardware_discovery` (which does WMI + IOCTL probes) or `install_driver` (which runs `pnputil`). When the elevated helper takes >15s, the bridge falls back to UAC — even though the scheduled task is working correctly and will eventually write the result.

3. **CRITICAL-3 — `MultipleInstancesPolicy=StopExisting` kills in-flight tasks** (`installer-hooks.nsi:103`, `elev_bridge.rs:525`): When a second elevated call arrives while the first is still running, `StopExisting` terminates the first task instance. The first call's result file is never written, so it times out → UAC fallback. The `ELEV_REQUEST_LOCK` serializes calls _within_ the main process, but does NOT prevent the scheduled task from being re-triggered by a _different_ MiControl instance (tray popup + main window), nor does it protect against the task scheduler's own queuing behavior.

These three combine into the observed symptom: **the user clicks anything → `schtasks /run` either fails transiently or the helper takes >15s → UAC fallback fires → prompt appears.**

---

## CRITICAL Findings ⛔

### CRITICAL-1: `ensure_task_correct_path()` return-value semantic bug causes spurious UAC fallback

**File:** `src-tauri/src/elev_bridge.rs:460-540` (function) and `src-tauri/src/elev_bridge.rs:115-130` (caller)
**OWASP:** A01 Broken Access Control (unnecessary privilege escalation)
**CWE-396: Declaration of Catch for Generic Exception / CWE-636: Not Failing Securely**

#### Description

`ensure_task_correct_path()` has **three** return paths, but only two return values:

```rust
fn ensure_task_correct_path() -> bool {
    // ...
    let need_reregister = match output {
        Ok(out) => {
            let xml = String::from_utf8_lossy(&out.stdout);
            let path_matches = xml.contains(&current_path) || /* ... */;
            if path_matches {
                false   // ← (A) path already correct, NO healing needed
            } else {
                true
            }
        }
        Err(_) => {
            true    // ← (B) task not found, healing needed
        }
    };

    if !need_reregister {
        return false;   // ← returns FALSE for "already correct"
    }

    // ... attempt healing ...
    success  // ← (C) returns TRUE if healing succeeded, FALSE if it failed
}
```

The caller interprets the return value as:

```rust
let healed = tokio::task::spawn_blocking(ensure_task_correct_path).await...?;

if healed {
    // Retry the task after healing.
    let retry_ok = run_schtasks_run().await;
    if retry_ok {
        // fall through to polling
    } else {
        launch_uac_fallback(...).await?;   // ← UAC!
    }
} else {
    // Healing failed (not admin or other error) — fall back to UAC.
    launch_uac_fallback(...).await?;       // ← UAC!
}
```

**The bug:** When the task is _already correctly registered_ (case A), `ensure_task_correct_path()` returns `false`. The caller interprets this `false` as "healing failed" and calls `launch_uac_fallback()` → **UAC prompt**.

This happens on **every** call where `run_schtasks_run()` returns `false` for any transient reason:

- The task is in `Running` state from a previous invocation (scheduler returns `ERROR_TASK_ALREADY_RUNNING`)
- A brief scheduler service hiccup
- The task was disabled by a group policy or by the user via Task Scheduler GUI
- Antivirus interference with `schtasks.exe`

#### Evidence

```rust
// elev_bridge.rs:460-475 — the ambiguous return
if !need_reregister {
    return false;   // ← "no healing needed" but caller reads as "healing failed"
}
```

```rust
// elev_bridge.rs:120-130 — caller treats false as failure
} else {
    // Healing failed (not admin or other error) — fall back to UAC.
    launch_uac_fallback(&request_id, &cmd_path).await?;
}
```

#### Recommended Fix

Change `ensure_task_correct_path()` to return a tri-state, or distinguish "already correct" from "healing failed":

```rust
enum TaskHealResult {
    AlreadyCorrect,   // task exists and points to the right exe — no action needed
    Healed,            // task was re-registered successfully
    Failed(String),    // healing attempted and failed
}

fn ensure_task_correct_path() -> TaskHealResult {
    // ...
    if !need_reregister {
        return TaskHealResult::AlreadyCorrect;
    }
    // ... healing logic ...
    if success { TaskHealResult::Healed } else { TaskHealResult::Failed(...) }
}
```

And in the caller:

```rust
match ensure_task_correct_path() {
    TaskHealResult::AlreadyCorrect => {
        // Task is fine — the /run failure was transient. Retry once,
        // then if still failing, surface a real error instead of UAC.
        let retry_ok = run_schtasks_run().await;
        if !retry_ok {
            return Err(format!(
                "Scheduled task '{}' is healthy but failed to start. \
                 Check Task Scheduler for errors.",
                TASK_NAME
            ));
        }
    }
    TaskHealResult::Healed => {
        let retry_ok = run_schtasks_run().await;
        if !retry_ok {
            return Err("Task re-registered but failed to start".to_string());
        }
    }
    TaskHealResult::Failed(reason) => {
        // Only here is UAC fallback appropriate — and only if the user
        // explicitly consents, not silently on every config change.
        log::warn!("Task healing failed: {reason}");
        return Err(format!(
            "Scheduled task unavailable: {reason}. \
             Reinstall MiControl or run as administrator."
        ));
    }
}
```

**Do NOT silently fall back to UAC.** The UAC fallback was intended as a dev-mode convenience (per the module doc comment: "only a dev ergonomics aid; production always uses the scheduled task"). In production, a failed scheduled task should produce a clear error, not a silent privilege escalation prompt.

---

### CRITICAL-2: 15-second timeout is too short → spurious UAC fallback on slow hardware ops

**File:** `src-tauri/src/elev_bridge.rs:30` (`ELEV_TIMEOUT_SECS = 15`), `src-tauri/src/elev_bridge.rs:209-260` (timeout fallback)
**CWE-697: Incorrect Comparison**

#### Description

The polling timeout is 15 seconds. Several elevated commands legitimately take longer:

| Command                       | Why it's slow                                         | Typical duration |
| ----------------------------- | ----------------------------------------------------- | ---------------- |
| `run_hardware_discovery`      | WMI queries + IOCTL probes + driver enumeration       | 10-30s           |
| `install_driver`              | `pnputil /add-driver /install` + driver store staging | 15-60s           |
| `set_performance_mode` (cold) | WMI method call + VHF device handshake on first call  | 5-20s            |
| `diag_wmi_query`              | 4 sequential WMI queries (HQ, MI, Esif, Battery)      | 5-15s            |

When the elevated helper exceeds 15s, the bridge:

1. Re-writes the command file (line 213)
2. Launches `launch_elevated_via_uac()` → **UAC prompt** (line 219)
3. The UAC-launched helper runs `--elevated --request-id <id>` and writes the result
4. Meanwhile, the _original_ scheduled-task helper is still running and will ALSO write a result file when it finishes

This causes **two elevated processes** running the same command, plus a UAC prompt.

#### Evidence

```rust
// elev_bridge.rs:28-30
const POLL_INTERVAL_MS: u64 = 150;
const ELEV_TIMEOUT_SECS: u64 = 15;   // ← too short for install_driver / discovery
```

```rust
// elev_bridge.rs:209-220 — timeout triggers UAC
if start.elapsed() > timeout {
    #[cfg(windows)]
    {
        let _ = tokio::fs::write(&cmd_path, payload.to_string()).await;
        let req_id_owned = request_id.clone();
        let uac_result = tokio::task::spawn_blocking(move || {
            launch_elevated_via_uac(&req_id_owned)   // ← UAC PROMPT
        }).await...?;
```

#### Recommended Fix

1. **Increase the timeout to 60 seconds** (matches the task's `ExecutionTimeLimit=PT30S` plus margin):

   ```rust
   const ELEV_TIMEOUT_SECS: u64 = 60;
   ```

   Better: make it per-command, since `install_driver` and `run_hardware_discovery` need more than `set_brightness`:

   ```rust
   fn timeout_for(cmd: &str) -> Duration {
       match cmd {
           "install_driver" | "run_hardware_discovery" => Duration::from_secs(120),
           "diag_wmi_query" | "diag_ecram_read" => Duration::from_secs(45),
           _ => Duration::from_secs(30),
       }
   }
   ```

2. **Do not fall back to UAC on timeout.** A timeout means the helper is still running (or crashed). The correct response is to return an error and let the user retry — NOT to launch a second elevated process via UAC. The UAC fallback on timeout is the single biggest source of spurious prompts.

3. **Detect helper crashes** by checking if the task is still in `Running` state via `schtasks /query /tn MiControlElevated /fo LIST /v` before declaring timeout.

---

### CRITICAL-3: `MultipleInstancesPolicy=StopExisting` kills in-flight elevated helpers

**File:** `src-tauri/nssis/installer-hooks.nsi:103`, `src-tauri/src/elev_bridge.rs:525` (heal XML)
**CWE-362: Race Condition**

#### Description

The scheduled task is registered with `MultipleInstancesPolicy=StopExisting`:

```xml
<MultipleInstancesPolicy>StopExisting</MultipleInstancesPolicy>
```

This means: if `schtasks /run` is called while a previous task instance is still running, **the scheduler stops the running instance** and starts a new one.

The `ELEV_REQUEST_LOCK` in `elev_bridge.rs:33` serializes calls _within one process_:

```rust
static ELEV_REQUEST_LOCK: Mutex<()> = Mutex::const_new(());
// ...
let _guard = ELEV_REQUEST_LOCK.lock().await;
```

But this lock does NOT protect against:

- **Two MiControl processes** (the main window + the tray quick-access popup can both be running — `tauri-plugin-single-instance` only deduplicates _full_ launches, not the tray popup which runs in the same process... but if the user has the app open and clicks a tray action that triggers `run_elevated`, the lock IS held. However, if a previous elevated call's helper is still running when the lock is released and a new call begins, the new `schtasks /run` kills the old helper.)
- **The lock is released as soon as `run_elevated` returns**, but the elevated helper may still be writing its result file. If the user clicks again quickly, the new `schtasks /run` arrives before the old helper exits → `StopExisting` kills the old helper → its result file is never written → the old call already returned `Ok` but the _new_ call's command file gets consumed by the dying old helper.

Wait — re-reading: the lock is held for the _entire_ duration of `run_elevated` (including polling), so within one process, calls are serialized end-to-end. The real problem is subtler:

- The elevated helper reads the command file, **deletes it immediately** (`elevated.rs:62`), then runs the command. If the command takes 10s and the user triggers another action, the lock prevents a second `run_elevated` from starting. Good.
- BUT: if the elevated helper crashes or is killed by `StopExisting` from a _different_ trigger (e.g. an external `schtasks /run`, or a second MiControl instance that bypasses the lock), the result file is never written → 15s timeout → UAC fallback.

The deeper issue: **`StopExisting` is the wrong policy for a request-response protocol.** It should be `Queue` or `IgnoreNew`, and the protocol should rely on the `ELEV_REQUEST_LOCK` + request-id matching to prevent mixups.

#### Evidence

```xml
<!-- installer-hooks.nsi:103 -->
<MultipleInstancesPolicy>StopExisting</MultipleInstancesPolicy>
```

```rust
// elev_bridge.rs:33 — lock only protects within one process
static ELEV_REQUEST_LOCK: Mutex<()> = Mutex::const_new(());
```

```rust
// elevated.rs:62 — helper deletes command file immediately, then runs
let _ = std::fs::remove_file(&pending.cmd_path);
```

#### Recommended Fix

1. **Change `MultipleInstancesPolicy` to `IgnoreNew`** in both the installer XML and the heal XML. This prevents a second `schtasks /run` from killing an in-flight helper. The `ELEV_REQUEST_LOCK` already serializes calls within the app; `IgnoreNew` makes the scheduler consistent with that.

   ```xml
   <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
   ```

2. **Increase `ExecutionTimeLimit`** from `PT30S` to `PT2M` to match the new polling timeout (CRITICAL-2 fix). If the helper exceeds `ExecutionTimeLimit`, the scheduler kills it — which again causes a missing result file.

3. **Add a per-request lock file** (e.g. `elev_lock_<request_id>`) that the helper holds while running, so the main process can detect "helper died" vs "helper still running".

---

## HIGH Findings 🔴

### HIGH-1: `is_admin()` fast path bypasses HMAC authentication for all elevated commands

**File:** `src-tauri/src/elev_bridge.rs:45-60`, `src-tauri/src/elevated.rs:1301-1315`
**OWASP:** A01 Broken Access Control
**CWE-862: Missing Authorization**

#### Description

When the main process is already running as administrator, `run_elevated()` dispatches the command directly in-process via `dispatch_cmd()`, **bypassing all HMAC verification, nonce anti-replay, and timestamp freshness checks**:

```rust
// elev_bridge.rs:45-60
#[cfg(windows)]
if is_admin() {
    let args2 = args.clone();
    return tokio::task::spawn_blocking(move || {
        let result = crate::elevated::dispatch_cmd(cmd, args2);
        // ...
    }).await...?;
}
```

```rust
// elevated.rs:1301-1315 — dispatch_cmd skips all auth
pub fn dispatch_cmd(cmd: &str, args: Value) -> Value {
    dispatch(ElevCmd {
        _protocol_version: None,
        _request_id: None,
        _created_at_ms: None,
        nonce: None,        // ← no nonce
        _hmac: None,        // ← no HMAC
        _caller_pid: None,
        cmd: cmd.to_string(),
        args,
    })
}
```

The stability report v2 acknowledges this: _"`dispatch_cmd` bypasses HMAC when already elevated — by design (attacker needs admin)."_ This is a **defense-in-depth failure**, not a direct escalation. However:

1. The `is_admin()` check uses `IsUserAnAdmin()`, which returns `true` if the process token has the Administrators group **enabled** — this includes processes running with `requireAdministrator` manifest, but ALSO includes processes where UAC was elevated once and the token is still elevated.
2. If a vulnerability in the webview (CSP bypass, XSS via a misconfigured `connect-src`) allows arbitrary `invoke()` calls, and the app happens to be running elevated, the attacker can call _any_ elevated command with no HMAC, no nonce, no timestamp — including `diag_ps` (arbitrary PowerShell execution), `install_driver`, and `set_scancode_map` (registry write to `HKLM\...\Keyboard Layout`).
3. The `diag_ps` command is especially dangerous: it runs **arbitrary PowerShell scripts** passed from the frontend. Even in the non-fast-path, the HMAC only verifies the _caller knows the key_ — it does not restrict _which_ commands can be called. A compromised frontend can call `diag_ps` with any script.

#### Evidence

```rust
// elevated.rs — diag_ps runs arbitrary PowerShell
"diag_ps" => {
    let script = cmd.args["script"as_str().unwrap_or("");
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output();
```

#### Recommended Fix

1. **Remove `diag_ps` from production builds** or gate it behind a `#[cfg(debug_assertions)]` / `MICONTROL_ENABLE_DIAG` env var. Arbitrary PowerShell execution from an IPC command is a critical attack surface.

2. **Apply HMAC verification even in the fast path.** The fast path should still verify that the caller is the legitimate main process, not a injected thread. At minimum, validate `cmd` against an allowlist of commands safe for in-process dispatch.

3. **Add a command allowlist** to `dispatch_cmd()` — even when elevated, only whitelisted commands should be executable without HMAC (e.g. `set_brightness`, `set_performance_mode`). `diag_ps`, `install_driver`, `set_scancode_map` should always require the full HMAC protocol.

---

### HIGH-2: Result file write is non-atomic — TOCTOU race on result consumption

**File:** `src-tauri/src/elevated.rs:148`
**CWE-367: Time-of-check Time-of-use (TOCTOU) Race Condition**

#### Description

The elevated helper writes the result file non-atomically:

```rust
// elevated.rs:148
let _ = std::fs::write(&pending.result_path, json);
```

The main process polls for `result_path.exists()` then reads it:

```rust
// elev_bridge.rs:152-156
if result_path.exists() {
    let content = tokio::fs::read_to_string(&result_path).await...?;
    let _ = tokio::fs::remove_file(&result_path).await;
```

**Race:** The main process can see the file exists (via `exists()`) while the elevated helper is still writing it. The main process then reads a partial JSON → `serde_json::from_str` fails → `Invalid result JSON` error. The command file is already deleted by the helper (`elevated.rs:62`), so the operation is lost.

The command file write IS atomic (temp + rename, `elev_bridge.rs:101-104`), but the result file write is NOT. This asymmetry is a bug.

#### Evidence

```rust
// elevated.rs:148 — non-atomic write
let _ = std::fs::write(&pending.result_path, json);
```

```rust
// elev_bridge.rs:101-104 — command file IS atomic
let tmp_path = dir.join(format!("elev_cmd_{request_id}.tmp"));
tokio::fs::write(&tmp_path, payload.to_string()).await...?;
tokio::fs::rename(&tmp_path, &cmd_path).await...?;
```

#### Recommended Fix

Make the result file write atomic, matching the command file pattern:

```rust
// elevated.rs — replace the direct write
let tmp_path = pending.result_path.with_extension("json.tmp");
if std::fs::write(&tmp_path, &json).is_ok() {
    if std::fs::rename(&tmp_path, &pending.result_path).is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    } else {
        let _ = auth::restrict_file_acl(&pending.result_path);
    }
}
```

---

### HIGH-3: `restrict_file_acl` uses spoofable `USERNAME` environment variable

**File:** `src-tauri/src/util/auth.rs:265`
**CWE-348: Use of Less Trusted Source**
**Status:** RESIDUAL (noted in STABILITY_REPORT_v4 SEC-008, not yet fixed)

#### Description

The ACL restriction grants access to the current user based on `std::env::var("USERNAME")`:

```rust
// auth.rs:265
let username = std::env::var("USERNAME").map_err(|e| format!("Cannot get USERNAME: {e}"))?;
```

A parent process can override the `USERNAME` environment variable before launching MiControl. If `USERNAME` is set to a different user (e.g. `Administrator` or `SYSTEM`), the ACL grants full access to that _other_ account, while the actual running user may have reduced access — or worse, if `USERNAME` is set to an attacker-controlled account, that account gets read access to `elev_key.bin` (the HMAC signing key).

The v4 report classifies this as LOW ("worst case is DoS"), but I disagree: if an attacker can read `elev_key.bin`, they can forge valid HMAC-signed commands and inject arbitrary privileged operations via the file-based IPC. This is **privilege escalation**, not just DoS.

#### Evidence

```rust
// auth.rs:265
let username = std::env::var("USERNAME").map_err(|e| format!("Cannot get USERNAME: {e}"))?;
```

#### Recommended Fix

Use `GetUserNameW` Win32 API, which reads from the process token (not the environment):

```rust
use windows::Win32::System::WindowsProgramming::GetUserNameW;

fn current_username() -> Result<String, String> {
    let mut size = 0u32;
    unsafe { GetUserNameW(None, &mut size); }
    let mut buf = vec![0u16; size as usize];
    unsafe {
        GetUserNameW(Some(windows::core::PWSTR(buf.as_mut_ptr())), &mut size)
            .map_err(|e| format!("GetUserNameW: {e}"))?;
    }
    Ok(String::from_utf16_lossy(&buf[..size as usize]))
}
```

Better: use the SID from the process token (`GetTokenInformation` + `TokenUser`) and build the trustee from the SID. This is immune to both `USERNAME` spoofing and username collision attacks.

---

### HIGH-4: `elev_dir()` has no directory-level ACL — only individual files are restricted

**File:** `src-tauri/src/elevated.rs:1317-1323`
**CWE-732: Incorrect Permission Assignment for Critical Resource**

#### Description

`elev_dir()` creates `%LOCALAPPDATA%\MiControl` with default permissions (inherited from `%LOCALAPPDATA%`, which is typically user-private). However, the code only restricts ACLs on _individual files_ (`elev_key.bin`, `nonces.json`, command/result files) — NOT on the directory itself.

This creates several risks:

1. **Symlink injection:** A non-admin process (or another user on the system) can create a symlink at `%LOCALAPPDATA%\MiControl\elev_cmd_<predicted_request_id>.json` pointing to an arbitrary file BEFORE the main process writes it. The main process's `tokio::fs::rename` would then follow the symlink (on Windows, `rename` to an existing symlink replaces the target). The elevated helper would read the attacker-controlled content. The HMAC would fail UNLESS the attacker knows the key — but the request_id format is predictable (`{pid:08x}-{ms:016x}-{seq:08x}`), enabling a pre-placement attack.

2. **File enumeration:** Any process running as the same user can `readdir()` the elev directory and see command/result filenames, timing, and sizes — a metadata leak.

3. **Result file deletion:** A malicious process can delete `elev_result_<id>.json` as soon as it appears, causing the main process to time out → UAC fallback (amplifying CRITICAL-1/2).

#### Evidence

```rust
// elevated.rs:1317-1323 — directory created with default ACL
pub fn elev_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")...;
    let dir = PathBuf::from(base).join("MiControl");
    let _ = std::fs::create_dir_all(&dir);   // ← no ACL restriction
    dir
}
```

#### Recommended Fix

Restrict the directory ACL once at creation time:

```rust
pub fn elev_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| {
        std::env::temp_dir().to_string_lossy().into_owned()
    });
    let dir = PathBuf::from(base).join("MiControl");
    if !dir.exists() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = auth::restrict_file_acl(&dir);  // ← restrict directory ACL
    }
    dir
}
```

Additionally, **reject symlinks** when reading command/result files:

```rust
// In elevated.rs, before reading the command file:
let meta = std::fs::symlink_metadata(&pending.cmd_path)?;
if meta.file_type().is_symlink() {
    log::warn!("Refusing to read symlinked command file: {}", pending.cmd_path.display());
    return make_err("Command file is a symlink — possible attack".to_string());
}
```

---

## MEDIUM Findings 🟡

### MEDIUM-1: HMAC verification is not fully constant-time on the timestamp path

**File:** `src-tauri/src/util/auth.rs:200-230`
**CWE-208: Observable Timing Discrepancy**

#### Description

`verify_hmac()` correctly uses a constant-time XOR comparison (line 175-185). However, `verify_payload()` has an early-exit on the timestamp check that leaks timing information:

```rust
// auth.rs:218-228
let ts = payload.get("created_at_ms").and_then(|v| v.as_u64())
    .ok_or_else(|| "Missing required created_at_ms field".to_string())?;
if !is_timestamp_fresh(ts) {
    return Err(format!("Command timestamp {ts} is stale ..."));
}
```

An attacker who can measure response timing can distinguish "HMAC valid but timestamp stale" from "HMAC invalid" — this is a minor oracle but could help in brute-forcing the key. The HMAC comparison itself is constant-time, but the _order_ of checks (HMAC first, then timestamp) means a timing difference exists between "wrong HMAC" and "right HMAC + wrong timestamp".

#### Recommended Fix

This is acceptable for the current threat model (local IPC, attacker cannot easily measure sub-microsecond timing across process boundaries). Document it as a known limitation. If you want to harden: always perform both checks and combine the errors with a constant-time `OR`.

---

### MEDIUM-2: Nonce store grows unbounded between batch saves

**File:** `src-tauri/src/elevated.rs:137-183`
**CWE-770: Allocation of Resources Without Limits**
**Status:** RESIDUAL (STABILITY_REPORT_v4 SEC-009)

#### Description

Nonces are persisted every 3rd insertion (`if map.len().is_multiple_of(3)`). The in-memory `HashMap` grows between saves. The 5-minute TTL purges on `load_nonces()` (next helper invocation), but within a single helper invocation (which handles ONE command then exits), the map only grows by 1 entry. The real risk is `nonces.json` growing over time if the purge on load fails.

#### Recommended Fix

Add a hard cap (e.g. 10,000 entries) and purge oldest entries when exceeded. Also purge on every load, not just when the file is read successfully.

---

### MEDIUM-3: `ensure_task_correct_path` heal XML embeds the exe path with embedded quotes — XML injection if path contains `"`

**File:** `src-tauri/src/elev_bridge.rs:518-522`
**CWE-79: Cross-site Scripting (XML variant)**

#### Description

The heal XML is built with `format!` and embeds `current_path` directly into the `<Command>` element with surrounding quotes:

```rust
let xml = format!(
    r#"...<Command>"{current_path}"</Command>..."#
);
```

If `current_path` contains a `"` character (unlikely for a standard install path, but possible if the user installed to a path with quotes), the XML breaks. More realistically, if the path contains `<` or `&` (valid in Windows paths), the XML is malformed and `schtasks /create /xml` fails silently → healing fails → UAC fallback.

The installer XML (`installer-hooks.nsi:111`) has the same issue with `$INSTDIR`.

#### Recommended Fix

XML-escape the path before embedding:

```rust
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

let xml = format!(
    r#"...<Command>{}</Command>..."#,
    xml_escape(&current_path)
);
```

Note: the surrounding quotes in `<Command>"{path}"</Command>` are actually unnecessary and can cause `schtasks` to fail on some Windows versions. The Task Scheduler schema expects the path as element text, not a quoted string. Test whether removing the quotes fixes task registration.

---

### MEDIUM-4: `key_needs_rotation()` returns `false` if the key file doesn't exist — silent skip of rotation

**File:** `src-tauri/src/util/auth.rs:355-368`
**CWE-754: Improper Check for Unusual or Exceptional Conditions**

#### Description

```rust
pub fn key_needs_rotation() -> bool {
    let path = key_path();
    match std::fs::metadata(&path) {
        Ok(meta) => { /* check age */ }
        Err(_) => false, // No key file yet — will be created by get_or_create_key
    }
}
```

If the key file is missing (e.g. deleted by an attacker, or by a failed `restrict_file_acl` in `get_or_create_key`), `key_needs_rotation()` returns `false` — so rotation is silently skipped. The next `run_elevated` call will call `get_or_create_key()` which creates a new key. This is not a direct vulnerability, but it means the 30-day rotation guarantee is not enforced if the key file is tampered with.

#### Recommended Fix

Return `true` if the metadata check fails (treat missing key as "needs rotation"), or log a warning.

---

### MEDIUM-5: UAC fallback on timeout re-writes the command file without re-signing

**File:** `src-tauri/src/elev_bridge.rs:213`
**CWE-345: Insufficient Verification of Data Authenticity**

#### Description

```rust
// elev_bridge.rs:211-213
// Re-write the command file in case the bad task process consumed or deleted it.
let _ = tokio::fs::write(&cmd_path, payload.to_string()).await;
```

`payload` still contains the original HMAC, but the `created_at_ms` timestamp is now up to 15 seconds older. If the UAC-launched helper takes another 20 seconds to start (UAC prompt + user delay + process init), the total age could exceed the 30-second `MAX_COMMAND_AGE_MS` window → the elevated helper rejects the command as stale → error returned to user.

#### Recommended Fix

Re-sign the payload with a fresh timestamp before re-writing:

```rust
payload["created_at_ms"] = json!(auth::now_ms());
auth::sign_payload(&mut payload, &key);
let _ = tokio::fs::write(&cmd_path, payload.to_string()).await;
```

Better: remove the timeout UAC fallback entirely (per CRITICAL-2).

---

### MEDIUM-6: `relaunch_self_as_admin()` is dead code — no caller in the codebase

**File:** `src-tauri/src/elev_bridge.rs:364-401`
**CWE-1164: Irrelevant Code**

#### Description

`relaunch_self_as_admin()` is `pub` but never called anywhere in the workspace (verified via grep). It was likely intended for a "restart as admin" UI flow but was never wired up. Dead code in a security-sensitive module increases the attack surface and maintenance burden.

#### Recommended Fix

Either wire it up to a user-facing "Restart as Administrator" button (with explicit consent), or remove it. If kept, add a `#[deprecated]` note and a test that verifies it's only callable from a user-initiated action.

---

## LOW Findings 🟢

### LOW-1: `MAX_COMMAND_AGE_MS` is 30s but `is_timestamp_fresh` allows ±30s skew — total window is 60s

**File:** `src-tauri/src/util/auth.rs:13, 155-160`

```rust
pub const MAX_COMMAND_AGE_MS: u64 = 30_000;

pub fn is_timestamp_fresh(timestamp_ms: u64) -> bool {
    let now = now_ms();
    now >= timestamp_ms.saturating_sub(MAX_COMMAND_AGE_MS)      // 30s in the past
        && now <= timestamp_ms.saturating_add(MAX_COMMAND_AGE_MS)  // 30s in the future
}
```

The freshness window is 60 seconds total (30s past + 30s future). The future window is for clock skew, but 30s of future skew is excessive — an attacker who can set the system clock forward can extend command validity. Reduce the future window to 5s.

---

### LOW-2: `nonce_store_path()` and `key_path()` use `elev_dir()` which falls back to `%TEMP%`

**File:** `src-tauri/src/elevated.rs:1317-1323`, `src-tauri/src/util/auth.rs:20-23`

If `LOCALAPPDATA` is unset (rare, but possible in service contexts or sandboxed environments), `elev_dir()` falls back to `std::env::temp_dir()` — which is typically `C:\Windows\Temp` for SYSTEM or `C:\Users\...\AppData\Local\Temp` for users. `%TEMP%` is world-readable in some configurations. The fallback should fail-closed instead.

---

### LOW-3: `get_or_create_key()` holds an exclusive lock for up to 5 seconds during key generation

**File:** `src-tauri/src/util/auth.rs:42-90`

The 5-second polling loop with `sleep(50ms)` blocks a `spawn_blocking` thread. This is acceptable but could be improved with a condition variable or `notify_one`.

---

### LOW-4: Installer does not verify the scheduled task was created successfully

**File:** `src-tauri/nssis/installer-hooks.nsi:118-122`

```nsis
nsExec::ExecToLog '"$SYSDIR\schtasks.exe" /create /tn "MiControlElevated" /xml "$TEMP\MCElev.xml" /f'
Pop $0
Delete "$TEMP\MCElev.xml"
DetailPrint "MiControlElevated task registered: $0"
```

The installer logs the exit code but does not abort or warn the user if task creation fails. If the installer is run without elevation (UAC declined for the installer itself), `schtasks /create` fails silently, and the app's self-healing (which has the CRITICAL-1 bug) takes over — triggering UAC on every use.

#### Recommended Fix

Check `$0` and show a warning page if task creation fails:

```nsis
${If} $0 != 0
  MessageBox MB_ICONEXCLAMATION "Failed to register the MiControlElevated scheduled task ($0). UAC prompts will appear on each hardware change. Please reinstall as administrator."
${EndIf}
```

---

### LOW-5: `rotate_key()` does not restrict the ACL on the `.old` backup key

**File:** `src-tauri/src/util/auth.rs:383-401`

```rust
pub fn rotate_key() -> Result<(), String> {
    // ...
    std::fs::copy(&path, &old_path)  // ← old key backup, no ACL restriction
        .map_err(|e| format!("Failed to backup old HMAC key: {e}"))?;
    // ...
    restrict_file_acl(&path)?;  // ← only new key is restricted
    // ...
}
```

The `.old` key file is created with default permissions. If an attacker reads it within the 7-day grace period, they can forge commands signed with the old key (which `verify_payload` still accepts). Add `restrict_file_acl(&old_path)?;` after the copy.

---

### LOW-6: `select_pending_command` picks the newest file by mtime — vulnerable to file-time manipulation

**File:** `src-tauri/src/elevated.rs:1395-1410`

When no `--request-id` is passed (scheduled task path), the helper scans for the newest `elev_cmd_*.json` without a matching result. An attacker who can write to the elev directory (see HIGH-4) can create a command file with a future mtime to ensure their command is selected over the legitimate one. The HMAC would still need to verify, but combined with HIGH-3 (USERNAME spoofing to read the key), this becomes exploitable.

#### Recommended Fix

Prefer the `--request-id` path always. The scheduled task should pass the request ID via argv or an environment variable rather than relying on file scanning.

---

## Positive Security Observations ✅

The following are correctly implemented and should be preserved:

1. ✅ **HMAC-SHA256 with constant-time comparison** — `verify_hmac()` uses `diff |= a ^ b` (auth.rs:175-185)
2. ✅ **Atomic command file writes** — temp file + rename (elev_bridge.rs:101-104)
3. ✅ **Fail-closed key creation** — `get_or_create_key()` deletes the key file if `restrict_file_acl` fails (auth.rs:120-125)
4. ✅ **Exclusive file lock during key generation** — `fs2::try_lock_exclusive` with 5s timeout (auth.rs:62-78)
5. ✅ **Nonce anti-replay with TTL** — 5-minute purge on load (elevated.rs:185-198)
6. ✅ **Nonce flush on exit** — `flush_nonces()` called before `exit(0)` (elevated.rs:48, 151)
7. ✅ **Key rotation with grace period** — 30-day rotation, 7-day old-key acceptance (auth.rs:370-420)
8. ✅ **HMAC-signed responses** — elevated helper signs the result, main process verifies (elevated.rs:153-156, elev_bridge.rs:166-170)
9. ✅ **`CREATE_NO_WINDOW` on schtasks** — no console flash (elev_bridge.rs:139)
10. ✅ **Serialized elevated requests** — `ELEV_REQUEST_LOCK` prevents cross-request mixups within one process (elev_bridge.rs:33)
11. ✅ **CSP in tauri.conf.json** — strict `default-src 'self'` with no `unsafe-inline`
12. ✅ **No `requireAdministrator` manifest** — the app runs as standard user, elevation is on-demand via the task

---

## Remediation Priority

| Priority | Finding                                                     | Effort  | Impact                         |
| -------- | ----------------------------------------------------------- | ------- | ------------------------------ |
| ⛔ P0    | CRITICAL-1: Fix `ensure_task_correct_path` return semantics | Small   | Eliminates most UAC prompts    |
| ⛔ P0    | CRITICAL-2: Increase timeout + remove timeout UAC fallback  | Small   | Eliminates slow-op UAC prompts |
| ⛔ P0    | CRITICAL-3: Change `StopExisting` → `IgnoreNew`             | Small   | Prevents in-flight task kills  |
| 🔴 P1    | HIGH-1: Remove `diag_ps` / add command allowlist            | Medium  | Closes arbitrary code exec     |
| 🔴 P1    | HIGH-2: Atomic result file write                            | Small   | Fixes lost results             |
| 🔴 P1    | HIGH-3: Use `GetUserNameW` instead of `USERNAME` env        | Small   | Closes key theft via env spoof |
| 🔴 P1    | HIGH-4: Restrict `elev_dir` ACL + reject symlinks           | Medium  | Closes symlink injection       |
| 🟡 P2    | MEDIUM-3: XML-escape exe path in heal XML                   | Small   | Fixes healing on odd paths     |
| 🟡 P2    | MEDIUM-5: Re-sign payload on timeout rewrite                | Small   | Fixes stale-command rejection  |
| 🟡 P2    | MEDIUM-6: Remove dead `relaunch_self_as_admin`              | Trivial | Reduces attack surface         |
| 🟢 P3    | LOW-1 through LOW-6                                         | Small   | Defense in depth               |

---

## Verification Steps After Fix

1. **Reinstall the app** (to register the task with `IgnoreNew` policy).
2. **Open Task Scheduler** → confirm `MiControlElevated` exists, points to the installed exe, `RunLevel=Highest`, `LogonType=InteractiveToken`.
3. **Click any config toggle in the app** → NO UAC prompt should appear.
4. **Check `%LOCALAPPDATA%\MiControl\`** → `elev_cmd_*.json` and `elev_result_*.json` should be created and deleted cleanly (no stale files after 120s).
5. **Run `schtasks /query /tn MiControlElevated /fo LIST /v`** → `Last Run Time` should update, `Last Result` should be `0x0`.
6. **Test slow operations** (`run_hardware_discovery`, `install_driver`) → should complete without UAC, even if >15s.
7. **Test rapid clicking** → `IgnoreNew` should prevent task kills; the `ELEV_REQUEST_LOCK` serializes calls.

---

## Conclusion

The UAC regression is caused by **three compounding bugs** in the fallback logic of `elev_bridge.rs`, not by a failure of the core HMAC protocol. The cryptographic foundation is sound. The fixes are small and localized:

1. Fix the `ensure_task_correct_path` return-value semantic (tri-state).
2. Increase the timeout to 60s and remove the timeout-triggered UAC fallback.
3. Change `MultipleInstancesPolicy` from `StopExisting` to `IgnoreNew`.

These three fixes will restore the silent-elevation behavior. The HIGH findings (symlink injection, `USERNAME` spoofing, `diag_ps` arbitrary execution) should be addressed in a follow-up sprint as they represent real privilege-escalation paths that are currently mitigated only by the difficulty of exploiting the file-based IPC locally.
