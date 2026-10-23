# Sprint 7 — DevOps & CI/CD Foundation

## Sprint Metadata

| Field                 | Value                                                                                                        |
| --------------------- | ------------------------------------------------------------------------------------------------------------ |
| **Sprint Name**       | DevOps & CI/CD Foundation                                                                                    |
| **Sprint Goal**       | Establish a CI pipeline, linting/formatting, a signed release workflow, and single-source version management |
| **Duration Estimate** | 2 weeks (10 working days)                                                                                    |
| **Priority**          | P1 — Foundational for all future sprints' quality gates.                                                     |
| **Sprint Type**       | DevOps / Infrastructure                                                                                      |
| **Primary Owner**     | DevOps engineer                                                                                              |
| **Secondary Owner**   | Release manager                                                                                              |

## Sprint Goal Statement

There is currently no CI/CD pipeline, no linting or formatting, no defined release/signing workflow, and the version is tracked in 3 places with no sync. By the end of this sprint, every pull request runs `cargo check`, `cargo clippy`, `cargo test`, `npm run build`, `eslint`, and `prettier` in CI; releases are built and signed via a documented workflow; and the version is defined in one place and propagated to `package.json`, `Cargo.toml`, and `tauri.conf.json`.

---

## Background

One critical DevOps finding (D1: no CI/CD) and three high findings: (D2) no linting/formatting, (D3) updater release process undefined with no signing, (D4) version tracked in 3 places. These gaps mean code quality is unenforced, releases are manual and unsigned, and version drift is inevitable.

---

## Tickets

### S7-001 — Create a CI/CD pipeline for pull requests

| Field                | Value                                                                       |
| -------------------- | --------------------------------------------------------------------------- |
| **Ticket ID**        | S7-001                                                                      |
| **Title**            | Set up GitHub Actions CI pipeline running build, lint, and test on every PR |
| **Priority**         | P0                                                                          |
| **Type**             | DevOps                                                                      |
| **Estimated Effort** | L                                                                           |

#### Description

No CI/CD pipeline exists. Every change is validated only locally, if at all. This ticket creates a GitHub Actions workflow that runs on every pull request and push to `main`, executing the full quality gate: Rust check/clippy/test, frontend build/lint, and (optionally) a Tauri build smoke test.

#### Affected Files and Line Ranges

- New: `.github/workflows/ci.yml`.
- Possibly: `src-tauri/Cargo.toml` (ensure test configuration), `package.json` (ensure scripts exist).

#### Root Cause Analysis

Without CI, broken code merges to `main` regularly, linting is inconsistent across contributors, and tests may be skipped. A CI pipeline enforces the quality bar on every change, catching regressions before merge.

#### Acceptance Criteria

- [x] A `.github/workflows/ci.yml` workflow triggers on `pull_request` and `push` to `main`.
- [x] The workflow runs on Windows (primary target OS) — `runs-on: windows-latest`.
- [x] Job 1 (Rust): `cargo check --manifest-path src-tauri/Cargo.toml`, `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`, `cargo test --manifest-path src-tauri/Cargo.toml`.
- [x] Job 2 (Frontend): `npm ci`, `npm run build`, `npm run lint` (eslint), `npm run format:check` (prettier).
- [x] Job 3 (Tauri smoke, optional/nightly): `npm run tauri build` to confirm the full app builds.
- [x] Dependencies are cached (`actions/cache` for `~/.cargo` and `node_modules`).
- [x] The workflow fails on any error and blocks merge (branch protection rule documented).
- [x] A README section documents how to run the same checks locally.
- [x] Manual test: open a PR with a deliberate lint error; confirm CI fails. Open a PR with a clean change; confirm CI passes.

#### Implementation Notes

- Use `actions/cache` with `Cargo.lock` and `package-lock.json` as keys.
- For Tauri builds on Windows, ensure the runner has the Windows SDK; the `windows-latest` image includes most prerequisites.
- Keep the Tauri build as a separate job (or nightly) to keep PR CI fast.
- Document the branch protection setup (require status checks before merge).

#### Testing Strategy

- **PR test**: deliberate failure (lint error, failing test) → CI fails.
- **PR test**: clean change → CI passes.
- **Cache effectiveness**: confirm second run is faster (cache hit).

#### Dependencies

- S7-002 (linting config must exist for CI to run lint).

---

### S7-002 — Configure linting and formatting for Rust and TypeScript

