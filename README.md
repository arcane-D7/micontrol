# miPC

> Desktop hardware control for gaming laptops — fan curves, battery, display, audio, and more.

[![CI](https://img.shields.io/github/actions/workflow/status/arcane-D7/micontrol/ci.yml?branch=main&style=flat-square)](https://github.com/arcane-D7/micontrol/actions)
[![License](https://img.shields.io/github/license/arcane-D7/micontrol?style=flat-square)](LICENSE)
[![Version](https://img.shields.io/github/v/release/arcane-D7/micontrol?style=flat-square)](https://github.com/arcane-D7/micontrol/releases)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen?style=flat-square)](CONTRIBUTING.md)

---

## Features

- **Hardware Control** — Manage fan curves, battery charge thresholds, display brightness & HDR, audio volume & devices, and touchpad settings; read the keyboard backlight level from EC RAM.
- **Overview Dashboard** — One screen with the machine's identity (model, CPU, GPU, NPU, RAM, disk, panel), live temperatures with their source, memory/storage health, battery state, and shortcuts into every other tab.
- **Task Manager** — Process list sortable by CPU, GPU, NPU, RAM, name or PID, with per-process detail and a guarded kill action.
- **System Optimization (OS Turbo)** — Measured, reversible Windows tweaks grouped by risk tier, with a debloater accordion and a rollback path.
- **Startup Manager** — Every place Windows can launch something at sign-in in one list: `HKCU`/`HKLM` Run keys (64- and 32-bit views), per-user and common Startup folders, and logon scheduled tasks. Enable/disable uses the same `StartupApproved` blobs Task Manager writes; `HKLM` and common-folder entries go through the elevated bridge.
- **Color & Display Calibration** — Load/unload ICC profiles per display, browse and apply profiles from the catalog, launch the Windows calibration wizard, and toggle eye-protection (colour temperature).
- **System Cleanup** — Scan and remove caches/temp data by category, with a per-category report of what was freed and what failed.
- **Security Overview** — Windows Defender status and one-click access to Windows Security.
- **Cross-Device** — Phone Link status and quick actions, LocalSend file transfer, scrcpy screen mirroring, Syncthing folder sync, NFC pairing, BLE presence, and on-device transcription.
- **Face Unlock** — Optional Windows credential provider backed by a LocalSystem service (Rust, ONNX models) that unlocks the lock screen with the built-in RGB webcam. Built behind the `face` Cargo feature.
- **IoT Service Integration** — Communicate with the embedded controller via EC RAM access, handle hotkeys, and cast the screen wirelessly.
- **EC RAM Access** — Direct embedded controller RAM read/write via a custom `IoTService.exe` replacement binary that proxies IOCTLs to the Xiaomi `IoTDriver.sys` kernel driver. Uses named pipe IPC (`\\.\pipe\ecram_service`) with JSON protocol. See [RE Analysis Report](docs/RE_ANALYSIS_REPORT.md) for full reverse engineering details.
- **EC Command Protocol** — Full implementation of the 4-phase EC command state machine with 16 cmd_ids covering cloud binding status, WiFi provisioning, firmware/model queries, device ID, and laptop power status notifications. See [EC Command Protocol RE Report](docs/EC_COMMAND_PROTOCOL_RE.md) for protocol details.
- **Driver Management** — Scan, install, and update hardware drivers with guided workflows.
- **System Info Dashboard** — Real-time CPU, GPU, RAM, and storage monitoring at a glance.
- **AI-Powered Analysis** — Optional AI system advisor that analyses your hardware logs and provides personalised recommendations for thermal management, performance modes, and battery health. Supports OpenAI, Ollama, and any OpenAI-compatible provider. See [AI Features Documentation](docs/ai-features.md) for details on data handling, privacy, and supported models.
- **Privacy-First** — All data stays local by default. Telemetry requires explicit opt-in via the consent audit log. Every privileged operation is logged and integrity-verified with HMAC.
- **Multi-Language** — Available in English, Portuguese, Spanish, and French.

---

## Installation

### Download a Release

Grab the latest NSIS installer from the [Releases page](https://github.com/arcane-D7/micontrol/releases). No additional runtime is required.

### Build from Source

#### Prerequisites

| Tool        | Version | Notes                           |
| ----------- | ------- | ------------------------------- |
| Rust        | stable  | rustup default stable           |
| Node.js     | 24+     | pinned by `.nvmrc` (24.15.0)    |
| pnpm        | 11.5.0  | via Corepack (`packageManager`) |
| Windows SDK | 10.0+   | Included with Visual Studio     |

#### Steps

```bash
# Clone the repository
git clone https://github.com/arcane-D7/micontrol.git
cd miControl

# Install frontend dependencies (pnpm)
pnpm install

# Run in development mode
pnpm run tauri dev

# Build for production
pnpm run tauri build
```

#### Running Checks

The single source of truth for local and CI checks is `scripts/health-check.mjs`:

```bash
pnpm run health-check                 # every check
pnpm run health-check -- --list       # list the steps
pnpm run health-check -- --step=fmt,clippy,tsc
```

It runs version consistency, `cargo fmt --check`, `cargo clippy --all-targets --features face -D warnings`, `cargo test --features face`, `tsc --noEmit`, ESLint, Prettier, Vitest with coverage, the Vite build, the i18n completeness check, the kernel-handle leak gate and a toolchain consistency check. `ci.yml` and `release.yml` invoke this exact script, so the pipeline cannot drift from what you run locally.

The individual commands, if you need them:

```bash
# Rust
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --features face -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --features face
cargo fmt --check --manifest-path src-tauri/Cargo.toml

# Frontend
pnpm install --frozen-lockfile
npx tsc --noEmit
pnpm run lint
pnpm run format:check
pnpm run build
```

---

## Architecture Overview

miPC is a Tauri v2 desktop application with a React 19 frontend and a Rust backend.

```
┌─────────────────────────────────────────────────┐
│                  React 19 + TypeScript           │
│  ┌──────────┐  ┌──────────────┐  ┌───────────┐  │
│  │ Sidebar  │  │  TabContent  │  │  Tray UI  │  │
│  └────┬─────┘  └──────┬───────┘  └─────┬─────┘  │
│       └───────────────┼─────────────────┘        │
│                       ▼                          │
│              Custom Hooks (useHardware)           │
│                       │                          │
│              Tauri IPC (invoke)                   │
├───────────────────────┼──────────────────────────┤
│                 Rust Backend                      │
│  ┌──────────┐  ┌──────────────┐  ┌───────────┐  │
│  │Commands  │  │  hw/* (HAL)  │  │Elev Bridge│  │
│  └──────────┘  └──────┬───────┘  └───────────┘  │
│                       │                          │
│    WMI ─ Registry ─ HID ─ EC RAM (pipe)          │
│                       │                          │
│              Named Pipe IPC                       │
│           \\.\pipe\ecram_service                  │
├───────────────────────┼──────────────────────────┤
│           Custom IoTService.exe                   │
│         (IOCTL Proxy to IoTDriver.sys)            │
├───────────────────────┼──────────────────────────┤
│                  Windows 10/11                    │
└─────────────────────────────────────────────────┘
```

- **Frontend** — React 19 with TypeScript and Vite. Hand-written CSS with design tokens (`src/styles/`) — no CSS framework. Tab-based UI with lazy-loaded pages, grouped sidebar navigation and a custom lightweight i18n layer (`src/hooks/useI18n.ts`).
- **Backend** — Rust modules organized by hardware domain (`hw/battery.rs`, `hw/display.rs`, `hw/ecram.rs`, `hw/fan.rs`, `hw/wmi_ec.rs`, etc.), exposed via Tauri command handlers.
- **Elevated Bridge** — A secure subprocess for privileged operations (driver installs, EC RAM access). Every request is HMAC-signed, nonce-protected against replay, and logged to an integrity-verified audit trail.
- **EC RAM Service** — A custom `IoTService.exe` replacement binary (`src-tauri/src/bin/ecram_service.rs`) that proxies IOCTLs to the Xiaomi `IoTDriver.sys` kernel driver. Communicates with MiControl via named pipe IPC (`\\.\pipe\ecram_service`, JSON protocol). Required because the driver validates the calling process name and directory. See [RE Analysis Report](docs/RE_ANALYSIS_REPORT.md) for details.
- **EC Command Protocol** — The ecram_service binary implements the full 4-phase EC command state machine (RamIsReady → WriteCommand → ReadCmdAck → ReadCmdRet) with 16 cmd_ids for cloud binding, WiFi management, firmware/model queries, device ID, and laptop power status notifications. EC reset is performed before and after each command for reliability. See [EC Command Protocol RE Report](docs/EC_COMMAND_PROTOCOL_RE.md) for the complete protocol specification.

---

## Documentation

| Document                                                           | Description                                                                                                                                               |
| ------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [RE Analysis Report](docs/RE_ANALYSIS_REPORT.md)                   | Complete reverse engineering of IoTDriver.sys & IoTService.exe — IOCTLs, buffer layout, security check, allowed address ranges, custom replacement binary |
| [EC Command Protocol RE](docs/EC_COMMAND_PROTOCOL_RE.md)           | Reverse engineering of the 4-phase EC command state machine — 16 cmd_ids, ECRAM address map, per-feature response layouts, error codes                    |
| [Hardware Investigation](docs/HARDWARE_INVESTIGATION.md)           | Consolidated hardware findings — ACPI DSDT, WMI WMAA, EC RAM field map, hotkey events, IoTService IPC protocol                                            |
| [IoTService RE Analysis (Phase 1)](docs/iotservice-re-analysis.md) | Ghidra strings analysis of IoTService.exe — IPC commands, pipe protocol, source file mapping                                                              |
| [Architecture](docs/architecture.md)                               | System architecture overview, HAL module inventory, EC RAM access architecture, data flow                                                                 |
| [Frontend Architecture](docs/frontend-architecture.md)             | React 19 + TypeScript + Vite frontend design, component hierarchy, hooks, i18n                                                                            |
| [Adding a Hardware Feature](docs/adding-a-hardware-feature.md)     | Step-by-step guide for adding new HAL modules, includes WORKING FORM guidelines                                                                           |
| [AI Features](docs/ai-features.md)                                 | AI system advisor — data handling, privacy, supported models (OpenAI, Ollama)                                                                             |
| [Release Process](docs/release.md)                                 | Build, signing, publication, and EC RAM Service deployment                                                                                                |
| [Crash Reporting](docs/crash-reporting.md)                         | Sentry integration, privacy controls, consent-gated reporting                                                                                             |
| [Privacy Policy Versioning](docs/privacy-policy-versioning.md)     | Policy version tracking, HMAC consent audit log, credential-store consent (re-consent flow: known gap)                                                    |
| [Stability Report v4](docs/STABILITY_REPORT_v4.md)                 | Multi-agent audit — security, architecture, UI/UX, performance, AI responsibility (historical snapshot)                                                   |
| [Logging](docs/LOGGING.md)                                         | Where logs live, rotation (8 MiB × 7), lifecycle markers, WARN/ERROR de-duplication, how to read a bug report                                             |
| [Edge Slide Protocol RE](docs/edge-slide-protocol-re.md)           | Real touchpad edge-slide protocol — HID reports, registry flags, haptic tick, and why our routing was wrong                                               |
| [Installer Decoupling](docs/installer-decoupling.md)               | Static setup shell + versioned payload architecture, signing, install/update/uninstall flows                                                              |
| [Branch Protection](docs/branch-protection.md)                     | Required CI checks, review rules and CODEOWNERS for `main`                                                                                                |
| [Hardware Gap Analysis](docs/HARDWARE_GAP_ANALYSIS.md)             | Xiaomi PC Manager vs MiControl feature-by-feature comparison and implementation status                                                                    |
| [Face Unlock Plan](docs/FACE_UNLOCK_PLAN.md)                       | Credential-provider architecture, pipe protocol, liveness/recognition pipeline, build feature                                                             |
| [Docs Audit 2026-11-11](docs/DOCS-AUDIT-2026-11-11.md)             | Documentation-vs-code audit: per-file status, stale claims found, corrections applied                                                                     |

---

## Privacy & Security

miPC is designed with privacy and security as first-class concerns.

| Principle            | Implementation                                                                                                           |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| **Local-First**      | All hardware state, profiles, and logs stay on your machine. No cloud dependency.                                        |
| **Consent Audit**    | Every telemetry-capable operation is logged with a timestamp and HMAC integrity check.                                   |
| **Telemetry Opt-In** | No data leaves your PC without explicit consent. You can review and revoke consent at any time.                          |
| **Elevated Bridge**  | Privileged commands use a secure subprocess with HMAC signing, nonce replay protection, and per-request correlation IDs. |
| **No Telemetry**     | The application does not phone home unless you explicitly enable it.                                                     |

---

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

This project follows a [Code of Conduct](CODE_OF_CONDUCT.md) — be respectful, inclusive, and constructive.

### Pre-commit Hooks

This project uses [husky](https://typicode.github.io/husky/) and [lint-staged](https://github.com/lint-staged/lint-staged) to run pre-commit checks. `pnpm install` installs the hook automatically.

On every commit the hook (`.husky/pre-commit`) runs, in order:

1. `npx lint-staged` — `eslint --fix` + `prettier --write` on staged `src/**/*.{ts,tsx}`, `prettier --write` on staged `src/**/*.{json,css,md}`
2. `cargo fmt --manifest-path src-tauri/Cargo.toml` — formats the whole Rust workspace
3. `npx tsc --noEmit`
4. `pnpm run version:check`

If any check fails, the commit is aborted. Fix the issues and try again.

---

## License

[MIT](LICENSE) © miPC contributors

---

## Acknowledgments

- [Tauri](https://tauri.app/) — Desktop application framework
- [React](https://react.dev/) — UI library
- [Vite](https://vitejs.dev/) — Frontend tooling
- [GSAP](https://gsap.com/) and [three.js](https://threejs.org/) — landing-page motion and 3D
- [lucide](https://lucide.dev/) — Icon set
- [Sentry](https://sentry.io/) — Consent-gated crash reporting
- The open-source community for the tools and libraries that make this project possible
