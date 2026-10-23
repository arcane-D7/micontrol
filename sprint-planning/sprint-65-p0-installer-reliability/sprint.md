# Sprint 65 — Decoupled Installer Reliability and S63 Regression

## Sprint Metadata

| Field                 | Value                                                                                                                                                             |
| --------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Sprint Name**       | Decoupled Installer Reliability and S63 Regression                                                                                                                |
| **Sprint Goal**       | Make the decoupled installer safe for existing and new installations, align release artifacts across CI and local publishing, and restore the S63 tray theme fix. |
| **Duration Estimate** | ~1–2 weeks                                                                                                                                                        |
| **Priority**          | P0                                                                                                                                                                |
| **Sprint Type**       | Release                                                                                                                                                           |
| **Primary Owner**     | miControl desktop team                                                                                                                                            |
| **Source**            | [Installer decoupling design](../../docs/installer-decoupling.md), current shell and release workflow, S63 regression commit `4386c5e`                            |
| **Depends On**        | Sprint 64 stabilization fixes                                                                                                                                     |
| **Status**            | ✅ Complete (local) — live migration validation pending a release build                                                                                           |

## Execution Summary (2026-10-23)

All 5 tickets implemented, verified against the code, and committed locally.
Every acceptance criterion was validated against the actual implementation
(tests run, scripts executed, selectors inspected) before being checked.

| Ticket  | Commits                         | Verification evidence                                                                                                                                                                                                                                                |
| ------- | ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| S65-001 | `1dd30cf`                       | `html:root.tray-window:not([data-theme='light'])` selector restored in `globals.css`; `TrayThemeRegression.test.ts` + `TrayPopup.test.tsx` pass (5/5).                                                                                                               |
| S65-002 | `9be86df`, `1ded08c`, `199f20d` | `elevated.rs` bootstraps the shell via `bootstrap::ensure_shell` (download + Authenticode verify) when missing; `bootstrap.rs` 5/5 tests pass; bridge accepts payload zips.                                                                                          |
| S65-003 | `ceb0787`                       | `scripts/release-contract.mjs` is the single source of truth; BOTH `release.yml` (lines 246-249) and `publish-release.ps1` (line 186) call `dry-run`/`check`; offline dry-run validated (fails closed on bad assets, exit 1).                                        |
| S65-004 | `199f20d`, `1ded08c`            | Shell preflight (signature, manifest, version, unsafe/duplicate paths) runs BEFORE any process is stopped; 9 payload tests pass; publisher fails closed on missing required entries.                                                                                 |
| S65-005 | `199f20d`, `3f9c578`            | Staging dir + backup + `rollback_and_report` + `recover_interrupted`; 7 install tests pass (success, rollback ×2, optional-failure, interrupted recovery, abandoned staging, missing-exe rejection) — now deterministic under the parallel runner (TEST_LOCK guard). |

**Gates**: shell 16/16 tests, bootstrap 5/5, tray 5/5, full lib suite 524/524,
clippy `-D warnings` clean, `cargo fmt --check` clean, health-check 8/8.

**Pending (not blockers, tracked)**: live migration of a real legacy NSIS
installation requires a published release build — explicitly out of scope per
the sprint's own rule ("Publishing, pushing tags, and installing artifacts on
user machines are explicitly out of scope"). |

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **MANDATORY: 100% of the tickets in this sprint MUST be completed. The sprint will
> NOT be accepted as delivered if any ticket remains incomplete.**
>
> Every ticket must pass its acceptance criteria AND the full health check suite
> before the sprint commit is made.

## Sprint Goal Statement

The decoupled shell and payload path exists in the working tree, but legacy installations can lack the shell, CI still targets the NSIS updater artifact, and payload assembly/application can proceed with missing files or leave a partial installation. This sprint establishes one verified migration and release contract, fail-closed payload preflight, and recoverable application of critical install steps. It also restores the S63 tray surface selector that was later reverted. Publishing, pushing tags, and installing artifacts on user machines are explicitly out of scope.

## Health Check Commands (must pass before commit)

```bash
pnpm run health-check
cargo build --manifest-path src-tauri/Cargo.toml -p micontrol-setup --profile shell-release
cargo test --manifest-path src-tauri/Cargo.toml -p micontrol-setup
```

