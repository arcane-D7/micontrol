# Code Review: MiControl — Security Audit
**Date**: 2026-06-19
**Scope**: `micontrol/src-tauri/` (Rust backend) + `micontrol/src/` (React/TS frontend)
**Stack**: Tauri v2.11.1 + React 19 + Rust (windows-rs 0.58, tokio 1.52)
**Ready for Production**: ⚠️ Conditional — fix Priority 1 items before release
**Critical Issues**: 3
**High Issues**: 6
**Medium Issues**: 7
**Low Issues**: 5

---

## Executive Summary

MiControl is a Tauri v2 desktop app that directly manipulates Xiaomi laptop hardware: EC RAM via IOCTL, HID devices, named-pipe IPC to `IoTService.exe`, keyboard/touchpad hooks, and an elevated helper spawned via a Windows Scheduled Task. The threat model is unusual: the app runs **unprivileged** (`asInvoker` manifest) but bridges to an **elevated** helper through a filesystem-based command/response protocol.

The most serious findings cluster around the **elevated-bridge file protocol** (TOCTOU + world-writable command files → arbitrary privileged command execution), **WiFi profile XML injection** (argument injection into `netsh`), and the **`Script` hotkey action** (arbitrary command execution from a user-editable config). Several `unsafe` Rust blocks are well-bounded, but a few lack sufficient validation of driver-returned lengths.

---

## Priority 1 (Must Fix) ⛔

### P1-1. Elevated bridge: world-writable command files enable privilege escalation
**File**: `src-tauri/src/elev_bridge.rs:80-130`, `src-tauri/src/elevated.rs:30-60, 380-460`
**Type**: A01 Broken Access Control / Privilege Escalation (CWE-269, CWE-377)
**Severity**: Critical

**Description**

The elevated bridge writes JSON commands to `%LOCALAPPDATA%\MiControl\elev_cmd_<id>.json` and the elevated helper (running as `HighestAvailable` via the `MiControlElevated` scheduled task) reads and executes them. `%LOCALAPPDATA%` is typically `%USERPROFILE%\AppData\Local`, which is owned by the user but **not protected from other processes running as the same user**.

Any process running as the same user (or a compromised browser/process) can:
1. Pre-create `elev_cmd_<predicted_id>.json` with a malicious payload (e.g. `install_driver` with a crafted `driver_name`, or any command the dispatcher accepts).
2. Wait for the scheduled task to pick it up and execute it with **administrator privileges**.

The `select_pending_command` function (`elevated.rs:415-460`) picks the **newest** `elev_cmd_*.json` by mtime when no `--request-id` is passed (the scheduled-task path). There is **no authentication** of the command file's origin — no caller PID verification against an allow-list, no HMAC/signature, no ACL set on the file. `caller_pid` is written into the payload but **never validated** by the elevated helper.

Additionally, `elev_dir()` creates the directory with default permissions (`std::fs::create_dir_all`), which inherit from the parent — no explicit ACL restricts it to the current user's SID.

**Exploit scenario**: A low-privilege malware process running as the user writes `%LOCALAPPDATA%\MiControl\elev_cmd_attack.json` containing `{"cmd":"install_driver","args":{"driver_name":"../malicious"}}` (or any privileged command), then triggers `schtasks /run /tn MiControlElevated`. The elevated helper executes it with admin rights.

**Recommended fix**

1. **Restrict the directory ACL** to the current user only at creation time using `CreateFileW` with an explicit security descriptor (DACL = current user SID, no Everyone/Users). Example:
   ```rust
   // Use windows::Win32::Security::Authorization::SetNamedSecurityInfoW
   // with DACL = current user + SYSTEM only.
   ```
2. **Sign command files**: include an HMAC-SHA256 of the payload using a key generated per-session and passed to the elevated helper via an environment variable or a secure alternate channel (not the filesystem). Verify in `elevated::dispatch` before executing.
3. **Validate `caller_pid`**: in the elevated helper, open the PID and verify its image path matches the main MiControl executable before executing the command.
4. **Prefer a named pipe with impersonation** over filesystem polling — the pipe can enforce `RpcImpLevel = RPC_C_IMP_LEVEL_IDENTIFY` and verify the client SID.

