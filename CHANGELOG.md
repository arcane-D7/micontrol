# Changelog

All notable changes to miPC will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/arcane-D7/micontrol/compare/v0.1.1...v0.2.0) (2026-07-03)


### Features

* implement WMI WMAA hardware access and fix MCPI IPC protocol ([e70e5f8](https://github.com/arcane-D7/micontrol/commit/e70e5f8c1a4ad962c7283283126f3f4a195c21ae))
* **sprint-31:** implement 8 P1 high-priority fixes and eliminate all stubs ([9a8f9fc](https://github.com/arcane-D7/micontrol/commit/9a8f9fc38a5b9ff47673405b4f23e17a49a30f13))
* **sprint-32:** implement 7 P2 medium-priority hardware reliability fixes ([8918a9c](https://github.com/arcane-D7/micontrol/commit/8918a9ce97cd33c3829875a539ebb753d3f19c23))
* **v0.1.2:** custom IoTService.exe, ECRAM pipe client, RE documentation, keyboard tab ([83d0bcf](https://github.com/arcane-D7/micontrol/commit/83d0bcf3310bc170d0f51109734b42ac07301a9a))


### Bug Fixes

* elevated helper now finds pending commands without --request-id ([900b6bb](https://github.com/arcane-D7/micontrol/commit/900b6bb69fa4af515371ca1842c9abd5ed96724a))
* **hotkeys:** fix Raw Input buffer bug and add Copilot key interception ([7b37d77](https://github.com/arcane-D7/micontrol/commit/7b37d77dd382f7cf4e2da75e9df6d58ef54cd13a))
* **hotkeys:** use Win+P for F8 and elevated bridge for F7 performance mode ([d88cb0d](https://github.com/arcane-D7/micontrol/commit/d88cb0d0998cd7f31641d876c6bbe4ad6f5ee16d))
* keyring using MockCredential + consent never persisting ([1548105](https://github.com/arcane-D7/micontrol/commit/154810543b00242c65713d26e51577670c5415e5))
* remove Explorer restart from disable_copilot_key to fix tray ([4feb065](https://github.com/arcane-D7/micontrol/commit/4feb0658c4eac98fd0b21457fb58c696dd0213be))
* resolve 6 hardware control issues ([92dca3d](https://github.com/arcane-D7/micontrol/commit/92dca3dbc35bf1109adabd1a66bc9f50032ae110))
* **sprint-30:** resolve 4 P0 critical issues from Audit_Final ([b597d66](https://github.com/arcane-D7/micontrol/commit/b597d667f35b97385900a64845d4c2fda3952837))

## [Unreleased]

## [0.1.3] - 2026-07-03

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

- **ERAM region (0xFE0B0300) not accessible** — IoTDriver.sys hardcoded address ranges do not include ERAM. AC adapter wattage (ADPW at ERAM+0x81) cannot be read via driver. Use WMI as alternative.
- **SMA2 region (0xFE0B0A00) not accessible** — Same limitation as ERAM.
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
