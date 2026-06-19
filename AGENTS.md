# Repository Agent Rules — miPC/micontrol

## Project Type

This is a **Tauri v2 + React/TypeScript desktop application** with Rust backend and web frontend.

## Architecture

- `src-tauri/` — Rust backend, Tauri configuration, native APIs
- `src/` — Frontend React application (likely Vite-based)
- `index.html` — Entrypoint
- Uses **Vite** as frontend build tool (`vite.config.ts`)

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

## Do Not Assume

- Do not assume this repository is Brainiak.
- Do not apply Brainiak project structure here.
- Do not assume monorepo or workspace boundaries unless confirmed.