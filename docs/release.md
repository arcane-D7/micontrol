# Release Process

MiControl releases are cut by a script and built by CI: `pnpm run release <type>` bumps the version everywhere, commits, tags and pushes; the tag push triggers `release.yml`, which runs the health checks, builds, signs and publishes. Changelog editing is manual (the release body is extracted from `CHANGELOG.md`).

## How It Works

```
  Conventional Commits            scripts/release.cjs             release.yml
  ────────────────────            ───────────────────             ───────────
  feat: add X
  fix: resolve Y

  pnpm run release patch ──────►  Guard: clean tree, on main,
  (or minor / major / X.Y.Z)      in sync with origin/main

                                  Bump version via
                                  scripts/sync-version.cjs:
                                    package.json
                                    src-tauri/Cargo.toml
                                    src-tauri/tauri.conf.json

                                  Commit chore(release): vX.Y.Z
                                  Tag vX.Y.Z
                                  Push main + tag

                                  tag push ──────────────────►  Job 1: Health checks
                                                                 (scripts/health-check.mjs)

                                                               Job 2: Build & publish
                                                                 pnpm run tauri build -f face
                                                                 build static setup shell
                                                                 sign payload + latest.json
                                                                 offline dry-run validation
                                                                 GitHub Release from CHANGELOG
```

There is **no** `auto-release.yml` and **no** release-please integration in this repository. A release is cut by running the release script on `main`; the tag push is what triggers `release.yml`.

## Conventional Commits