---

### P1-2. WiFi profile XML injection → command/argument injection into netsh
**File**: `src-tauri/src/hw/wifi.rs:55-110`
**Type**: A03 Injection (CWE-79, CWE-78)
**Severity**: Critical

**Description**

`wifi::connect` builds a WLAN profile XML by string-interpolating `ssid` and `password` directly:

```rust
let profile_xml = format!(
    r#"<?xml version="1.0"?>
<WLANProfile ...>
    <name>{ssid}</name>
    <SSIDConfig><SSID><name>{ssid}</name></SSID></SSIDConfig>
    ...
    <keyMaterial>{pwd}</keyMaterial>
</WLANProfile>"#,
);
```

Neither `ssid` nor `password` is XML-escaped. A malicious SSID like `</name><name>x</name><SSIDConfig>` or one containing `</WLANProfile>` can break out of the XML structure. Worse, the profile is written to `temp_dir().join(format!("micontrol_wifi_{ssid}.xml"))` — an SSID containing `..\..\` path traversal characters yields a path-traversal write, and an SSID with `"` or `<` corrupts the XML passed to `netsh wlan add profile filename`.

The subsequent `netsh wlan connect name <ssid>` passes `ssid` as a separate `.arg()` (safe from shell injection), but the **profile XML file** is the injection vector: a crafted SSID can inject arbitrary profile elements (e.g., a second `<SSID>` with a different auth mode, or a `<keyMaterial>` that netsh interprets).

**Recommended fix**

1. XML-escape `ssid` and `password` before interpolation:
   ```rust
   fn xml_escape(s: &str) -> String {
       s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
        .replace('"', "&quot;").replace('\'', "&apos;")
   }
   ```
2. Sanitize the SSID for the temp filename — use a hash or a fixed name (`micontrol_wifi_profile.xml`), not the raw SSID.
3. Validate the SSID against the 802.11 character set before building the profile.

---

### P1-3. `Script` hotkey action allows arbitrary command execution from user config
**File**: `src-tauri/src/hw/hotkeys.rs:1085-1108` (`HotkeyAction::Script` dispatch)
**Type**: A01 Broken Access Control / Arbitrary Code Execution (CWE-78)
**Severity**: Critical

**Description**

The `HotkeyAction::Script` variant, stored in `hotkeys.json`, executes arbitrary scripts:

```rust
HotkeyAction::Script { interpreter, path, args } => {
    let result = match interpreter.as_str() {
        "powershell" => Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-File", path.as_str()])
            .args(args)...
        "cmd" => Command::new("cmd").args(["/C", path.as_str()]).args(args)...,
        _ => Command::new(path).args(args)...,
    };
}
```

`hotkeys.json` lives in `%LOCALAPPDATA%\MiControl\hotkeys.json` — writable by any process running as the user. A malicious process can edit this file to add a `Script` action pointing to any payload, and it will execute **the next time any configured hotkey fires** (or on app restart when `load_config` reads it). The `cmd /C` path is especially dangerous: `path` can be `cmd`-interpreted.

There is **no validation** that `path` is an absolute path, no allow-listing, no signature check on the config file. Combined with P1-1, this is a persistence mechanism for malware.

**Recommended fix**

1. **Remove the `Script` action** or gate it behind an explicit user confirmation dialog in the UI (not just config-file editing).
2. If kept, validate `path` is absolute, exists, and is not an interpreter (`cmd.exe`, `powershell.exe`) when `interpreter` is empty.
3. Sign `hotkeys.json` with an HMAC and verify on load; reject unsigned/modified configs.
4. At minimum, log a prominent warning when a `Script` action is configured.

---

## Priority 2 (High) 🔴

### P2-1. `unsafe` pointer cast of IPC response header without alignment validation
**File**: `src-tauri/src/hw/iotservice.rs:382-385`
**Type**: Memory Safety / Undefined Behavior (CWE-119)
**Severity**: High

**Description**

```rust
let resp_header: &IpcWireHeader =
    unsafe { &*(resp_header_buf.as_ptr() as *const IpcWireHeader) };
```