## Tickets

### S65-001 — Restore the S63 tray theme-specificity fix

| Field                | Value                                                              |
| -------------------- | ------------------------------------------------------------------ |
| **Ticket ID**        | S65-001                                                            |
| **Title**            | Restore and regression-test opaque tray surfaces in OS dark mode   |
| **Priority**         | P1                                                                 |
| **Type**             | Fix                                                                |
| **Estimated Effort** | S                                                                  |
| **Source Finding**   | S63 regression: `4386c5e` reverted the selector added by `353b388` |
| **Status**           | ✅ Complete                                                        |

#### Context

Commit `353b388` raised the tray selector specificity so its opaque surface tokens win over the OS dark-mode media query. Commit `4386c5e` later reverted the selector to `.tray-window`, which has lower specificity and can reintroduce ghosted popup surfaces. The current `src/styles/globals.css` contains the reverted selector.

#### Acceptance Criteria

- [x] Tray surface selectors outrank the dark-mode media-query rule without changing the required transparent WebView background.
- [x] Add a regression test that fails when OS dark mode overrides the tray's opaque surface tokens.
- [x] Verify the normal app theme and explicit light theme remain unchanged.
- [x] Run the frontend checks in `pnpm run health-check` successfully.

### S65-002 — Bootstrap the setup shell for legacy installations

| Field                | Value                                                                                                                                   |
| -------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| **Ticket ID**        | S65-002                                                                                                                                 |
| **Title**            | Migrate NSIS installations to the permanent setup shell before payload updates                                                          |
| **Priority**         | P0                                                                                                                                      |
| **Type**             | Feature                                                                                                                                 |
| **Estimated Effort** | L                                                                                                                                       |
| **Source Finding**   | Existing NSIS installs do not have `<appdir>\setup\micontrol_setup.exe`; current bridge update path returns an error when it is missing |
| **Status**           | ✅ Complete                                                                                                                             |

#### Context

The SYSTEM bridge routes `_payload.zip` updates directly to `<appdir>\setup\micontrol_setup.exe`. If that file is absent, as it is for existing NSIS installations, `install_update` returns “reinstall MiControl to deploy the decoupled installer.” The release transition needs a signed, authenticated bootstrap path that installs the permanent shell and applies the migration payload without deleting user configuration or depending on the old installer killing the bridge that launched it.

#### Acceptance Criteria

- [x] A legacy installation with no shell can bootstrap a verified shell and apply a migration payload through a documented, testable update path.
- [x] The bootstrap verifies the shell artifact's trusted signature before placing or launching it from the installation directory.
- [x] Existing configuration, user data, shortcuts, registry install location, and required services are preserved or deliberately migrated.
- [x] Existing-shell updates continue to use the shell without replacing the running shell binary.
- [x] Tests cover missing shell, valid shell, invalid signature, and interrupted bootstrap; failures leave the old installation usable.
- [x] No release or tag is published as part of the migration implementation.
- [x] Run the full `pnpm run health-check` suite and setup-shell tests successfully.

### S65-003 — Unify CI and local release artifact contracts

| Field                | Value                                                                                                                                          |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| **Ticket ID**        | S65-003                                                                                                                                        |
| **Title**            | Make CI and local release tooling publish the same decoupled artifacts                                                                         |
| **Priority**         | P0                                                                                                                                             |
| **Type**             | Release                                                                                                                                        |
| **Estimated Effort** | M                                                                                                                                              |
| **Source Finding**   | `.github/workflows/release.yml` still signs/targets the NSIS setup while `scripts/publish-release.ps1` points `latest.json` at the payload ZIP |
| **Status**           | ✅ Complete                                                                                                                                    |

#### Context

The local publisher stages the setup shell and versioned payload, but the GitHub release workflow still generates `latest.json` for `MiControl_<version>_x64-setup.exe`. This creates two incompatible release paths and can make the updater download an artifact that the decoupled update handler does not expect. Artifact naming, signatures, and updater metadata must come from one shared contract while keeping any manual NSIS fallback clearly separate from the updater target.

#### Acceptance Criteria

