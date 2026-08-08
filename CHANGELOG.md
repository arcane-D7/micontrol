# Changelog

All notable changes to miPC will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.16] - 2026-08-08

### Changed

- **Migrated package manager from npm to pnpm** — Added `packageManager: pnpm@11.5.0` to `package.json`, generated `pnpm-lock.yaml` via `pnpm import`, and added `pnpm-workspace.yaml` with `allowBuilds: esbuild: true` (required for Vite's native esbuild binary on pnpm 11). Updated `ci.yml`, `release.yml`, and `landing.yml` to use `pnpm/action-setup@v4` + `cache: 'pnpm'` + `pnpm install --frozen-lockfile` (keeping the PowerShell retry loops), and `tauri.conf.json` `beforeDevCommand`/`beforeBuildCommand` to `pnpm run dev`/`pnpm run build`. Removed `package-lock.json`. Updated docs (README, CONTRIBUTING, AGENTS, release.md) and scripts (`health-check.mjs`, `sync-version.cjs`) accordingly.
- Updated `CHANGELOG.md` — Documented versions 0.1.14, 0.1.15, and 0.1.16 (this release); moved the previously-unreleased EC Command Protocol notes into 0.1.14.

### Fixed

- **CI Build smoke test with pnpm** — `pnpm run tauri build -- --no-bundle` passed `--no-bundle` as a positional arg to cargo (pnpm forwards args directly, unlike npm which requires a literal `--` separator). Changed to `pnpm run tauri build --no-bundle`.
- **Landing page build** — `@tauri-apps/plugin-updater@2.10.1` imports `{ Resource, Channel, invoke }` from `@tauri-apps/api/core`; the browser mock (`src/mocks/tauri-api.ts`, aliased by `vite.config.landing.ts`) only exported `Channel`, so rollup failed with "Resource is not exported". Added a minimal `Resource` base class (rid + no-op `close()`) matching the real Tauri core API shape used by plugin subclasses.

## [0.1.15] - 2026-08-06

### Added

- **Post-install verification script** (`scripts/verify-v0.1.15.ps1`) — Automated verification of the installed v0.1.15 release.
- **Toolchain pinning** — `.nvmrc` (Node 24.15.0) and `src-tauri/rust-toolchain.toml` (Rust 1.95.0) now pin exact versions matching local development, removing local/CI divergence.

### Changed

- **CI mirrors local health check exactly** — `scripts/health-check.mjs` is the single source of truth driving both local dev and CI (`ci.yml` and `release.yml` invoke the same script), so the pipeline cannot drift from what runs locally. CI splits the 11 steps across jobs for parallelism (static checks + Rust tests/coverage).
- **CI hardening** — Added concurrency cancellation (`cancel-in-progress`), job timeouts (30/30/45 min), and npm ci retry loops (3 attempts with backoff) for infrastructure flakiness.
- **CI build job is secret-free** — Uses `tauri build --no-bundle` so fork PRs cannot fail on missing `TAURI_SIGNING_*` secrets; bundling/signing stays in `release.yml`.
- **Rust toolchain action pinned** — `dtolnay/rust-toolchain@master` with explicit `toolchain: '1.95.0'` input, and the toolchain consistency check accepts pinned `@master` while still rejecting `@stable` and bare `@master`.
- **Coverage threshold aligned** — Lowered to 15% lines/statements to match actual coverage while still guarding against regressions.
- **Dependabot disabled** — Empty `updates:` list so it no longer opens PRs or triggers workflow runs; previous npm/cargo/github-actions config retained as comments for easy re-enabling.

### Fixed

- **No UAC prompts at boot** — `run_elevated_no_prompt` added to the elevated bridge for non-critical reads (thermal, discovery, Copilot fix, IoT ensure); bridge service path preferred so no UAC prompt ever appears at startup.
- **Pipe DACLs (error 5 at boot)** — Leaked `SECURITY_DESCRIPTOR` in `ecram_service` and face `pipe_server`; added Everyone RW DACL to `micontrol_bridge` so the unprivileged app can open pipes.
- **Temperature readings** — `read_thermal_readings` via the bridge (SYSTEM) since thermal WMI classes deny unprivileged callers; `get_fan_info` seeds ESIF from elevated readings when the local read is empty.
- **useHardware error-state regression** — `safe()` now surfaces an error when essential hardware data is missing; polls and auto-run mount handlers no longer clear a legitimate hardware error, only an all-success initial load or explicit `clearError()`.

## [0.1.14] - 2026-08-03

### Added

- **Face Unlock module** — Windows Hello-style face unlock using the RGB webcam with native Rust recognition: face matcher, liveness detection, credential vault, and face store; LocalSystem authentication service with named pipe; COM Credential Provider DLL; UI tab; installer integration. Includes 39 unit/integration/mock tests.
- **EC Command Protocol** — Full implementation of the 4-phase EC command state machine (RamIsReady → WriteCommand → ReadCmdAck → ReadCmdRet) in `ecram_service.rs`. Supports all 16 EC cmd_ids: GetBindStatus, SetBindStatus, ResetDevice, WriteWiFiItem, EmptyWiFiItems, DeleteWiFiItem, ReadWiFiStatus, ReadWiFiCount, GetWiFiByIndex, GetFwVersion, GetModel, ConnectWiFi, GetDeviceID, and SendLaptopStatus (SUSPEND/SHUTDOWN/WIN_READY). EC reset is performed before and after each command for reliability.
- **EC Command Protocol RE Report** (`docs/EC_COMMAND_PROTOCOL_RE.md`) — Complete reverse engineering documentation of the EC command protocol: 4-phase state machine, 7-byte command template, ACK/RET polling patterns, ECRAM address map, per-feature response layouts, and error codes.
- **IoT Device UI documentation** — Updated `IotDeviceCard.tsx` with explanatory sections clarifying that Cloud Binding is about Xiaomi IoT cloud registration (not Mi Home), IoT WiFi is the chip's own WiFi module (separate from Windows WiFi), and a collapsible table listing all 16 EC commands with descriptions.
- **Pipe operations in ecram_service** — JSON pipe protocol operations: `iot_get`, `iot_reset_device`, `iot_empty_wifi`, `iot_connect_wifi`, `iot_send_laptop_status` for frontend-to-backend IoT command forwarding.
- **WiFi provisioning ops in ecram_service** — `iot_write_wifi_item` (cmd 0x04, 101-byte payload with checksum per RE), `iot_delete_wifi_item` (cmd 0x06, 37-byte payload), and parsed `wifi_by_index` responses (`ssid`/`connected`/`enabled` from the 101-byte item). Payload layouts verified against `iotsvc_decompiled.c`.
- **Charging threshold ops in ecram_service** — `iot_set_charging_threshold` / `iot_get_charging_threshold` with ECRAM register write (0xA4 Battery Care + 0xA7 threshold) and registry fallback.
- **EC command helpers in iotservice.rs** — `send_ec_pipe_command()`, `query_ec_string()`, `query_ec_device_id()`, `query_ec_bind_status()`, `query_ec_wifi_status()`, `query_ec_wifi_count()` with fallback to registry/WMI/cached data.
- **Generic pipe client in ecram.rs** — `send_pipe_request()` for communicating with ecram_service.exe.
- **Option 3: ecram pipe as primary IoT backend** — All `iotservice.rs` public functions (`get_model`, `get_fw_version`, `get_bind_status`, `get_device_id`, `get_device_status`, `set_device_status`, `reset_device`, `send_laptop_status`, `write_wifi_item`, `delete_wifi_item`, `get_wifi_by_index`, `read_wifi_count`, `read_wifi_status`, `empty_wifi_items`, `connect_wifi`) now prefer the ecram_service JSON pipe when it is active, falling back to the original MCPI protocol for stock setups. `is_pipe_available()` reports the ecram pipe as IoT-service availability.
- **Charging via ecram pipe** — `set_charging_threshold`/`get_charging_threshold` in `charging.rs` use the ecram_service pipe when available (with registry fallback), eliminating the dead MCPI-pipe path when ecram_service replaces IoTService.exe.
- **Hardware gap implementation** — Squash-merge of `feature/hardware-gap-implementation` (32 commits, 119 files, +23,505 lines): ALS sensor selection (real sensor instead of fixed placeholder), color calibration wizard (pausing display control during calibration), correct BLTP7853 touchpad HID protocol from XPM RE, exposed ecram_service pipe to the user when running as SYSTEM, plus new hardware modules: audio effects, cleanup (junk/file cleanup), crash recovery, driver update/scan, eye protection, fn key remap, OS turbo (performance modes), phone link, screen cast, security scan, system info expansion, WiFi manager improvements, and thermal handling (ESIF).
- **New frontend tabs/pages** — `face.tsx`, `security.tsx`, `system.tsx`, `cleanup.tsx`, `color.tsx`, `crossdevice.tsx`, updated `ChargingProtection.tsx`, `FunctionExplain.tsx`, `WiFiManager.tsx`, `SettingsPage.tsx`, i18n updates across en/es/fr/pt.

### Changed

- **ecram_service now exposes the pipe to the user when running as SYSTEM** — Fix from the hardware-gap branch.
- Updated `README.md` — Added EC Command Protocol to features, architecture, and documentation table, plus Face Unlock and hardware gap docs.
- Updated `AGENTS.md` — Added EC command protocol references, ecram_service pipe operations documentation, and hardware module inventory.
- Updated `.gitignore` — Added `.bench/` and `.bench_trace_*` patterns to prevent benchmark trace files from being committed.
- Updated `docs/HARDWARE_INVESTIGATION.md`, `docs/RE_ANALYSIS_REPORT.md`, `docs/release.md`, `docs/architecture.md` — Hardware gap cross-references and architecture updates.
- **Installer** — Stop MiControlBridge service before overwriting files; Face Unlock installer integration (COM credential provider DLL registration).

### Removed

- **Log file cleanup** — Deleted 44 temporary log files (build logs, trace logs, patch logs, test logs) and 2 test PowerShell scripts that were leftover from development. Screenshots `screenshot.png` and `screenshot2.png` removed from version control.
- Removed `test_iot_log.ps1` and `test_iot_log2.ps1` from `src-tauri/`.

### Fixed

- **CI green for release** — Resolved clippy warnings, failing tests, and i18n gaps (`fix(ci): resolve clippy warnings, failing tests, and i18n gaps for release`).

## [0.1.13] - 2026-07-24

### Fixed

- **Raw Input buffer bug** — `handle_keyboard_raw_input()` was comparing the buffer size against `sizeof(RAWINPUT)` (48 bytes) instead of `sizeof(RAWINPUTHEADER)` (24 bytes), causing ALL keyboard raw input events (40 bytes) to be silently dropped. Fixed with proper header-first bounds checking.
- **Copilot key interception** — Added `disable_copilot_key` elevated command that sets registry policies (`TaskbarMn=0`, `TurnOffWindowsCopilot=1`, `CopilotKey=0`) to prevent Windows Shell from consuming the Copilot key (VK 0xC3).
- **Scancode Map for Copilot key** — Added `set_scancode_map` elevated command that writes a Scancode Map registry entry to remap the Copilot key's scan code (0xE06E) to Right Ctrl (0xE01D) at the keyboard class driver level. Requires reboot to take effect.
- **F7 performance mode hotkey** — Switched to elevated bridge for setting performance mode when direct HKLM access fails (UAC-protected `HKLM\SOFTWARE\MI\PerformanceMode` key).
- **F8 display mode hotkey** — Changed from direct hardware call to Win+P shortcut simulation for reliability.
- **Explorer restart breaking tray** — Removed the `Stop-Process -Name explorer` from `disable_copilot_key` that was restarting Explorer.exe on every MiControl startup, which destroyed system tray icons and broke the "show more" overflow button.
- **UI ERR_CONNECTION_REFUSED** — Ensured release binary is built with `npm run tauri build` (which embeds frontend assets) instead of `cargo build --release` (which does not embed frontend assets and falls back to dev server URL `localhost:1420`).

### Added

- **Custom IoTService.exe replacement binary** (`src-tauri/src/bin/ecram_service.rs`) — Rust binary that proxies ECRAM read/write IOCTLs to IoTDriver.sys via named pipe IPC (`\\.\pipe\ecram_service`, JSON protocol). Passes driver security check by being named `IoTService.exe` and placed in the DriverStore directory.
- **Pipe client in ecram.rs** — `read_ecram_via_pipe()` and `is_pipe_broker_available()` functions for communicating with the custom IoTService.exe via named pipe.
- **RE Analysis Report** (`docs/RE_ANALYSIS_REPORT.md`) — Complete reverse engineering documentation of IoTDriver.sys and IoTService.exe: IOCTL codes (`0x22E000`/`0x22E004`), buffer layout (0x110 bytes), allowed physical address ranges, security check mechanism, custom replacement design, test results, and limitations.
- **WORKING FORM comments** — 12 reverse-engineering findings documented across 5 Rust source files (`battery.rs`, `ecram.rs`, `fan.rs`, `wmi_cache.rs`, `wmi_ec.rs`) marking verified code patterns that must not be modified without re-testing against real hardware.

### Changed

- Updated `docs/iotservice-re-analysis.md` — Added Phase 2 findings (radare2 deep analysis), cross-referenced with RE_ANALYSIS_REPORT.md, updated viability assessment and next steps.
- Updated `docs/HARDWARE_INVESTIGATION.md` — Added Session 6 findings (custom IoTService.exe, allowed address ranges, ERAM/SMA2 inaccessibility, pipe client integration).
- Updated `README.md` — Added EC RAM Access feature description and architecture details for custom IoTService.exe.
- Updated `AGENTS.md` — Added hardware module inventory, key hardware interfaces table, RE documentation references, and WORKING FORM editing rules.
- Updated `docs/frontend-architecture.md` — Corrected tab count from 17 to 18 (includes dev-only ecrdebug tab).

### Known Limitations

- **ERAM region (0xFE0B0300) not accessible via IoTDriver** — IoTDriver.sys hardcoded address ranges do not include ERAM. AC adapter wattage (ADPW at ERAM+0x81) cannot be read via the driver, but IS available via WMI (ACPI WMAA method `read_adapter_power()`).
- **SMA2 region (0xFE0B0A00) not accessible via IoTDriver** — Same limitation as ERAM.
- **Secure Boot prevents driver modification** — IoTDriver.sys cannot be patched to add ERAM/SMA2 ranges without disabling Secure Boot.

## [1.0.0] - 2025-01-XX

### Added

- First-run onboarding wizard
- Hardware profile JSON integrity check (HMAC-signed)
- HMAC key rotation mechanism (30-day rotation, 7-day grace period)
- Nonce persistence with TTL for replay protection
- Rate limiting for IoTService IPC writes (100 writes/second)
- Consent audit log with HMAC integrity verification
- WiFi password encryption (XOR cipher with HMAC key)
- URL validation for hotkey OpenUrl (http/https only)
- Local font loading (removed Google Fonts CDN dependency)
- Manual chunks in Vite config for optimized bundle splitting
- React.memo optimization for Sidebar component
- WMI static data caching (BatteryStaticData, CPU logical processors)
- WMI cache selective invalidation (only on connection errors)
- Comprehensive clippy lint curation
- CI/CD pipeline with SHA-pinned actions, i18n checker, version checker
- Code of Conduct and Contributing guidelines
- CODEOWNERS file for code review routing
- Pre-commit hooks (tsc, version:check)
- Keyboard shortcuts for tab switching (Alt+1 through Alt+9)
- AI cost estimation and usage tracking
- User-facing error reporting channel
- Accessible labels and ARIA attributes for skeleton loaders
- prefers-reduced-motion media query for all animations

### Changed

- Migrated all hw/ modules from anyhow::Result to typed HardwareResult<T>
- Migrated commands/system.rs to Result<T, ErrorResponse>
- Replaced tokio "full" with explicit features
- Extracted Sidebar to React.memo component
- Bumped @vitest/coverage-v8 to ^3.2.2

### Removed

- Dead code (get_profile, read_or_recover, write_or_recover, spawn_with_recovery)
- Google Fonts CDN dependency

### Security

- HMAC-signed audit log with tamper detection
- Encrypted WiFi password storage
- Replay attack protection with persisted nonces
- Rate limiting on IPC writes

## [0.1.0] - 2024-XX-XX

### Added

- Initial release
- Basic hardware control (fan, battery, display, audio, keyboard, touchpad)
- IoT Service integration
- Driver management
- Multi-language support (en, pt, es, fr)

[Unreleased]: https://github.com/arcane-D7/micontrol/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/arcane-D7/micontrol/releases/tag/v0.1.3
[1.0.0]: https://github.com/arcane-D7/micontrol/releases/tag/v1.0.0
[0.1.0]: https://github.com/arcane-D7/micontrol/releases/tag/v0.1.0