| Field                | Value                                                   |
| -------------------- | ------------------------------------------------------- |
| **Ticket ID**        | S7-002                                                  |
| **Title**            | Add clippy, rustfmt, eslint, and prettier configuration |
| **Priority**         | P0                                                      |
| **Type**             | DevOps                                                  |
| **Estimated Effort** | M                                                       |

#### Description

No linting or formatting is configured: no `clippy` lints, no `rustfmt.toml`, no `eslint` config, no `prettier` config. Code style is inconsistent and common bugs (unused vars, etc.) go uncaught. This ticket adds and configures all four tools.

#### Affected Files and Line Ranges

- New: `src-tauri/rustfmt.toml`, `src-tauri/clippy.toml` (or `Cargo.toml` `[lints]` section).
- New: `.eslintrc.*` (or `eslint.config.js` for flat config), `.prettierrc`.
- `package.json` — add `lint`, `format`, `format:check` scripts.
- `src-tauri/Cargo.toml` — add `[lints.clippy]` or a `clippy` config.

#### Root Cause Analysis

Without linting, the codebase accumulates style inconsistencies and avoidable bugs. Without formatting, PRs contain noise from reformatting. The tools exist in the ecosystem but were never configured for this project.

#### Acceptance Criteria

- [x] `rustfmt.toml` is added with a consistent style (e.g. `edition = "2021"`, `max_width = 100`).
- [x] `cargo fmt --check` passes on the current codebase (run `cargo fmt` once to normalize).
- [x] `clippy` is configured with `-D warnings` (deny all warnings) and the current codebase passes (fix existing warnings or allow with justification).
- [x] `eslint` is configured with a recommended ruleset (e.g. `@typescript-eslint/recommended` + React plugins) and the current codebase passes.
- [x] `prettier` is configured (`.prettierrc`) and `prettier --check` passes (run `prettier --write` once to normalize).
- [x] `package.json` has scripts: `"lint": "eslint src"`, `"format": "prettier --write src"`, `"format:check": "prettier --check src"`.
- [x] A pre-commit hook (via `husky` + `lint-staged`, optional but recommended) runs `cargo fmt`, `clippy`, `eslint --fix`, and `prettier` on staged files.
- [x] Manual test: introduce a lint violation; confirm `npm run lint` and `cargo clippy` catch it.

#### Implementation Notes

- Normalizing the existing codebase may produce a large initial diff — do this in a dedicated "style normalization" commit before enabling the checks.
- For clippy, start with the default warning set; add `-D warnings` once existing warnings are resolved.
- Use `lint-staged` to run checks only on changed files for speed.

#### Testing Strategy

- **Manual test**: introduce violations; confirm tools catch them.
- **CI integration**: S7-001 runs these checks on every PR.

#### Dependencies

- None (foundational; S7-001 depends on this).

---

### S7-003 — Define and document the release signing workflow

| Field                | Value                                                                    |
| -------------------- | ------------------------------------------------------------------------ |
| **Ticket ID**        | S7-003                                                                   |
| **Title**            | Create a signed release workflow with code signing for the Tauri updater |
| **Priority**         | P1                                                                       |
| **Type**             | DevOps                                                                   |
| **Estimated Effort** | L                                                                        |

#### Description

The updater release process is undefined — there is no signing workflow, so updates cannot be verified and the Tauri updater's signature check is either disabled or will reject updates. This ticket defines and implements a release workflow that builds and signs the app and update artifacts.

#### Affected Files and Line Ranges

- New: `.github/workflows/release.yml`.
- `src-tauri/tauri.conf.json` — updater configuration (signature public key).
- Documentation: `docs/release.md`.

#### Root Cause Analysis

Tauri's updater requires signed update bundles (the updater verifies a signature against a public key bundled with the app). Without a signing workflow, either updates are unsigned (insecure) or the updater is non-functional. The release process being "undefined" means releases are ad-hoc and unrepeatable.

#### Acceptance Criteria

- [x] A `.github/workflows/release.yml` triggers on tag push (e.g. `v*.*.*`).
- [x] The workflow builds the Tauri app for Windows (`npm run tauri build`).
- [x] The build artifacts (MSIX/NSIS installer + updater bundle) are signed with a code-signing certificate.
- [x] The certificate is stored as a GitHub Actions secret (not in the repo); the signing step uses the secret.
- [x] The Tauri updater's signature public key is configured in `tauri.conf.json` and matches the signing key.
- [x] The signed update bundle is uploaded to a release (GitHub Releases or a designated update server).
- [x] A `docs/release.md` documents: how to cut a release (tag format), how the signing key is managed, and how to rotate keys.
- [x] Manual test (dry run): trigger the workflow on a test tag; confirm artifacts are built and signed; verify the signature locally.
- [x] The signing certificate's expiry is tracked (document the expiry date and rotation plan).

