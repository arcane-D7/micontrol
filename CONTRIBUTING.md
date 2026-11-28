# Contributing to miPC

Thank you for your interest in contributing to miPC! This document outlines the process for contributing to the project.

## Development Setup

### Prerequisites

- **Node.js** 24+ (see `.nvmrc`, pinned to 24.15.0) and **pnpm** 11.5.0 (via Corepack)
- **Rust** stable toolchain (install via [rustup](https://rustup.rs/))
- **Windows 10/11** (this is a Windows-only application)
- **Visual Studio Build Tools** with C++ workload
- **Windows SDK** (for Windows API access)

### Getting Started

1. Clone the repository:

   ```bash
   git clone https://github.com/arcane-D7/micontrol.git
   cd micontrol
   ```

2. Install frontend dependencies:

   ```bash
   pnpm install
   ```

3. Run the development server:
   ```bash
   pnpm run tauri dev
   ```

### Project Structure

```
micontrol/
├── src/                    # React frontend (TypeScript)
│   ├── components/          # Reusable UI components
│   ├── hooks/              # Custom React hooks
│   ├── pages/              # Window shells (MainWindow, TrayPopup) + tabs/ (24 lazy-loaded)
│   ├── i18n/               # Internationalization (en/pt/es/fr JSON)
│   └── styles/             # CSS and design tokens
├── src-tauri/              # Rust backend (Tauri v2)
│   ├── src/
│   │   ├── hw/             # Hardware abstraction layer (HAL, ~55 modules)
│   │   │   ├── battery.rs      # Battery health & AC adapter (WMI)
│   │   │   ├── ecram.rs        # EC RAM access (IOCTL + pipe client)
│   │   │   ├── fan.rs          # Fan speed & performance mode (WMI)
│   │   │   ├── wmi_ec.rs       # WMI EC read/write (MICommonInterface)
│   │   │   └── wmi_cache.rs    # WMI connection caching
│   │   ├── bin/
│   │   │   ├── ecram_service.rs    # Custom IoTService.exe replacement
│   │   │   ├── micontrol_bridge.rs # SYSTEM bridge service (updates, elevated ops)
│   │   │   └── micontrol_face_svc.rs # Face Unlock service (`face` feature)
│   │   ├── commands/       # Tauri command handlers (ai, hardware, system, privacy, …)
│   │   ├── util/           # Utility modules (logging, consent audit, crash report, …)
│   │   ├── elevated.rs     # Elevated helper (--elevated)
│   │   └── lib.rs          # Application entry point
│   ├── shell/              # Static installer shell crate
│   └── Cargo.toml          # Rust dependencies
├── .github/workflows/      # CI/CD pipelines
└── docs/                   # Documentation
```

### Development Workflow

1. Create a feature branch: `git checkout -b feature/your-feature`
2. Make your changes
3. Run the health check (the same script CI runs):
   ```bash
   pnpm run health-check
   ```
   Or the individual steps:
   ```bash
   cargo fmt --manifest-path src-tauri/Cargo.toml --check
   cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --features face -- -D warnings
   cargo test --manifest-path src-tauri/Cargo.toml --features face
   npx tsc --noEmit
   pnpm run lint
   pnpm run build
   ```
4. Commit with a conventional commit message
5. Push and create a pull request

### Commit Message Convention

We use [Conventional Commits](https://www.conventionalcommits.org/):

- `feat:` New feature
- `fix:` Bug fix
- `docs:` Documentation only
- `refactor:` Code refactoring
- `test:` Adding tests
- `chore:` Build/tooling changes

### Code Style

#### Rust

- Run `cargo fmt` before committing
- Zero clippy warnings allowed (`-D warnings`)
- Add tests for new hardware modules
- Use `HardwareResult<T>` for hardware functions

#### TypeScript/React

- Run `pnpm run lint` before committing
- Use functional components with hooks
- Add TypeScript types for all props
- Use the i18n system for all user-facing strings

### Hardware Modules & Reverse Engineering

Some hardware modules contain code marked with `// WORKING FORM — DO NOT MODIFY`. These sections have been reverse-engineered and verified against the real Xiaomi IoTDriver.sys kernel driver and WMI interface. **Do not change the logic, API call patterns, or buffer layouts** in these sections without re-testing against the actual hardware.

Key resources:

- `docs/RE_ANALYSIS_REPORT.md` — Complete RE report (IOCTLs, buffer layout, security check, allowed ranges)
- `docs/HARDWARE_INVESTIGATION.md` — Consolidated hardware investigation findings
- `docs/iotservice-re-analysis.md` — IoTService.exe IPC protocol and string analysis

The custom `ecram_service.rs` binary (`src-tauri/src/bin/ecram_service.rs`) is a replacement for Xiaomi's IoTService.exe that proxies ECRAM IOCTLs to the kernel driver via named pipe IPC. When deployed, it must be named `IoTService.exe` and placed in the DriverStore directory to pass the driver's security check.

### Testing

- Rust tests: `cargo test --manifest-path src-tauri/Cargo.toml --features face`
- Frontend tests: `pnpm test` (Vitest)
- Integration tests: `cargo test --manifest-path src-tauri/Cargo.toml --features face --test '*'`
- Everything at once: `pnpm run health-check`

### Pull Request Process

1. Ensure all health checks pass
2. Update documentation if needed
3. Add a changelog entry
4. Request review from maintainers

## Questions?

Feel free to open an issue for any questions about contributing.
