# Release Process

This document describes how to cut a release of MiControl, including build, signing, and publication.

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

The release process is fully automated. Run a single command locally:

```bash
# Bump patch version: 1.0.0 → 1.0.1
pnpm release patch

# Bump minor version: 1.0.0 → 1.1.0
pnpm release minor

# Bump major version: 1.0.0 → 2.0.0
pnpm release major

# Or specify an explicit version
pnpm release 1.2.3
```

The script will:

1. Verify the working tree is clean and you're on `main` (in sync with remote)
2. Bump the version in `package.json`, `Cargo.toml`, and `tauri.conf.json`
3. Commit the version bump as `chore(release): vX.Y.Z`
4. Create an annotated git tag `vX.Y.Z`
5. Push the commit and tag to `origin`

The tag push triggers the GitHub Actions release workflow which:

- Builds the Tauri app for Windows (NSIS installer)
- Signs the update bundle with the Tauri signing key
- Signs the installer with Authenticode (if cert configured)
- Generates `latest.json` for the auto-updater
- Creates a GitHub Release with the installer `.exe` and `latest.json` attached

### Verify the release

- Check the Actions run: https://github.com/arcane-D7/micontrol/actions
- Check the release: https://github.com/arcane-D7/micontrol/releases
- Download the installer and verify the signature
- Test the updater by installing the previous version and updating

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