`resp_header_buf` is a `[u8; 12]` on the stack. `IpcWireHeader` is `#[repr(C)]` with `u16, u16, u32, u32` — naturally aligned to 4 bytes. A stack `[u8; 12]` is **not guaranteed** to be 4-byte aligned. Casting an unaligned pointer to a `&IpcWireHeader` and reading `msg_type`/`payload_len` is **undefined behavior** on x86 (works in practice but UB per Rust's rules) and can trap on ARM.

The same pattern appears in `ecram.rs` (but `EcramBuf` is heap-allocated via `Vec`, so alignment is fine there) and in `touchpad.rs:830` (`buf.as_ptr() as *const RAWINPUT` — `Vec<u8>` is suitably aligned, so this is OK).

**Recommended fix**

Parse the header field-by-field from the byte buffer instead of casting:
```rust
let src_id = u16::from_le_bytes([resp_header_buf[0], resp_header_buf[1]]);
let dst_id = u16::from_le_bytes([resp_header_buf[2], resp_header_buf[3]]);
let msg_type = u32::from_le_bytes([resp_header_buf[4], resp_header_buf[5], resp_header_buf[6], resp_header_buf[7]]);
let payload_len = u32::from_le_bytes([resp_header_buf[8], resp_header_buf[9], resp_header_buf[10], resp_header_buf[11]]);
```
Or use `zerocopy` / `bytemuck` for safe transmutation.

---

### P2-2. `IpcWireHeader::as_bytes` uses `from_raw_parts` on a reference — safe but fragile
**File**: `src-tauri/src/hw/iotservice.rs:155-162`
**Type**: Memory Safety (CWE-119)
**Severity**: High (defense in depth)

**Description**

```rust
fn as_bytes(&self) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            self as *const IpcWireHeader as *const u8,
            std::mem::size_of::<IpcWireHeader>(),
        )
    }
}
```

This is technically safe because `IpcWireHeader` is `#[repr(C)]` and the slice length matches the struct size. However, it bypasses Rust's aliasing rules. Prefer `bytemuck::bytes_of(self)` or `zerocopy`. The same applies to the `EcramBuf` usage in `ecram.rs:300-310`.

**Recommended fix**: Use `bytemuck::bytes_of(&header)` or a manual `to_le_bytes` serialization.

---

### P2-3. ECRAM write path allows writing to arbitrary EC RAM offsets (hardware damage risk)
**File**: `src-tauri/src/commands/hardware.rs:115-145`, `src-tauri/src/hw/ecram.rs:410-440`
**Type**: A01 Broken Access Control / Hardware Safety (CWE-20)
**Severity**: High

**Description**

`write_iot_hex` accepts an arbitrary `address` and `hex_data` from the frontend. The "safe" path (`is_known_safe_single_byte_write`) only allows 9 specific offsets, but the **raw path** is enabled by setting `MICONTROL_ENABLE_RAW_ECRAM_WRITE=1` — an environment variable. Once enabled, it allows writing up to 32 bytes to **any address in the ERAM range** (`0xFE0B0300..0xFE0B0400`).

The ERAM range includes critical EC registers: fan control (`0x68`), TDP (`0x42`), battery thresholds (`0x96`), and charging flags. Writing wrong values can:
- Disable charging, brick the battery management.
- Set unsafe TDP/fan values → thermal damage.
- Corrupt the EC state machine.

The env-var gate is weak: any process running as the user can set environment variables for a new process launch, and the check is per-process (not per-call). There is **no confirmation dialog** in the UI for raw writes — `EcrDebugPanel.tsx` calls `write_iot_hex` directly on button click.

**Recommended fix**

1. Require an explicit, typed confirmation token (not just an env var) for raw writes — e.g., a UI dialog that returns a one-time token.
2. Narrow the raw-write range further or require a "developer mode" flag persisted in registry with a visible warning.
3. Log every raw write with timestamp, address, and data to an audit file.

---

### P2-4. Named pipe to IoTService has no server authentication
**File**: `src-tauri/src/hw/iotservice.rs:330-390`
**Type**: A07 Identification & Authentication Failures (CWE-345)
**Severity**: High

**Description**

`send_ipc_message` opens `\\.\pipe\LOCAL\IoTService_IPC_Broker` (or a path from the discovery profile) with `OpenOptions::new().read(true).write(true)`. There is **no verification** that the pipe server is the genuine `IoTService.exe`:

- Any process can create a named pipe with the same name (pipe squatting) if it wins the race at startup.
- The `iot_pipe_path` from `hardware_profile.json` is user-writable — a malicious edit can redirect IPC to an attacker-controlled pipe.
- The response is trusted verbatim: `payload_len` from the response header is used to allocate `vec![0u8; payload_len]` (bounded by `MAX_RESPONSE_PAYLOAD = 0x10000`, which is good), but the **content** is deserialized as JSON and forwarded to the frontend without sanitization.

A malicious pipe server could return crafted JSON that, while not directly exploitable in the current frontend (no `dangerouslySetInnerHTML`), could mislead the user (e.g., fake WiFi credentials, fake device status).

**Recommended fix**

1. Verify the pipe server's process image path matches `IoTService.exe` in the expected DriverStore directory (the driver itself checks this for IOCTL access — mirror it for the pipe).
2. Validate `iot_pipe_path` against a known-good prefix before opening.
3. Consider signing IPC responses with a shared key negotiated at service start.

---

### P2-5. `OpenUrl` hotkey action passes unvalidated URL to `explorer`
**File**: `src-tauri/src/hw/hotkeys.rs:1013-1020`
**Type**: A03 Injection (CWE-78, CWE-88)
**Severity**: High

**Description**

```rust
HotkeyAction::OpenUrl { url } => {
    let result = std::process::Command::new("explorer")
        .arg(url)
        ...
}
```

`url` comes from `hotkeys.json` (user-editable, see P1-3). `explorer.exe` interprets arguments specially: a URL like `file:///C:/Windows/System32/` opens Explorer at that path, and crafted arguments can launch executables. While `.arg()` avoids shell injection, `explorer` itself will happily open any file path or protocol handler (e.g., `ms-settings:`, `javascript:` via a registered handler).

Combined with the unsigned config file, this is a secondary arbitrary-execution vector.

**Recommended fix**

1. Validate `url` starts with `http://` or `https://` before passing to `explorer`.
2. Sign `hotkeys.json` (see P1-3).

---

### P2-6. Updater fetches release metadata over HTTPS but lacks pinning; pubkey is minisign
**File**: `src-tauri/tauri.conf.json:42-48`
**Type**: A08 Software & Data Integrity Failures (CWE-494)
**Severity**: High

**Description**

The updater config:
```json
"pubkey": "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEZCQjc3MTVCMkVDNDEyRDQKUldUVUVzUXVXM0czKzl2bGpUdjFrc2Nsd0d3SUwwUmJ3WGIrVTRELyt6VFZ0UEpsTWpqV3lKbHMK",
"endpoints": ["https://github.com/Freitas-MA/micontrol/releases/latest/download/latest.json"]
```

The endpoint uses GitHub releases (HTTPS) and the signature is verified with minisign (good). However:
- The endpoint follows GitHub redirects to `objects.githubusercontent.com` — a compromise of the GitHub account or a redirect interception could serve a malicious `latest.json` pointing to a malicious binary.
- The pubkey is embedded in the config (good), but there is **no certificate pinning** for the endpoint.
- `dialog: false` means updates can apply silently — combined with a stolen signing key, this is a remote code execution vector.

**Recommended fix**

1. Keep minisign verification (already present).
2. Set `dialog: true` or add a user-confirmation step for non-manual updates.
3. Document the key-rotation procedure and store the private key offline (HSM/password manager).
4. Consider a secondary signature or hash published on a separate channel (e.g., a static site).

---

## Priority 3 (Medium) 🟡

### P3-1. `unsafe` raw input buffer cast in touchpad.rs — size validation present but trust boundary is the driver
**File**: `src-tauri/src/hw/touchpad.rs:830-845`, `src-tauri/src/hw/hotkeys.rs:560-580`
**Type**: Memory Safety (CWE-119)
**Severity**: Medium

**Description**

Both files do:
```rust
let raw = buf.as_ptr() as *const RAWINPUT;
// ... access (*raw).data.hid, (*raw).header.dwType ...
```

The size is validated (`size > 4096` rejected, `written` checked), which is good. However, the code then accesses `(*raw).data.hid.bRawData` via `from_raw_parts(hid.bRawData.as_ptr(), hid.dwSizeHid as usize)` without validating that `dwSizeHid * dwCount` fits within `written`. A malicious or buggy driver could report `dwSizeHid` larger than the actual buffer, causing an out-of-bounds read.

**Recommended fix**: Validate `hid.dwSizeHid as usize * hid.dwCount as usize <= written as usize - offset_of!(RAWINPUT, data)` before slicing `bRawData`.

---

### P3-2. `HidD_SetFeature` report length derived from device-reported `FeatureReportByteLength`
**File**: `src-tauri/src/hw/touchpad.rs:410-440`
**Type**: Memory Safety (CWE-119)
**Severity**: Medium

**Description**

```rust
let feature_len = unsafe { ... HidP_GetCaps(preparsed, &mut caps) ... caps.FeatureReportByteLength as usize };
let report_len = feature_len.max(8).min(64);
```

The `.min(64)` cap is good, but `feature_len` is trusted from the device's HID descriptor. A malicious HID device (e.g., a USB rubber ducky) could report `FeatureReportByteLength = 0xFFFFFFFF`, which after `.min(64)` becomes 64 — safe. But if the `.min(64)` were ever removed, this would be a heap overflow. The current code is safe but fragile.

**Recommended fix**: Add a comment documenting the `.min(64)` invariant; consider a `const MAX_FEATURE_REPORT: usize = 64` constant.

---

### P3-3. `install_driver` resolves `.inf` paths but the elevated helper trusts the resolved path
**File**: `src-tauri/src/hw/discovery.rs:130-180`, `src-tauri/src/elevated.rs:265-280`
**Type**: A01 Broken Access Control / Path Traversal (CWE-22)
**Severity**: Medium

**Description**

`resolve_bundled_inf_by_name` correctly validates `driver_name` (no `\`, `/`, `:`, `..`) and canonicalizes the result to ensure it's inside `resources/`. **However**, this validation happens in the **unprivileged** process. The elevated helper (`elevated.rs:265`) receives `driver_name` as a string and calls `resolve_bundled_inf_by_name` again — but if an attacker bypasses the frontend and calls the elevated helper directly (via P1-1), they can supply a `driver_name` that passes the char check but the canonicalization runs in the elevated context.

The canonicalization check (`canon.starts_with(&resources_canon)`) is sound, so direct path traversal is blocked. The residual risk is that `pnputil /add-driver <path> /install` is called with an elevated token — if the resources dir is writable by the user (it is, in `%LOCALAPPDATA%`), an attacker can drop a malicious `.inf` there and have it installed with admin rights.

**Recommended fix**

1. Verify the `.inf` is signed or matches a known hash before calling `pnputil`.
2. Restrict the resources directory ACL to read-only for the user.

---

### P3-4. `set_device_status` and `set_performance_mode` accept arbitrary strings forwarded to IoTService
**File**: `src-tauri/src/commands/hardware.rs:300-320`, `src-tauri/src/hw/iotservice.rs:540-560`
**Type**: A03 Injection (CWE-20)
**Severity**: Medium

**Description**

`iot_set_device_status(status: String)` forwards `status` directly into a JSON payload to IoTService:
```rust
SetDeviceStatusRequest { status: status.to_string() }
```

There is no allow-list of valid status values. While the JSON serialization escapes special characters (safe from JSON injection), the `status` string is interpreted by IoTService's parser — if IoTService has any command-injection or format-string issues in its status handler, this is the entry point. The same applies to `set_performance_mode`'s `mode` string in the hotkey path (`hotkeys.rs:1055`), which is round-tripped through JSON.

**Recommended fix**: Validate `status` against a known enum (`["idle", "active", "update", ...]`) before forwarding.

---

### P3-5. `read_ai_perf_logs` deserializes untrusted JSONL without size limits
**File**: `src-tauri/src/commands/ai_logs.rs:90-130`
**Type**: A08 Software & Data Integrity Failures (CWE-502)
**Severity**: Medium

**Description**

`read_ai_perf_logs` reads `.jsonl` files from `%APPDATA%\MiControl\ai_perf_logs\` and deserializes each line with `serde_json::from_str::<AiPerfLogEntry>`. The log directory is user-writable. A malicious process could drop a multi-gigabyte `.jsonl` file or a line with a pathological structure causing `serde_json` to allocate excessively. The `cap` limits the number of entries (500) but not the file size read into memory (`read_to_string`).

**Recommended fix**

1. Cap the file size before `read_to_string` (e.g., skip files > 10 MB).
2. Use a streaming JSONL reader instead of `read_to_string`.

---

### P3-6. `debug_ecram_dump` and `get_iot_region_hex` expose raw EC memory to the frontend
**File**: `src-tauri/src/commands/system.rs:385-395`, `src-tauri/src/commands/hardware.rs:90-110`
**Type**: A01 Broken Access Control / Information Disclosure (CWE-200)
**Severity**: Medium

**Description**

`debug_ecram_dump` returns a hex dump of EC RAM (battery serial, WiFi bind status, device IDs) to any frontend caller. `get_iot_region_hex` accepts an arbitrary region name. While the frontend is the app's own UI, the data includes potentially sensitive identifiers (device ID, bind UID) that could be exfiltrated if the frontend is compromised (e.g., via an XSS in a future change — currently no XSS sinks exist, but the CSP allows `connect-src 'self' https:`).

**Recommended fix**

1. Gate `debug_ecram_dump` behind an explicit "developer mode" toggle.
2. Redact `device_id` and `uid` in `get_iot_device_info` unless explicitly requested.

---

### P3-7. CSP allows `style-src 'unsafe-inline'` and `connect-src https:`
**File**: `src-tauri/tauri.conf.json:24`
**Type**: A05 Security Misconfiguration (CWE-693)
**Severity**: Medium

**Description**

```json
"csp": "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; font-src 'self' https://fonts.gstatic.com; connect-src 'self' https:"
```

- `style-src 'unsafe-inline'` allows inline style injection (mitigated since there's no `script-src 'unsafe-inline'`, but still weakens the policy).
- `connect-src 'self' https:` allows the frontend to connect to **any** HTTPS endpoint — data exfiltration to an arbitrary server is possible if the frontend is compromised.
- `font-src https://fonts.gstatic.com` and `style-src ... https://fonts.googleapis.com` are broad third-party origins.

**Recommended fix**

1. Remove `'unsafe-inline'` from `style-src` (use nonces or hashes).
2. Restrict `connect-src` to the specific update endpoint: `connect-src 'self' https://github.com https://objects.githubusercontent.com`.
3. Self-host fonts instead of loading from Google Fonts (privacy + availability).

---

## Priority 4 (Low) 🟢

### P4-1. `elev_bridge` UAC fallback uses `ShellExecuteExW` with `SEE_MASK_NOASYNC` on a non-UI thread
**File**: `src-tauri/src/elev_bridge.rs:200-240`
**Type**: Reliability (CWE-362)
**Severity**: Low

`SEE_MASK_NOASYNC` combined with calling `ShellExecuteExW` from a tokio worker thread can deadlock if the API tries to pump messages on the calling thread. The code waits with `WaitForSingleObject(handle, 30_000)` which mitigates this, but the mask is intended for UI threads. Consider using `run_on_main_thread` for the ShellExecute call.

### P4-2. `cleanup_stale_elev_files` uses mtime, which can be spoofed
**File**: `src-tauri/src/elev_bridge.rs:330-360`
**Type**: A01 Broken Access Control (CWE-697)
**Severity**: Low

Stale file cleanup relies on `modified()` time. An attacker who creates a command file can set its mtime to the future to prevent cleanup, causing the directory to fill. Low impact (disk exhaustion only).

### P4-3. `winreg` usage in `set_windows_ptp_sensitivity` writes to HKCU without transaction
**File**: `src-tauri/src/hw/touchpad.rs:460-475`
**Type**: Reliability (CWE-362)
**Severity**: Low

Registry writes are not transactional; a crash mid-write could leave a partial value. Low impact since the values are simple DWORDs.

### P4-4. `wmi` crate 0.13 — check for advisories
**File**: `src-tauri/Cargo.toml:34`
**Type**: A06 Vulnerable & Outdated Components (CWE-1104)
**Severity**: Low

`wmi = "0.13"` is older; the latest is 0.14+. No known CVEs, but the WMI deserialization path (`raw_query`, `raw_notification`) parses untrusted WMI provider output. Ensure the crate is kept current.

### P4-5. `windows = "0.58"` — superseded by 0.59/1.0
**File**: `src-tauri/Cargo.toml:24`
**Type**: A06 Vulnerable & Outdated Components (CWE-1104)
**Severity**: Low

`windows-rs` 0.58 is functional but 1.0 is the current line. No security-relevant CVEs in 0.58, but staying current reduces risk of API misuse.

---

## Dependency Security Summary

| Dependency | Version | Status |
|---|---|---|
| tauri | 2.11.1 | ✅ Current (2.x line) |
| tokio | 1.52.3 | ✅ Current |
| serde_json | (locked) | ✅ No known CVEs |
| reqwest | 0.13.3 | ✅ Current |
| windows | 0.58 | ⚠️ Superseded by 1.0 |
| wmi | 0.13 | ⚠️ Older (0.14 latest) |
| winreg | 0.52 | ✅ Current |
| libloading | 0.8 | ✅ Current |
| react | 19.1.0 | ✅ Current |
| vite | 6.3.5 | ✅ Current |

No known critical CVEs in the pinned versions. Run `cargo audit` and `npm audit` in CI to catch future advisories.

---

## Positive Findings ✅

1. **CSP present** with `script-src 'self'` (no `'unsafe-inline'` for scripts) — strong XSS baseline.
2. **No `dangerouslySetInnerHTML`, `eval`, or `innerHTML`** in the frontend (grep confirmed).
3. **Tauri capabilities are minimal**: `core:default` + `shell:default` only, scoped to the `main` window.
4. **ECRAM write has a safe-path allow-list** (`is_known_safe_single_byte_write`) for 9 known offsets.
5. **IPC response payload is size-bounded** (`MAX_RESPONSE_PAYLOAD = 0x10000`) — no unbounded allocation.
6. **Raw input buffer sizes are validated** (`size > 4096` rejected) before casting.
7. **Driver `.inf` resolution canonicalizes and checks `starts_with(resources_canon)`** — solid path-traversal defense.
8. **`SendInput` injection is tagged with `MICONTROL_INJECT_MAGIC`** to prevent re-trigger loops.
9. **Updater uses minisign signature verification** (not just HTTPS).
10. **`asInvoker` manifest** — the app does not request admin by default; elevation is opt-in via the scheduled task.
11. **`panic = "abort"` in release** — reduces ROP gadget surface.
12. **`lto = true`, `strip = true`, `codegen-units = 1`** — hardened release profile.

---

## Recommended Action Plan

| Priority | Item | Effort |
|---|---|---|
| P1-1 | Secure the elevated bridge (ACL + HMAC + PID check) | High |
| P1-2 | XML-escape WiFi SSID/password | Low |
| P1-3 | Remove or gate `Script` hotkey action; sign `hotkeys.json` | Medium |
| P2-1 | Replace unsafe IPC header cast with byte parsing | Low |
| P2-3 | Add confirmation token for raw ECRAM writes | Medium |
| P2-4 | Authenticate IoTService pipe server | Medium |
| P2-5 | Validate `OpenUrl` scheme | Low |
| P2-6 | Enable updater dialog; document key rotation | Low |
| P3-7 | Tighten CSP (`connect-src`, remove `'unsafe-inline'`) | Low |

**Bottom line**: The app has a thoughtful security baseline (CSP, minisign updater, capability scoping, safe-path ECRAM writes). The critical gaps are in the **elevated-bridge trust boundary** (filesystem-based IPC without authentication) and **user-config-driven command execution** (`Script`/`OpenUrl`/`LaunchApp` actions from an unsigned config file). Fix P1-1, P1-2, and P1-3 before any public release.