#### Implementation Notes

- For initial setup, a self-signed certificate may be used for testing; document that production requires a trusted CA-signed certificate (e.g. from a vendor).
- Tauri's updater uses a minisign/Ed25519 keypair for update signatures, separate from the Windows code-signing certificate. Configure both.
- Store secrets in GitHub Actions encrypted secrets; never commit keys.
- Consider a release-drafter or changelog generation step.

#### Testing Strategy

- **Dry-run release**: trigger on a test tag; verify build + sign + upload.
- **Signature verification**: use Tauri's updater tools to verify the signature locally.
- **Documentation review**: confirm `docs/release.md` is complete and actionable.

#### Dependencies

- S7-001 (CI infrastructure exists).
- S7-004 (version is single-sourced so the release tag matches).

---

### S7-004 — Single-source version management

| Field                | Value                                                                                       |
| -------------------- | ------------------------------------------------------------------------------------------- |
| **Ticket ID**        | S7-004                                                                                      |
| **Title**            | Define the version in one place; propagate to package.json, Cargo.toml, and tauri.conf.json |
| **Priority**         | P1                                                                                          |
| **Type**             | DevOps                                                                                      |
| **Estimated Effort** | S                                                                                           |

#### Description

The version is tracked in 3 places — `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json` — with no sync mechanism. Version drift causes the updater, the about dialog, and the build to disagree. This ticket establishes a single source of truth and a propagation script.

#### Affected Files and Line Ranges

- `package.json` — `version` field.
- `src-tauri/Cargo.toml` — `version` field.
- `src-tauri/tauri.conf.json` — `version` field.
- New: a version-sync script (e.g. `scripts/sync-version.js` or a `cargo`/`npm` tool).

#### Root Cause Analysis

Each file independently tracks the version because they belong to different toolchains (npm, cargo, tauri). Without a sync step, bumping one and forgetting the others causes drift. The updater checks the version against the server; a mismatch can cause skipped or repeated updates.

#### Acceptance Criteria

- [x] A single source of truth is chosen (recommend `package.json` `version`, or a dedicated `VERSION` file).
- [x] A sync script reads the source version and writes it to `Cargo.toml` and `tauri.conf.json`.
- [x] The sync script runs automatically in the release workflow (S7-003) before building.
- [x] A `pre-commit` or CI check verifies all three files agree; if they disagree, CI fails with a clear message.
- [x] A `npm run version:bump <new-version>` (or equivalent) script bumps the source and runs sync.
- [x] Manual test: bump the version via the script; confirm all three files updated.
- [x] CI test: deliberately desync the files; confirm CI fails.

#### Implementation Notes

- A simple Node script using `fs` + regex/JSON parse to read `package.json` and write the others is sufficient.
- Alternatively, use a tool like `tauri-plugin-version` or a `cargo` workspace version, but a custom script is the lowest-friction option.
- Add the sync check to the CI workflow (S7-001) as an additional step.

#### Testing Strategy

- **Manual test**: bump version, verify all files.
- **CI test**: desync, verify failure.

#### Dependencies

- S7-001 (CI runs the sync check).

---

## Sprint Exit Criteria

- [x] All 4 tickets merged.
- [x] CI pipeline runs on every PR with build + lint + test.
- [x] `cargo fmt --check`, `cargo clippy -D warnings`, `eslint`, `prettier --check` all pass on `main`.
- [x] A dry-run release produces signed artifacts.
- [x] Version is single-sourced; CI enforces sync.
- [x] `docs/release.md` is complete.

## Risks & Mitigations

| Risk                                                         | Mitigation                                                           |
| ------------------------------------------------------------ | -------------------------------------------------------------------- |
| Clippy/eslint normalization produces huge diff               | Isolate in a dedicated commit before enabling checks.                |
| Code-signing certificate acquisition delays release workflow | Use self-signed for testing; document CA requirement for production. |
| CI is slow, discouraging PRs                                 | Cache aggressively; keep Tauri build as nightly.                     |
| Version sync script breaks on format changes                 | Add tests for the script; pin file formats.                          |
