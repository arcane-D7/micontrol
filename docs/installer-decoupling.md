# Decoupled Installer Architecture (v2)

> Status: implemented — shell crate `src-tauri/shell/`, release script
> `scripts/publish-release.ps1` (decoupled edition).

## Why

The old NSIS setup bundled the entire app AND all privileged logic in one
version-dependent binary. That produced two chronic problems:

1. **Unstable installer hash** — every release produced a different
   `MiControl_<ver>_x64-setup.exe` (~107 MB), so there was never a stable
   "installer" artifact.
2. **Self-kill races** — updates were driven by the NSIS installer, which had
   to kill the very processes that orchestrated it (the `MiControlBridge`
   SYSTEM service launches the installer; the installer's `KillBridgeProcess`
   then terminates that service mid-flight). This caused the SCM
   error-1072/1053 install failures documented in v0.1.21/v0.1.22/v0.1.23.

## The decoupling

```
MiControl_Setup.exe                ← THE SHELL (static, hash-stable)
  • tiny Rust crate (src-tauri/shell), ~2.5 MB
  • contains NO app payload, NO version metadata
  • installs itself into <INSTDIR>\setup\ and NEVER gets replaced
  • performs every privileged operation: install / update / uninstall

MiControl_<version>_payload.zip    ← THE PAYLOAD (versioned)
  • app binaries + drivers + services + payload.json
  • minisign-signed with the SAME key pair the Tauri updater uses
  • the versioned part: changes every release

latest.json                        ← updater manifest → payload URL
```

The structural win: **updates only replace payload files, never the shell**,
so the shell never needs to kill or replace its own process. The bridge hands
the payload to the shell and dies in peace; the shell restarts everything.

## Flows

### First install

1. User downloads `MiControl_Setup.exe` (hash never changes).
2. `install` → UAC → copies itself to `<INSTDIR>\setup\micontrol_setup.exe`
   → re-launches from there (`apply-install`).
3. Shell resolves the payload: `--payload <zip>` → `--url <url>` → latest
   GitHub release (GitHub API, asset `MiControl_*_payload.zip`).
4. Verifies the minisign signature against the embedded public key (same key
   as `tauri.conf.json` `plugins.updater.pubkey`). Unsigned payloads are
   REFUSED unless `--allow-unsigned` is passed (dev only).
5. Kills app/services → extracts zip (zip-slip guarded; `setup\` never
   touched) → applies ALL install points (ported 1:1 from
   `installer-hooks.nsi`): Run-key autostart, pnputil drivers, DriverStore
   `ecram_service` deploy + IoTSvc, `MiControlElevated` scheduled task,
   `MiControlBridge` service, Face Unlock service + Credential Provider,
   post-install verification, WebView2 check, uninstaller registration,
   shortcuts → launches the app.

### Update (app-driven)

1. Tauri updater plugin downloads `MiControl_<ver>_payload.zip` and verifies
   the signature (unchanged mechanism — only `latest.json` now points at the
   payload).
2. `find_latest_downloaded_installer` (Tauri command) locates the downloaded
   artifact (`.exe` OR `.zip`) and copies it to
   `%LOCALAPPDATA%\MiControl\<canonical name>`.
3. The app relays `install_update` through the SYSTEM bridge (no UAC).
4. `elevated.rs::install_update` detects the `_payload.zip` name and launches
   `<appdir>\setup\micontrol_setup.exe update --payload <zip>` — detached,
   fire-and-forget.
5. The shell kills app + services, extracts, re-applies every install point
   and relaunches the app for the logged-on user (WTS-aware when running as
   SYSTEM).

### Uninstall

`micontrol_setup.exe uninstall` → stages itself in `%TEMP%` → kills
everything, removes services/task/registry/shortcuts, restores the DriverStore
`IoTService.exe.bak`, deletes the install dir and the uninstall key.

## S50 security gate (unchanged semantics, extended)

The SYSTEM bridge only launches files matching:

- `MiControl_<version>[-<channel>]_x64-setup.exe` (legacy NSIS path), or
- `MiControl_<version>[-<channel>]_payload.zip` (decoupled path).

Both gates live in `src-tauri/src/elevated.rs`
(`is_micontrol_installer_name` / `is_micontrol_payload_name`, unit-tested).

## Release pipeline

`scripts/publish-release.ps1`:

1. `tauri build -f face --bundles nsis` (validates the bundle; NSIS setup
   remains a manual fallback).
2. **Shell**: cached in `setup-shell/MiControl_Setup.exe` (gitignored). If
   present it is REUSED byte-for-byte (stable hash); `-RebuildShell` forces a
   rebuild. New builds are Authenticode-signed (signtool, same cert as the
   bundler) and the SHA256 is printed.
3. **Payload**: zips `target/release/{micontrol.exe, ecram_service.exe,
micontrol_bridge.exe, micontrol_face_svc.exe, micontrol_facecp.dll,
drivers/, resources/, WebView2Loader.dll}` + `payload.json`.
4. Payload is signed with the updater key (`tauri signer sign`) → `.sig`.
5. `latest.json` points at the **payload** URL with the payload signature.
6. Release assets: payload zip + `.sig` + `latest.json` + **static shell** +
   (legacy) NSIS setup + `.sig`.

## Shell CLI reference

```
micontrol_setup install   [--payload <zip>] [--url <url>] [--dir <dir>] [--silent]
micontrol_setup update    --payload <zip> [--dir <dir>] [--no-relaunch]
micontrol_setup uninstall [--dir <dir>]
micontrol_setup status    [--dir <dir>]
```

Build: `cargo build -p micontrol-setup --profile shell-release`.

## Migration notes

- Machines installed with the OLD NSIS flow: the shell detects the existing
  `InstallLocation` from the uninstall registry key, refreshes files in place
  and re-registers the uninstaller to point at the shell. The legacy
  `uninstall.exe` is removed on the first shell-applied update.
- The `install_update` elevated command keeps backwards compatibility: it
  still accepts the NSIS setup exe when provided.
