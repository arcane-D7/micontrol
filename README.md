# miPC

> Desktop hardware control for gaming laptops — fan curves, battery, display, audio, and more.

[![CI](https://img.shields.io/github/actions/workflow/status/Freitas-MA/miPC/ci.yml?branch=main&style=flat-square)](https://github.com/Freitas-MA/miPC/actions)
[![License](https://img.shields.io/github/license/Freitas-MA/miPC?style=flat-square)](LICENSE)
[![Version](https://img.shields.io/github/v/release/Freitas-MA/miPC?style=flat-square)](https://github.com/Freitas-MA/miPC/releases)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen?style=flat-square)](CONTRIBUTING.md)

---

## Features

- **Hardware Control** — Manage fan curves, battery charge thresholds, display brightness & HDR, audio volume & devices, keyboard backlight, and touchpad settings.
- **IoT Service Integration** — Communicate with the embedded controller via EC RAM access, handle hotkeys, and cast the screen wirelessly.
- **Driver Management** — Scan, install, and update hardware drivers with guided workflows.
- **System Info Dashboard** — Real-time CPU, GPU, RAM, and storage monitoring at a glance.
- **AI-Powered Analysis** — Optional AI system advisor that analyses your hardware logs and provides personalised recommendations for thermal management, performance modes, and battery health. Supports OpenAI, Ollama, and any OpenAI-compatible provider. See [AI Features Documentation](docs/ai-features.md) for details on data handling, privacy, and supported models.
- **Privacy-First** — All data stays local by default. Telemetry requires explicit opt-in via the consent audit log. Every privileged operation is logged and integrity-verified with HMAC.
- **Multi-Language** — Available in English, Portuguese, Spanish, and French.

---

## Installation

### Download a Release

Grab the latest NSIS installer from the [Releases page](https://github.com/Freitas-MA/miPC/releases). No additional runtime is required.

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
git clone https://github.com/Freitas-MA/miPC.git
cd miPC

# Install frontend dependencies
npm install

# Run in development mode
npm run tauri dev

# Build for production
npm run tauri build
```

#### Running Checks

```bash
# Rust
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo fmt --check --manifest-path src-tauri/Cargo.toml

# Frontend
npm ci
npx tsc --noEmit
npm run lint
npm run format:check
npm run build
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
│          WMI ─ Registry ─ HID ─ EC RAM           │
├───────────────────────┼──────────────────────────┤
│                  Windows 10/11                    │
└─────────────────────────────────────────────────┘
```

- **Frontend** — React 19 with TypeScript, Vite, and Tailwind CSS. Tab-based UI with lazy-loaded pages.
- **Backend** — Rust modules organized by hardware domain (`hw/battery.rs`, `hw/display.rs`, etc.), exposed via Tauri command handlers.
- **Elevated Bridge** — A secure subprocess for privileged operations (driver installs, EC RAM access). Every request is HMAC-signed, nonce-protected against replay, and logged to an integrity-verified audit trail.

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
npm run tauri build
```

These checks run automatically in CI on every pull request.

### Pre-commit Hooks

This project uses [husky](https://typicode.github.io/husky/) and [lint-staged](https://github.com/lint-staged/lint-staged) to run pre-commit checks.

When you first clone the repository, run:

```bash
npm install
```

This will automatically install the husky pre-commit hook. On every commit, lint-staged will:

- Run `eslint --fix` and `prettier --write` on staged TypeScript/JavaScript files
- Run `prettier --write` on staged JSON, CSS, and Markdown files
- Run `rustfmt` on staged Rust files

If any check fails, the commit will be aborted. Fix the issues and try again.
