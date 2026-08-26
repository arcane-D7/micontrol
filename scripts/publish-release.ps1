# Publishes a MiControl release locally (the v0.1.18 recipe, automated).
#
# Builds the signed NSIS installer (prompts for the key password ONLY IF
# TAURI_SIGNING_PRIVATE_KEY_PASSWORD is not already set), generates
# latest.json, extracts the CHANGELOG body, creates the GitHub release with
# all assets, and pushes the git tag.
#
# USAGE (from repo root, i.e. C:\Users\mafsc\Documents\Projects\miPC\micontrol):
#   powershell -ExecutionPolicy Bypass -File scripts/publish-release.ps1 -Version 0.1.19
#
# The signing key password is NEVER read from arguments or the command line.
# If TAURI_SIGNING_PRIVATE_KEY_PASSWORD is unset, `tauri build` will prompt
# interactively — type the passphrase into the terminal there.
param(
  [string]$Version = '0.1.19',
  [string]$Repo = 'arcane-D7/micontrol'
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$tag = "v$Version"
$product = 'MiControl'
$installer = "$product`_$($Version)_x64-setup.exe"
$bundleDir = Join-Path $root "src-tauri\target\release\bundle\nsis"

Write-Host "==> MiControl release publisher v$Version (tag $tag)" -ForegroundColor Cyan

# 1. Private key (required). Password: leave unset so tauri prompts
#    interactively -- NEVER capture it in a variable/argument.
$keyPath = Join-Path $env:USERPROFILE '.tauri\micontrol-new.key'
if (-not (Test-Path $keyPath)) { throw "Signing key not found at $keyPath" }
$env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content $keyPath -Raw).Trim()
Write-Host "==> key loaded from $keyPath"

# 2. Build the signed NSIS bundle (incremental -- minutes on this machine).
Write-Host '==> tauri build (signed NSIS)...' -ForegroundColor Cyan
Push-Location $root
try {
  pnpm --dir $root exec tauri build -f face --bundles nsis
  if ($LASTEXITCODE -ne 0) { throw "tauri build failed (exit $LASTEXITCODE)" }
} finally {
  Pop-Location
}

$exePath = Join-Path $bundleDir $installer
$sigPath = "$exePath.sig"
if (-not (Test-Path $exePath)) { throw "Installer not found: $exePath" }
if (-not (Test-Path $sigPath)) { throw "Signature not found: $sigPath (signing did not run?)" }
Write-Host "==> installer  : $exePath"
Write-Host "==> signature  : $sigPath"

# 3. Generate latest.json for the Tauri v2 updater.
Write-Host '==> generating latest.json...' -ForegroundColor Cyan
$latestJson = Join-Path $bundleDir 'latest.json'
$installerUrl = "https://github.com/$Repo/releases/latest/download/$installer"
node (Join-Path $root 'scripts\generate-latest-json.mjs') $Version $sigPath $installerUrl $latestJson
if ($LASTEXITCODE -ne 0) { throw 'generate-latest-json.mjs failed' }

# 4. Extract the release body from CHANGELOG.md.
Write-Host '==> extracting changelog body...' -ForegroundColor Cyan
$bodyPath = Join-Path $env:TEMP "micontrol-$Version-body.md"
node (Join-Path $root 'scripts\extract-changelog.mjs') $Version $bodyPath
if ($LASTEXITCODE -ne 0) { throw 'extract-changelog.mjs failed' }

# 5. Create the GitHub release with all assets.
Write-Host '==> creating GitHub release...' -ForegroundColor Cyan
gh release create $tag --repo $Repo --title "$product v$Version" --notes-file $bodyPath $exePath $sigPath $latestJson
if ($LASTEXITCODE -ne 0) { throw 'gh release create failed' }
Write-Host "==> release created: https://github.com/$Repo/releases/tag/$tag" -ForegroundColor Green

# 6. Push the tag so the git history/remote matches the release.
#    NOTE: dispatching release.yml on tag push will re-attempt a cold build on
#    the 2-vCPU runner (historically infeasible) -- cancel that run if it
#    starts (gh run list --workflow=release.yml; gh run cancel <id>).
Write-Host '==> pushing tag...' -ForegroundColor Cyan
git -C $root push origin $tag
if ($LASTEXITCODE -ne 0) { Write-Warning 'git push origin <tag> failed (tag may already exist remotely — fine).' }
else { Write-Host "==> tag pushed: origin/$tag" -ForegroundColor Green }

Write-Host ''
Write-Host "DONE. Release $tag published. If release.yml started on CI, cancel it:" -ForegroundColor Green
Write-Host "  gh run list --repo $Repo --workflow=release.yml --limit 3"
Write-Host '  gh run cancel <run-id>'
