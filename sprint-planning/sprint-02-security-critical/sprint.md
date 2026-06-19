# Sprint 2 — Security Hardening: Critical Vulnerabilities

## Sprint Metadata

| Field | Value |
|-------|-------|
| **Sprint Name** | Security Hardening — Critical Vulnerabilities |
| **Sprint Goal** | Eliminate the 3 critical security vulnerabilities: elevated bridge privilege escalation, WiFi profile XML injection, and hotkey script arbitrary code execution |
| **Duration Estimate** | 3 weeks (15 working days) |
| **Priority** | P0 — Security-critical. These are exploitable by a local attacker. |
| **Sprint Type** | Security |
| **Primary Owner** | Rust security engineer |
| **Secondary Owner** | Threat model reviewer (auditor) |

## Sprint Goal Statement

Three critical vulnerabilities allow a local attacker to escalate privileges, inject commands into WiFi profile XML, or execute arbitrary code via hotkey configuration. By the end of this sprint, the elevated bridge must require authenticated commands, WiFi profile XML must be safely constructed with no injection vector, and hotkey script actions must be disabled or sandboxed with explicit user consent. Each fix must include a regression test demonstrating the exploit is closed.

---

## Background & Threat Model

The miPC elevated bridge runs a filesystem-based command/response protocol between the user-mode app and an elevated helper. The WiFi module constructs WLAN profile XML by string interpolation. The hotkey system supports a "script" action type that runs `cmd /C` with user-supplied content. All three are exploitable by a local, unprivileged attacker who can write to predictable file paths or supply crafted configuration.

---

## Tickets

### S2-001 — Authenticate elevated bridge commands and eliminate TOCTOU race

| Field | Value |
|-------|-------|
| **Ticket ID** | S2-001 |
| **Title** | Add HMAC authentication to elevated bridge protocol; replace filesystem command files with atomic, ACL-locked writes |
| **Priority** | P0 |
| **Type** | Security |
| **Estimated Effort** | XL |

#### Description

The elevated bridge (`src-tauri/src/elev_bridge.rs` ~lines 60–75 and `src-tauri/src/elevated.rs` ~lines 55–70) uses a filesystem-based command/response protocol: the user-mode app writes a command file, the elevated helper reads and executes it, then writes a response file. There is no authentication on the command file, and the write-then-read pattern is a classic TOCTOU (time-of-check/time-of-use) race: an attacker can swap the command file between write and read.

#### Affected Files and Line Ranges

- `src-tauri/src/elev_bridge.rs` — command file write (~lines 60–75).
- `src-tauri/src/elevated.rs` — command file read and execution (~lines 55–70).
- Any shared protocol definition module.

#### Root Cause Analysis

1. **No authentication**: any process that can write to the command file path can issue elevated commands. The path is in a predictable temp/appdata location.
2. **TOCTOU race**: the elevated helper checks the file (e.g. existence, maybe a magic header) then reads it. Between check and read, an attacker can replace the file contents.
3. **Default ACLs**: the command/response files are written with default filesystem ACLs, so other user-context processes can read or replace them.

#### Acceptance Criteria

