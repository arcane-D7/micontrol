# miPC

> Desktop hardware control for gaming laptops — fan curves, battery, display, audio, and more.

[![CI](https://img.shields.io/github/actions/workflow/status/arcane-D7/micontrol/ci.yml?branch=main&style=flat-square)](https://github.com/arcane-D7/micontrol/actions)
[![License](https://img.shields.io/github/license/arcane-D7/micontrol?style=flat-square)](LICENSE)
[![Version](https://img.shields.io/github/v/release/arcane-D7/micontrol?style=flat-square)](https://github.com/arcane-D7/micontrol/releases)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen?style=flat-square)](CONTRIBUTING.md)

---

## Features

- **Hardware Control** — Manage fan curves, battery charge thresholds, display brightness & HDR, audio volume & devices, keyboard backlight, and touchpad settings.
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

| Tool        | Version | Notes                       |
| ----------- | ------- | --------------------------- |
| Rust        | stable  | rustup default stable       |
| Node.js     | 20+     | LTS recommended             |
| Windows SDK | 10.0+   | Included with Visual Studio |

#### Steps

```bash
# Clone the repository
git clone https://github.com/arcane-D7/micontrol.git
cd miPC

# Install frontend dependencies (pnpm)
pnpm install

# Run in development mode
pnpm run tauri dev

# Build for production
pnpm run tauri build
```

#### Running Checks

```bash
# Rust
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
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

- **Frontend** — React 19 with TypeScript, Vite, and Tailwind CSS. Tab-based UI with lazy-loaded pages.
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
| [Privacy Policy Versioning](docs/privacy-policy-versioning.md)     | Policy version tracking, consent audit log, re-consent flow                                                                                               |
| [Stability Report v4](docs/STABILITY_REPORT_v4.md)                 | Multi-agent audit — security, architecture, UI/UX, performance, AI responsibility                                                                         |

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

---

## License

[MIT](LICENSE) © miPC contributors

---

## Acknowledgments

- [Tauri](https://tauri.app/) — Desktop application framework
- [React](https://react.dev/) — UI library
- [Vite](https://vitejs.dev/) — Frontend tooling
- [Tailwind CSS](https://tailwindcss.com/) — Utility-first CSS
- The open-source community for the tools and libraries that make this project possible

**Full Tauri build:**

```bash
pnpm run tauri build
```

These checks run automatically in CI on every pull request.

### Pre-commit Hooks

This project uses [husky](https://typicode.github.io/husky/) and [lint-staged](https://github.com/lint-staged/lint-staged) to run pre-commit checks.

When you first clone the repository, run:

```bash
pnpm install
```

This will automatically install the husky pre-commit hook. On every commit, lint-staged will:

- Run `eslint --fix` and `prettier --write` on staged TypeScript/JavaScript files
- Run `prettier --write` on staged JSON, CSS, and Markdown files
- Run `rustfmt` on staged Rust files

If any check fails, the commit will be aborted. Fix the issues and try again.
