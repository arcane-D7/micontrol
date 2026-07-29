# Repository Agent Rules — miPC/micontrol

## Project Type

This is a **Tauri v2 + React/TypeScript desktop application** with Rust backend and web frontend.

## Architecture

- `src-tauri/` — Rust backend, Tauri configuration, native APIs
- `src-tauri/src/hw/` — Hardware abstraction layer (HAL) modules:
  - `battery.rs` — Battery health & AC adapter via WMI
  - `display.rs` — Brightness & HDR
  - `ecram.rs` — EC RAM access via IoTDriver.sys IOCTLs + named pipe client for custom IoTService.exe. Includes `send_pipe_request()` generic pipe client for JSON protocol communication.
  - `iotservice.rs` — IoT device info queries using EC commands with fallback to registry/WMI/cached. Helper functions: `send_ec_pipe_command()`, `query_ec_string()`, `query_ec_device_id()`, `query_ec_bind_status()`, `query_ec_wifi_status()`, `query_ec_wifi_count()`.
  - `fan.rs` — Fan speed monitoring & performance mode via WMI
  - `wmi_ec.rs` — WMI-based EC read/write (MICommonInterface, root\WMI)
  - `wmi_cache.rs` — WMI connection caching with RefCell pattern
  - `thermal.rs`, `hotkeys/`, etc.
- `src-tauri/src/bin/ecram_service.rs` — Custom IoTService.exe replacement binary (Windows service + named pipe server + IOCTL proxy). Implements the full EC command protocol: 4-phase state machine (RamIsReady → WriteCommand → ReadCmdAck → ReadCmdRet) with 16 cmd_ids for cloud binding, WiFi provisioning, firmware/model queries, device ID, and laptop power status notifications. JSON pipe operations: `iot_get`, `iot_reset_device`, `iot_empty_wifi`, `iot_connect_wifi`, `iot_send_laptop_status`.
- `src/` — Frontend React application (Vite-based, 18 lazy-loaded tabs)
- `index.html` — Entrypoint
- Uses **Vite** as frontend build tool (`vite.config.ts`)

### Key Hardware Interfaces

| Interface                                    | Purpose                                                  | Status                                 |
| -------------------------------------------- | -------------------------------------------------------- | -------------------------------------- |
| WMI (`MICommonInterface`, root\WMI)          | Performance mode, battery health, fan RPM, adapter power | ✅ Working                             |
| IoTDriver.sys IOCTLs (`0x22E000`/`0x22E004`) | EC RAM read/write                                        | ✅ Working (via custom IoTService.exe) |
| Named pipe (`\\.\pipe\ecram_service`)        | IPC between MiControl and custom IoTService.exe          | ✅ Working                             |
| ERAM/SMA2 regions                            | AC adapter wattage, additional EC data                   | ❌ Not accessible (driver blocks)      |

### Reverse Engineering Documentation

- `docs/RE_ANALYSIS_REPORT.md` — Complete RE report (IoTDriver.sys IOCTLs, buffer layout, security check, allowed address ranges, custom replacement binary)
- `docs/EC_COMMAND_PROTOCOL_RE.md` — EC command protocol RE report (4-phase state machine, 16 cmd_ids, ECRAM address map, per-feature response layouts, error codes)
- `docs/HARDWARE_INVESTIGATION.md` — Consolidated hardware investigation (ACPI DSDT, WMI WMAA, EC RAM field map, hotkey events, IoTService IPC protocol)
- `docs/iotservice-re-analysis.md` — Phase 1 analysis (Ghidra strings, IPC command mapping, original IoTService.exe architecture)
- `docs/architecture.md` — System architecture overview with EC RAM access architecture diagram
- `docs/adding-a-hardware-feature.md` — Guide for adding new HAL modules, includes WORKING FORM guidelines

## Commands

Use commands defined in `package.json` and Tauri CLI. Common commands:

```bash
# Install frontend deps
npm install

# Dev (Vite + Tauri)
npm run tauri dev

# Build desktop app
npm run tauri build

# Frontend only
npm run dev
```

For Rust side, standard Cargo commands apply in `src-tauri/`:

```bash
# Check Rust
cargo check --manifest-path src-tauri/Cargo.toml

# Build Rust
cargo build --manifest-path src-tauri/Cargo.toml

# Build ecram_service.exe (custom IoTService.exe replacement)
cargo build --manifest-path src-tauri/Cargo.toml --release --bin ecram_service
```

## Validation

Before finishing executable code changes:

- If frontend changed: run `npm run build` (Vite build must pass).
- If Rust changed: run `cargo check --manifest-path src-tauri/Cargo.toml`.
- Tauri dev must start without runtime errors.

## Editing Rules

- Frontend vs backend separation: `src/` is web, `src-tauri/src/` is Rust.
- Use Tauri commands (invoke/handle) for IPC between frontend and backend.
- Do not bypass Tauri security model.
- Keep frontend framework-agnostic where possible (Tauri supports any web frontend).
- **WORKING FORM comments** — Code marked with `// WORKING FORM — DO NOT MODIFY` has been reverse-engineered and verified against the real hardware. Do NOT change the logic, API call patterns, or buffer layouts in these sections without re-testing against the actual driver/WMI interface. See `docs/RE_ANALYSIS_REPORT.md` for technical context.
- **ecram.rs pipe client** — The `read_ecram_via_pipe()`, `is_pipe_broker_available()`, and `send_pipe_request()` functions communicate with the custom `ecram_service.exe` binary via named pipe. The pipe name is `\\.\pipe\ecram_service`.
- **ecram_service.rs** — Must be named `IoTService.exe` when deployed, and placed in the IoTDriver DriverStore directory to pass the driver's security check. Implements the full EC command protocol with 16 cmd_ids. EC reset is performed before and after each command for reliability.
- **iotservice.rs** — Uses EC commands via pipe to query IoT device info. Falls back to registry/WMI/cached data when EC commands fail. The IoT chip's WiFi is separate from Windows WiFi — when the chip is not bound to the cloud, IoT WiFi queries return timeout (expected behavior).

## Do Not Assume

- Do not assume this repository is Brainiak.
- Do not apply Brainiak project structure here.
- Do not assume monorepo or workspace boundaries unless confirmed.