- [ ] Every command and response message includes an **HMAC-SHA256 tag** computed over the message body using a shared secret established at bridge startup (e.g. a random 256-bit key exchanged via a named pipe with the elevated helper's initial handshake, or derived from a machine+session token).
- [ ] The elevated helper **rejects** any command whose HMAC does not verify, logs the rejection, and does not execute.
- [ ] Command/response files are written with a restrictive ACL: only the current user and SYSTEM have access. On Windows, use `CreateFile` with a `SECURITY_ATTRIBUTES` that grants access only to the current user SID and SYSTEM.
- [ ] The TOCTOU race is eliminated by writing commands atomically (write to a temp file in the same directory, then `MoveFileEx` with `MOVEFILE_REPLACE_EXISTING` and `MOVEFILE_WRITE_THROUGH`) and having the helper read the file once into a buffer before any validation.
- [ ] A **nonce/timestamp** is included in each command to prevent replay; the helper rejects commands older than 30 seconds or with a reused nonce.
- [ ] A regression test demonstrates that an unauthenticated command file (no valid HMAC) is rejected and not executed.
- [ ] A regression test demonstrates that a command file swapped after write (simulated by writing a valid command, then overwriting with a malicious body before the helper reads) is rejected due to HMAC mismatch.

#### Implementation Notes

- Key exchange: the simplest robust approach is for the elevated helper to generate a random key on startup and write it to a file with a restrictive ACL readable only by the current user, then the user-mode app reads it once. Alternatively, use a named pipe with `ImpersonateNamedPipeClient` for a more robust exchange. Document the chosen approach.
- Use the `hmac` and `sha2` crates (add to `Cargo.toml`).
- Atomic write pattern: write to `cmd.tmp`, then rename to `cmd.json`. The helper reads `cmd.json` into a `Vec<u8>` in one `read_to_end` call before parsing — this closes the read window.
- Consider migrating away from filesystem IPC entirely to a named pipe with `RpcImpersonationLevel` in a future sprint; this sprint hardens the existing mechanism.

#### Testing Strategy

- **Unit tests**: HMAC verification logic with valid/invalid/missing tags.
- **Integration test**: launch the elevated helper in a test harness, send an unauthenticated command, assert rejection.
- **Security regression test**: simulate the TOCTOU swap (write valid, overwrite body, trigger read) and assert rejection.
- **Manual penetration test**: attempt to issue an elevated command from a separate unprivileged process; confirm rejection.

#### Dependencies

- None (foundational security fix).

---

### S2-002 — Escape WiFi profile XML to prevent injection

| Field | Value |
|-------|-------|
| **Ticket ID** | S2-002 |
| **Title** | Escape SSID and password in WLAN profile XML; validate against XML injection |
| **Priority** | P0 |
| **Type** | Security |
| **Estimated Effort** | M |

#### Description

In `src-tauri/src/hw/wifi.rs` (~lines 55–110), the `connect` function interpolates `ssid` and `password` directly into a WLAN profile XML template via `format!`. A malicious or malformed SSID/password containing XML metacharacters (`<`, `>`, `&`, `"`, `'`) can break out of the XML structure, inject arbitrary profile elements, or cause the `netsh wlan add profile` command to behave unexpectedly.

#### Affected Files and Line Ranges

- `src-tauri/src/hw/wifi.rs` — `connect` function (~lines 55–110), the `profile_xml` `format!` call.

#### Root Cause Analysis

The code does:

```rust
let profile_xml = format!(r#"...<name>{ssid}</name>...<keyMaterial>{pwd}</keyMaterial>..."#, ssid, pwd);
```

No escaping is applied. An SSID like `test</name><name>evil` would close the `<name>` element early and inject a new one. A password containing `</keyMaterial>` could truncate the key and inject trailing XML. While `netsh` may reject some malformed XML, relying on the downstream parser for safety is fragile and version-dependent.

#### Acceptance Criteria

- [ ] A `fn escape_xml(s: &str) -> String` helper is added that escapes `&` → `&amp;`, `<` → `&lt;`, `>` → `&gt;`, `"` → `&quot;`, `'` → `&apos;`.
- [ ] Both `ssid` and `password` are passed through `escape_xml` before interpolation into the template.
- [ ] The SSID is additionally validated against WLAN SSID rules: max 32 bytes, no null bytes. Invalid SSIDs return an `Err` before XML construction.
- [ ] The password is validated: 8–63 characters for WPA2 passphrases (per IEEE 802.11). Out-of-range passwords return an `Err`.
- [ ] The temp profile file is written with a restrictive ACL (only current user) and deleted immediately after import (existing behavior, but verify the cleanup runs even on error paths).
- [ ] Unit test: SSID `a</name><name>b` produces escaped output with no element breakout.
- [ ] Unit test: password containing `</keyMaterial><x>` produces escaped output.
- [ ] Unit test: 33-byte SSID is rejected.
- [ ] Unit test: 7-character password is rejected.

#### Implementation Notes

- Place `escape_xml` in a shared util module (e.g. `src-tauri/src/util/xml.rs`) so it can be reused if other XML is constructed.
- Consider using a proper XML builder crate (e.g. `quick-xml`) for robustness, but a correct manual escaper is acceptable for this sprint given the fixed template structure.
- Ensure the temp file path does not collide with attacker-controlled names — use a random suffix (e.g. `uuid`) in the filename.

#### Testing Strategy

- **Unit tests** for `escape_xml` covering all five metacharacters.
- **Unit tests** for SSID/password validation rules.
- **Integration test**: attempt to connect with an injection-crafted SSID; confirm the profile XML is well-formed (parse it with an XML parser in the test) and no breakout occurs.

#### Dependencies

- None.

---

### S2-003 — Disable or sandbox hotkey script actions with explicit consent

| Field | Value |
|-------|-------|
| **Ticket ID** | S2-003 |
| **Title** | Remove arbitrary code execution via hotkey "script" action; require explicit user consent and signing |
| **Priority** | P0 |
| **Type** | Security |
| **Estimated Effort** | L |

#### Description

In `src-tauri/src/hw/hotkeys.rs` (~lines 1085–1108), the hotkey system supports a "script" action type that executes `cmd /C <script>` with the content read from `hotkeys.json`. Because `hotkeys.json` is an unsigned, user-writable config file, any process that can modify it can execute arbitrary commands with the app's privileges. This is an arbitrary code execution vector.

#### Affected Files and Line Ranges

- `src-tauri/src/hw/hotkeys.rs` — script action handler (~lines 1085–1108).
- `hotkeys.json` schema/config definition.

#### Root Cause Analysis

The "script" action type was likely intended for power-user customization, but it provides a direct code execution path from an unsigned config file. There is no consent dialog, no signing, and no allowlist. An attacker who can write to the config file (or trick the user into importing a malicious config) gains code execution.

#### Acceptance Criteria

- [ ] The "script" action type is **disabled by default** — it is not processed unless an explicit feature flag / registry setting enables it.
- [ ] When enabled, a script action requires an **explicit consent dialog** the first time it is triggered (not at config load): "This hotkey will run the command: `<cmd>`. Allow?" with Allow / Deny / Always Allow options.
- [ ] "Always Allow" decisions are stored in a separate, ACL-protected consent file keyed by a hash of the script content (so changing the script re-prompts).
- [ ] The script content is validated against an **allowlist** of permitted executables (e.g. only `cmd.exe`, `powershell.exe` from System32) — arbitrary paths are rejected.
- [ ] A `log::warn!` is emitted every time a script action executes, including the command and the triggering hotkey.
- [ ] Unit test: a script action with the feature flag disabled is a no-op (logged, not executed).
- [ ] Unit test: a script action with a non-allowlisted executable path is rejected.
- [ ] Unit test: the consent dialog state machine correctly transitions: first run prompts, "Always Allow" stores the hash, subsequent runs skip the prompt.

#### Implementation Notes

- Feature flag: read from a registry key under `HKCU\Software\miPC\hotkeys` (`EnableScriptActions` DWORD, default 0). Document this in the user guide.
- Consent file: `appdata/micontrol/hotkey_consent.json`, written with restrictive ACL, mapping `sha256(script)` → `bool`.
- Allowlist: a `const ALLOWED_EXECUTABLES: &[&str]` containing canonical paths. Resolve the script's executable via `CreateProcess` path resolution and compare.
- Consider deprecating the script action entirely in a future release; this sprint makes it safe-by-default.

#### Testing Strategy

- **Unit tests** for the feature-flag check, allowlist, and consent state machine.
- **Security regression test**: with the flag disabled, place a malicious `hotkeys.json` with a script action; confirm no execution occurs.
- **Manual test**: enable the flag, trigger a script hotkey, confirm the consent dialog appears and execution only proceeds on Allow.

#### Dependencies

- None.

---

## Sprint Exit Criteria

- [ ] All 3 tickets merged and security-reviewed.
- [ ] `cargo check` and `cargo test` pass.
- [ ] Each fix has a regression test demonstrating the exploit is closed.
- [ ] Threat model updated to reflect the mitigations.
- [ ] No new attack surface introduced (reviewed by auditor).

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| HMAC key exchange itself is vulnerable to MITM | Use restrictive ACLs on the key file; consider named-pipe impersonation in a follow-up. |
| WiFi escaping breaks legitimate SSIDs with special chars | Escaping is reversible by the XML parser; valid SSIDs survive round-trip. |
| Disabling script actions breaks existing power-user configs | Feature flag allows opt-in; consent dialog preserves usability. |
| Consent file becomes a new attack target | ACL-protect it; hash-keying means content changes re-prompt. |