All commits to `main` must follow [Conventional Commits](https://www.conventionalcommits.org/):

```
type(scope): subject

body (optional)

footer(s) (optional)
```

### Commit Types

| Type       | Bump to pass            | Example                            |
| ---------- | ----------------------- | ---------------------------------- |
| `feat`     | `minor` (0.1.0 → 0.2.0) | `feat: add fan curve editor`       |
| `fix`      | `patch` (0.1.0 → 0.1.1) | `fix: resolve tray icon crash`     |
| `perf`     | `patch` (0.1.0 → 0.1.1) | `perf: optimize WMI cache refresh` |
| `feat!`    | `major` (0.1.0 → 1.0.0) | `feat!: redesign settings UI`      |
| `fix!`     | `major` (0.1.0 → 1.0.0) | `fix!: change ECRAM protocol`      |
| `revert`   | `patch` (0.1.0 → 0.1.1) | `revert: remove fan curve editor`  |
| `chore`    | no release              | `chore: update dependencies`       |
| `docs`     | no release              | `docs: update README`              |
| `refactor` | no release              | `refactor: simplify IPC layer`     |
| `test`     | no release              | `test: add battery unit tests`     |
| `ci`       | no release              | `ci: add auto-release workflow`    |
| `style`    | no release              | `style: format imports`            |
| `build`    | no release              | `build: update Cargo.toml deps`    |

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

### `release.yml`

- **Trigger**: tag push `v*`, or `workflow_dispatch` with a `tag` input
- **Job 1 — Health checks**: runs `node scripts/health-check.mjs` (version consistency, `cargo fmt --check`, clippy with `-D warnings`, Rust tests, `tsc --noEmit`, ESLint, Prettier, Vitest + coverage, Vite build, i18n check, handle-leak gate, toolchain check)
- **Job 2 — Build & publish**: `pnpm run tauri build -f face`, builds the static setup shell, signs the decoupled payload and generates `latest.json`, runs an offline dry-run release validation, then creates the GitHub Release with the body extracted from `CHANGELOG.md`
- **Job 2 depends on Job 1**: the release only proceeds if every health check passes
- **Concurrency**: `release-build` with `cancel-in-progress`, so a re-pushed tag cancels the older build instead of doubling the Windows runner budget

### `ci.yml`

- **Trigger**: PR to `main`, push to `main`, `workflow_dispatch`
- **Jobs**: `quality` (health-check steps `version,fmt,clippy,tsc,lint,format,i18n,toolchain`) and `test` (health-check steps `test-rust,test-front`, then uploads the coverage artifact)

### Other workflows

| Workflow                    | Trigger                               | Purpose                                            |
| --------------------------- | ------------------------------------- | -------------------------------------------------- |
| `codeql.yml`                | push / PR to `main`                   | CodeQL static analysis (JS/TS; non-blocking)       |
| `driverstore-integrity.yml` | weekly cron, paths push/PR, dispatch  | Verifies the deployed DriverStore `IoTService.exe` |
| `landing.yml`               | push to `main` touching landing paths | Builds and deploys the landing page                |
| `warm-cache.yml`            | dispatch only (disabled)              | Kept for manual release-cache warming              |

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

1. **Commit changes** using Conventional Commits format (e.g., `feat: add X`, `fix: resolve Y`) and merge them to `main`.
2. **Make sure `main` is clean and in sync** — the release script refuses to run otherwise.
3. **Run the release script** with the bump type:

   ```bash
   pnpm run release patch    # 0.2.20 → 0.2.21
   pnpm run release minor    # 0.2.20 → 0.3.0
   pnpm run release major    # 0.2.20 → 1.0.0
   pnpm run release 1.2.3    # explicit version
   ```

   It bumps `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` through `scripts/sync-version.cjs`, commits `chore(release): vX.Y.Z`, creates the annotated tag and pushes both `main` and the tag.

4. **The tag push triggers `release.yml`**, which runs the health checks, builds, signs, and publishes.

No manual changelog editing is needed — the release body is extracted from `CHANGELOG.md` by `scripts/extract-changelog.mjs`.

### Verify the release

- Check the Actions run: https://github.com/arcane-D7/micontrol/actions
- Check the release: https://github.com/arcane-D7/micontrol/releases
- Download the installer and verify the signature
- Test the updater by installing the previous version and updating

### Re-running a release build

If the build failed but the tag is correct, re-run `release.yml` from the Actions UI (`workflow_dispatch` with the `tag` input) instead of creating a new version. Re-pushing the same tag also works — the `release-build` concurrency group cancels the stale run.

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

- **Build fails**: Check that all versions are synced (`pnpm run version:check`)
- **Signing fails**: Verify the `TAURI_SIGNING_PRIVATE_KEY` secret is set correctly
- **Release not created**: Ensure the workflow has `permissions: contents: write`

## EC RAM Service (ecram_service.exe)

MiControl includes a custom `IoTService.exe` replacement binary (`src-tauri/src/bin/ecram_service.rs`) that proxies ECRAM read/write IOCTLs to the Xiaomi `IoTDriver.sys` kernel driver. This binary is required for ECRAM access (IOT_STATUS, IOT_SENSORS, ECRAM sensor block).

### Building

The binary is built as part of the normal Tauri release build:

```bash
pnpm run tauri build
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
- ERAM (`0xFE0B0300`) and SMA2 (`0xFE0B0A00`) are **not accessible via IoTDriver** — not in allowed ranges. AC adapter wattage is available via WMI instead.
- See [RE_ANALYSIS_REPORT.md](./RE_ANALYSIS_REPORT.md) for complete details


## Assinatura do updater — protocolo recuperado (2026-11-14)

Fonte: histórico de sessões do Copilot (runbook das releases 0.1.18/0.1.20) + verificação ao vivo na máquina.

### Factos
- **Chave correcta**: `~/.tauri/micontrol-new.key` — a pubkey em `tauri.conf.json` **bate exactamente** com `micontrol-new.key.pub`. Existem duas chaves antigas (`micontrol.key`, `micontrol-key`) e o CLI escolhe-as por omissão se `TAURI_SIGNING_PRIVATE_KEY` não estiver definida. **Definir sempre explicitamente.**
- **Password**: está guardada como **variável de ambiente de utilizador** `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (13 caracteres) e também como secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` no repo GitHub (confirmado via `gh secret list`, 2026-07-20). Não é preciso digitá-la — com as duas env vars definidas o signing corre sem prompt (verificado: `tauri signer sign` produziu um `.sig` válido).

### Sequência (PowerShell, sem prompts)
```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content "$env:USERPROFILE\.tauri\micontrol-new.key" -Raw).Trim()
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = [Environment]::GetEnvironmentVariable('TAURI_SIGNING_PRIVATE_KEY_PASSWORD','User')
pnpm run tauri build -f face --config build-local-nosign.json   # sem cert local; o updater fica assinado
# latest.json para o endpoint do updater:
node scripts/generate-latest-json.mjs <versao> <caminho .sig> "https://github.com/arcane-D7/micontrol/releases/latest/download/MiControl_<versao>_x64-setup.exe" src-tauri/target/release/bundle/nsis/latest.json
# corpo da release a partir do CHANGELOG:
node scripts/extract-changelog.mjs <versao> body.md
gh release create v<versao> --repo arcane-D7/micontrol --title "..." --notes-file body.md <artefactos>
```
- Alternativa: `scripts/publish-release.ps1 -Version <versao>` faz tudo (carrega a chave sozinho; a password vem da env var — "no prompt" confirmado em 0.1.20).
- **O `release.yml` já não compila**: é um verificador. Com o tag push, o CI descarrega a release publicada e valida o contrato (instalador + `.sig` + `latest.json` com a versão e a assinatura minisign corretas) em ~1 minuto. Um release mal publicado falha aí. O build assinado continua a ser local (`publish-release.ps1`).
- Produção local usa `--config build-local-nosign.json` (sem certificado Authenticode); o `TAURI_SIGNING_PRIVATE_KEY` trata apenas do artefacto do updater.
