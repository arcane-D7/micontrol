# Release Process

MiControl uses a fully automated semantic release pipeline. No manual version bumping, changelog editing, or tag creation is needed.

## How It Works

```
  Conventional Commits          release-please              release.yml
  ────────────────────          ──────────────              ──────────
  feat: add X
  fix: resolve Y
  push to main ──────────────►  Maintains a "release PR"
                                with version bump +
                                CHANGELOG.md

  Merge release PR ──────────►  Creates git tag vX.Y.Z

                                tag push ──────────────►  Health checks
                                                          (fmt, clippy, test,
                                                           lint, tsc, build)

                                                          Build + sign Tauri

                                                          Generate latest.json

                                                          Create GitHub Release
                                                          with installer artifacts
```

## Conventional Commits

All commits to `main` must follow [Conventional Commits](https://www.conventionalcommits.org/):

```
type(scope): subject

body (optional)

footer(s) (optional)
```

### Commit Types

| Type       | Release Trigger       | Example                            |
| ---------- | --------------------- | ---------------------------------- |
| `feat`     | Minor (0.1.0 → 0.2.0) | `feat: add fan curve editor`       |
| `fix`      | Patch (0.1.0 → 0.1.1) | `fix: resolve tray icon crash`     |
| `perf`     | Patch (0.1.0 → 0.1.1) | `perf: optimize WMI cache refresh` |
| `feat!`    | Major (0.1.0 → 1.0.0) | `feat!: redesign settings UI`      |
| `fix!`     | Major (0.1.0 → 1.0.0) | `fix!: change ECRAM protocol`      |
| `chore`    | No release            | `chore: update dependencies`       |
| `docs`     | No release            | `docs: update README`              |
| `refactor` | No release            | `refactor: simplify IPC layer`     |
| `test`     | No release            | `test: add battery unit tests`     |
| `ci`       | No release            | `ci: add release-please workflow`  |
| `style`    | No release            | `style: format imports`            |
| `build`    | No release            | `build: update Cargo.toml deps`    |

### Breaking Changes

Add `!` after the type (and optional scope):

```
feat(api)!: change IPC protocol format
```

Or use the `BREAKING CHANGE:` footer:

```
feat: redesign settings UI

BREAKING CHANGE: Settings schema changed, old configs need migration.
```

## Workflows

### `release-please.yml`

- **Trigger**: Push to `main`
- **Action**: Analyzes commits since last release, maintains a "release PR"
- **When release PR is merged**: Creates git tag `vX.Y.Z`, updates `package.json`, `Cargo.toml`, `tauri.conf.json`, `CHANGELOG.md`

### `release.yml`

- **Trigger**: Tag push `v*.*.*`
- **Job 1 — Health Checks**: `cargo fmt --check`, `cargo check`, `cargo clippy -D warnings`, `cargo test`, `tsc --noEmit`, `eslint`, `prettier --check`, `npm run build`, i18n check
- **Job 2 — Build & Publish**: Builds Tauri app with signing, generates `latest.json`, creates GitHub Release with installer artifacts
- **Job 2 depends on Job 1**: Release only proceeds if all health checks pass

### `ci.yml`

- **Trigger**: PR to `main`, push to `main`
- **Purpose**: Continuous integration checks on every PR/push

## Prerequisites

### Tauri Updater Signing Key

The Tauri updater uses an Ed25519 keypair to sign update bundles. The private key signs the update; the public key (embedded in the app) verifies it.

**Generate a keypair:**

```bash
npx @tauri-apps/cli signer generate -w tauri-key
```

This creates a file containing the private key and prints the public key.

**Configure GitHub Secrets:**

- `TAURI_SIGNING_PRIVATE_KEY` — the contents of the private key file
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — the password (if you set one)

**Configure the public key:**

Replace the `pubkey` placeholder in `src-tauri/tauri.conf.json` with the generated public key.

### Authenticode Code Signing (Optional but recommended)

To avoid SmartScreen warnings:

1. Obtain a code signing certificate (OV or EV) from a trusted CA.
2. Export it as a PFX file.
3. Base64-encode it: `base64 -w0 cert.pfx > cert.b64`
4. Add two repository secrets:
   - `WINDOWS_CERTIFICATE`: the base64-encoded PFX
   - `WINDOWS_CERTIFICATE_PASSWORD`: the PFX password

If these secrets are not set, the release will succeed but the installer will be unsigned.

## Cutting a Release

The release process is **fully automated** via [release-please](https://github.com/googleapis/release-please):

1. **Commit changes** using Conventional Commits format (e.g., `feat: add X`, `fix: resolve Y`)
2. **Push to `main`** — release-please automatically maintains a "release PR" with version bump + changelog
3. **Merge the release PR** — release-please creates the git tag `vX.Y.Z`
4. **Tag push triggers `release.yml`** which runs health checks, builds, signs, and publishes

No manual version bumping, changelog editing, or tag creation is needed.

### Verify the release

- Check the Actions run: https://github.com/arcane-D7/micontrol/actions
- Check the release: https://github.com/arcane-D7/micontrol/releases
- Download the installer and verify the signature
- Test the updater by installing the previous version and updating

### Emergency Manual Release

If the automated pipeline is broken, you can manually trigger a release:

```bash
# Bump patch version: 1.0.0 → 1.0.1
npm run release patch

# Bump minor version: 1.0.0 → 1.1.0
npm run release minor

# Bump major version: 1.0.0 → 2.0.0
npm run release major

# Or specify an explicit version
npm run release 1.2.3
```

This bumps version, syncs configs, commits, tags, and pushes — triggering `release.yml`.

## Key Rotation

### Tauri Updater Key

1. Generate a new keypair
2. Update the `pubkey` in `tauri.conf.json`
3. Update the `TAURI_SIGNING_PRIVATE_KEY` GitHub secret
4. Release a new version with the new key
5. Old versions will not be able to update to the new version (key mismatch) — document this as a breaking change

### Windows Code Signing Certificate

1. Obtain a new certificate before the old one expires
2. Update the `WINDOWS_CERTIFICATE` and `WINDOWS_CERTIFICATE_PASSWORD` GitHub secrets
3. No app-side changes needed (the OS trusts the CA, not a specific cert)

## Certificate Expiry Tracking

- **Tauri updater key**: No expiry (Ed25519 keys don't expire)
- **Windows code signing cert**: Track the expiry date in your team calendar; renew at least 30 days before expiry

## Rollback Procedure

If a release needs to be rolled back:

### 1. Mark the Release as Draft (GitHub)

1. Go to the GitHub Releases page
2. Find the release to roll back
3. Click "Edit" (pencil icon)
4. Change the status to "Draft"
5. This hides the release from public view

### 2. Revert the Commit

```bash
git revert <release-commit-hash>
git push origin main
```

### 3. Re-publish Previous Version

1. Find the previous stable release tag
2. Re-publish the release artifacts from the previous tag
3. Update `latest.json` to point to the previous version

### 4. Notify Users

- Post an announcement in the release notes
- Update the download page if applicable

### 5. Post-Mortem

- Document the reason for rollback
- Add a regression test to prevent recurrence

## Troubleshooting

- **Build fails**: Check that all versions are synced (`npm run version:check`)
- **Signing fails**: Verify the `TAURI_SIGNING_PRIVATE_KEY` secret is set correctly
- **Release not created**: Ensure the workflow has `permissions: contents: write`

## EC RAM Service (ecram_service.exe)

MiControl includes a custom `IoTService.exe` replacement binary (`src-tauri/src/bin/ecram_service.rs`) that proxies ECRAM read/write IOCTLs to the Xiaomi `IoTDriver.sys` kernel driver. This binary is required for ECRAM access (IOT_STATUS, IOT_SENSORS, ECRAM sensor block).

### Building

The binary is built as part of the normal Tauri release build:

```bash
npm run tauri build
```

The resulting binary is at `src-tauri/target/release/ecram_service.exe`.

### Deployment

To deploy the custom IoTService.exe:

1. Copy `ecram_service.exe` to the IoTDriver DriverStore directory:
   ```
   C:\Windows\System32\DriverStore\FileRepository\iotdriver.inf_amd64_*\IoTService.exe
   ```
2. Rename it to `IoTService.exe` (if not already named)
3. Restart the IoTSvc service:
   ```powershell
   Restart-Service IoTSvc
   ```

### Security Notes

- The binary **must** be named `IoTService.exe` — the driver validates the process name
- The binary **must** be in the DriverStore directory — the driver validates the path
- The driver only allows access to 3 hardcoded physical address ranges:
  - `0xFE0B0F00` / 0x80 bytes (IOT_STATUS + IOT_SENSORS)
  - `0xFE0B0AB8` / 0x08 bytes (small status region)
  - `0xFE0B0E00` / 0x100 bytes (ECRAM sensor block)
- ERAM (`0xFE0B0300`) and SMA2 (`0xFE0B0A00`) are **not accessible** — not in allowed ranges
- See [RE_ANALYSIS_REPORT.md](./RE_ANALYSIS_REPORT.md) for complete details