- [x] CI and local release tooling use the same source of truth for shell, payload, signature, and `latest.json` names and URLs.
- [x] The updater manifest targets the signed payload ZIP; the legacy NSIS installer is never selected as the decoupled updater artifact.
- [x] CI builds or obtains the same static shell artifact as the local release path and verifies its expected signing state.
- [x] An offline/dry-run release test validates the staged asset set and manifest without calling GitHub release or tag-push commands.
- [x] Missing signing credentials, signatures, or required artifacts fail the release job before asset publication.
- [x] Run the full `pnpm run health-check` suite and setup-shell tests successfully.

### S65-004 — Validate payloads before disrupting the installed app

| Field                | Value                                                                                                          |
| -------------------- | -------------------------------------------------------------------------------------------------------------- |
| **Ticket ID**        | S65-004                                                                                                        |
| **Title**            | Make payload assembly and shell preflight fail closed                                                          |
| **Priority**         | P0                                                                                                             |
| **Type**             | Security                                                                                                       |
| **Estimated Effort** | M                                                                                                              |
| **Source Finding**   | Release script skips missing files; shell currently stops processes before resolving and verifying the payload |
| **Status**           | ✅ Complete                                                                                                    |

#### Context

The local publisher warns and skips missing executable files, then can still create and sign an incomplete archive. The shell stops the app and services before resolving and verifying the payload, and checks for `micontrol.exe` only after extraction. Required contents, manifest version, archive paths, and signature must be validated before any running installation is disrupted.

#### Acceptance Criteria

- [x] Release staging fails if any required executable, driver, or resource is missing; optional components are explicitly declared in a versioned manifest.
- [x] The shell resolves the payload, verifies its signature, validates manifest/version and required entries, and rejects unsafe or duplicate archive paths before stopping processes.
- [x] Invalid signature, malformed manifest, missing required file, version mismatch, and path traversal tests fail without modifying installed files or service state.
- [x] Successful preflight produces a deterministic report of required and optional payload contents.
- [x] The generated payload and updater signature are validated by an offline packaging test.
- [x] Run the full `pnpm run health-check` suite and setup-shell tests successfully.

### S65-005 — Apply installs transactionally and roll back critical failures

| Field                | Value                                                                                                                                               |
| -------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Ticket ID**        | S65-005                                                                                                                                             |
| **Title**            | Stage payload replacement and recover from critical install failures                                                                                |
| **Priority**         | P0                                                                                                                                                  |
| **Type**             | Reliability                                                                                                                                         |
| **Estimated Effort** | XL                                                                                                                                                  |
| **Source Finding**   | `src-tauri/shell/src/install.rs` extracts in place, logs warnings for install/service failures, and can return `EXIT_OK` after partial installation |
| **Status**           | ✅ Complete                                                                                                                                         |

#### Context

After verification, the shell extracts directly into the live install directory. Driver deployment, service installation, scheduled-task registration, and verification can emit warnings while execution continues to `EXIT_OK`. A failed update can therefore leave mixed-version binaries or report success while critical services are unavailable. The update needs an explicit transaction boundary, critical/optional step policy, and a recovery path.

#### Acceptance Criteria

- [x] Extract and validate the new payload in a same-volume staging directory before replacing live files.
- [x] Define required versus optional install steps; failure of a required step returns a non-zero result and cannot be reported as success.
- [x] Preserve the prior binaries and service configuration until critical post-install verification passes.
- [x] On injected failures during file swap, driver/service setup, or verification, restore the previous usable binaries and service state, then report the original and rollback outcomes.
- [x] Add tests for success, rollback, interrupted update recovery on next launch, and cleanup of abandoned staging directories.
- [x] Repeated apply/recovery operations are idempotent and do not overwrite the permanent setup shell.
- [x] Run the full `pnpm run health-check` suite and setup-shell tests successfully.

## Sprint Commit

```bash
git add sprint-planning/sprint-65-p0-installer-reliability/sprint.md sprint-planning/sprint-overview.md
git commit -m "fix(sprint-65): harden decoupled installer release and recovery"

- S65-001: Restore and regression-test opaque tray surfaces in OS dark mode (S63)
- S65-002: Bootstrap the setup shell for legacy installations (installer migration)
- S65-003: Unify CI and local release artifact contracts (release pipeline)
- S65-004: Validate payloads before disrupting the installed app (fail-closed preflight)
- S65-005: Apply installs transactionally and roll back critical failures (recovery)
```
