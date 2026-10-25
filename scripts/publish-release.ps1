# Publishes a MiControl release — DECOUPLED INSTALLER EDITION.
#
# Architecture (v2):
#   * MiControl_Setup.exe              — the STATIC setup shell. Contains NO
#     app payload. Its bytes (and hash) stay STABLE across releases; it is
#     rebuilt+re-uploaded only when shell code changes (-RebuildShell).
#   * MiControl_<version>_payload.zip  — the versioned, minisign-signed
#     payload the shell extracts and applies.
#   * latest.json                      — Tauri updater manifest pointing at
#     the payload zip (the plugin downloads + signature-verifies it; the
#     SYSTEM bridge then hands it to the installed shell).
#
# The legacy NSIS installer is still built (bundle validation + manual
# fallback) but is no longer the updater artifact.
#
# USAGE (from repo root, i.e. C:\Users\mafsc\Documents\Projects\miPC\micontrol):
#   powershell -ExecutionPolicy Bypass -File scripts/publish-release.ps1 -Version 0.2.20
#   powershell -ExecutionPolicy Bypass -File scripts/publish-release.ps1 -Version 0.2.20 -RebuildShell
#
# The signing key password is NEVER read from arguments or the command line.
# Password source order:
#   1) $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -- if set, `tauri build` and
#      `tauri signer sign` run without prompting.
#   2) interactive prompt (type the passphrase in the terminal).
# NOTE: run `setx ...` yourself in your own terminal -- never share the secret
# through chat or tooling.
param(
  [string]$Version = '0.2.20',
  [string]$Repo = 'arcane-D7/micontrol',
  [switch]$RebuildShell
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$tag = "v$Version"
$product = 'MiControl'
$payloadName = "$product`_$($Version)_payload.zip"
$setupName = "$product`_Setup.exe"
$releaseDir = Join-Path $root 'src-tauri\target\release'
$bundleDir = Join-Path $releaseDir 'bundle\nsis'
$shellCache = Join-Path $root 'setup-shell'

Write-Host "==> MiControl release publisher (DECOUPLED) v$Version (tag $tag)" -ForegroundColor Cyan

# 1. Private key (required). Password: leave unset so tauri prompts
#    interactively -- NEVER capture it in a variable/argument.
$keyPath = Join-Path $env:USERPROFILE '.tauri\micontrol-new.key'
if (-not (Test-Path $keyPath)) { throw "Signing key not found at $keyPath" }
$env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content $keyPath -Raw).Trim()
Write-Host "==> key loaded from $keyPath"

# 2. Build the app (NSIS bundle validates the payload file set; it also stays
#    available as a manual fallback installer).
Write-Host '==> tauri build (NSIS, validated)...' -ForegroundColor Cyan
Push-Location $root
try {
  pnpm --dir $root exec tauri build -f face --bundles nsis
  if ($LASTEXITCODE -ne 0) { throw "tauri build failed (exit $LASTEXITCODE)" }
} finally {
  Pop-Location
}

# 3. Build/publish the STATIC setup shell (stable hash!).
#    - If a signed shell is cached in setup-shell\ and -RebuildShell is not
#      given, REUSE the exact same bytes (hash stays identical across
#      releases). This is the whole point: one installer, one hash, forever.
Write-Host '==> setup shell...' -ForegroundColor Cyan
$shellCached = Join-Path $shellCache $setupName
if ((Test-Path $shellCached) -and -not $RebuildShell) {
  Write-Host "    reusing cached shell (stable hash): $shellCached"
} else {
  Push-Location (Join-Path $root 'src-tauri')
  try {
    cargo build -p micontrol-setup --profile shell-release
    if ($LASTEXITCODE -ne 0) { throw "cargo build (shell) failed (exit $LASTEXITCODE)" }
  } finally {
    Pop-Location
  }
  # Custom cargo profiles (anything other than dev/release) emit to
  # `target\<profile-name>\` — a SIBLING of `target\release\`, not nested
  # inside it. Point at the sibling path.
  $shellRaw = Join-Path $root 'src-tauri\target\shell-release\micontrol_setup.exe'
  if (-not (Test-Path $shellRaw)) { throw "Shell binary not found: $shellRaw" }
  New-Item -ItemType Directory -Force -Path $shellCache | Out-Null
  Copy-Item $shellRaw $shellCached -Force
  # Authenticode-sign the shell (same cert the Tauri bundler uses).
  Write-Host '    signing shell (signtool)...'
  $certThumb = '4678FB4266F446C72CF4A181D6A348B9EE9C9661'
  # SDK layout: bin\<version>\<arch>\signtool.exe (NOT bin\*-x64\).
  $signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Recurse -Filter 'signtool.exe' -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\' } |
    Sort-Object FullName -Descending | Select-Object -First 1
  if ($null -eq $signtool) { throw 'signtool.exe not found (Windows SDK)' }
  & $signtool.FullName sign /sha1 $certThumb /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $shellCached
  if ($LASTEXITCODE -ne 0) { throw 'signtool failed for the setup shell' }
  Write-Host "    shell built + signed: $shellCached"
}
$shellHash = (Get-FileHash $shellCached -Algorithm SHA256).Hash
Write-Host "    shell SHA256: $shellHash"

# 4. Build the versioned payload zip (what the shell actually installs).
#    FAIL-CLOSED (S65-004): if any REQUIRED payload entry (executable,
#    driver, resource) is missing the release aborts immediately — a partial
#    archive is never built, let alone signed or uploaded. Optional components
#    are explicitly declared in payload.json and only reported (never fatal)
#    when absent.
#    KEEP IN SYNC: REQUIRED/OPTIONAL lists mirror REQUIRED_PAYLOAD_ENTRIES /
#    OPTIONAL_PAYLOAD_ENTRIES in scripts/release-contract.mjs and
#    REQUIRED_ENTRIES / OPTIONAL_ENTRIES in src-tauri/shell/src/payload.rs.
Write-Host '==> building payload zip (fail-closed)...' -ForegroundColor Cyan
$payloadZip = Join-Path $bundleDir $payloadName
if (Test-Path $payloadZip) { Remove-Item $payloadZip -Force }
$staging = Join-Path $env:TEMP "micontrol-payload-$Version-$PID"
if (Test-Path $staging) { Remove-Item $staging -Recurse -Force }
New-Item -ItemType Directory -Force -Path $staging | Out-Null

$requiredPayloadFiles = @('micontrol.exe', 'ecram_service.exe', 'micontrol_bridge.exe')
$optionalPayloadFiles = @('micontrol_face_svc.exe', 'micontrol_facecp.dll', 'WebView2Loader.dll')
$requiredPayloadDirs = @()
$optionalPayloadDirs = @('drivers', 'resources')

$missingRequired = @()
foreach ($f in $requiredPayloadFiles) {
  $src = Join-Path $releaseDir $f
  if (Test-Path $src) { Copy-Item $src (Join-Path $staging $f) }
  else { $missingRequired += $f }
}
foreach ($d in $requiredPayloadDirs) {
  $src = Join-Path $releaseDir $d
  if (Test-Path $src) { Copy-Item $src (Join-Path $staging $d) -Recurse }
  else { $missingRequired += $d }
}
if ($missingRequired.Count -gt 0) {
  throw "FAIL-CLOSED: required payload entry(s) missing — $($missingRequired -join ', '). " +
        'A complete archive is mandatory (S65-004); release aborted.'
}
foreach ($f in $optionalPayloadFiles) {
  $src = Join-Path $releaseDir $f
  if (Test-Path $src) { Copy-Item $src (Join-Path $staging $f) }
  else { Write-Warning "optional payload component absent (declared optional in payload.json): $f" }
}
foreach ($d in $optionalPayloadDirs) {
  $src = Join-Path $releaseDir $d
  if (Test-Path $src) { Copy-Item $src (Join-Path $staging $d) -Recurse }
  else { Write-Warning "optional payload component absent (declared optional in payload.json): $d" }
}
# Versioned manifest consumed by the shell (status/reporting). Explicitly
# declares the OPTIONAL component set (S65-004 AC: 'optional components are
# explicitly declared in a versioned manifest').
$manifest = @{
  version = $Version
  optional = @(
    'micontrol_face_svc.exe',
    'micontrol_facecp.dll',
    'WebView2Loader.dll',
    'drivers',
    'resources'
  )
}
($manifest | ConvertTo-Json -Compress) | Set-Content -Encoding UTF8 (Join-Path $staging 'payload.json')

Compress-Archive -Path (Join-Path $staging '*') -DestinationPath $payloadZip -CompressionLevel Optimal
Remove-Item $staging -Recurse -Force
Write-Host "    payload: $payloadZip ($((Get-Item $payloadZip).Length) bytes)"

# 5. Sign the payload with the SAME minisign key pair the updater uses.
Write-Host '==> signing payload (minisign)...'
Push-Location $root
try {
  pnpm --dir $root exec tauri signer sign -k $env:TAURI_SIGNING_PRIVATE_KEY $payloadZip
  if ($LASTEXITCODE -ne 0) { throw 'tauri signer sign failed for the payload' }
} finally {
  Pop-Location
}
$payloadSig = "$payloadZip.sig"
if (-not (Test-Path $payloadSig)) { throw 'Payload signature not found (signing did not run?)' }

# 6. Generate latest.json for the Tauri v2 updater (now points at the PAYLOAD).
Write-Host '==> generating latest.json (payload artifact)...' -ForegroundColor Cyan
$latestJson = Join-Path $bundleDir 'latest.json'
$payloadUrl = "https://github.com/$Repo/releases/latest/download/$payloadName"
node (Join-Path $root 'scripts\generate-latest-json.mjs') $Version $payloadSig $payloadUrl $latestJson
if ($LASTEXITCODE -ne 0) { throw 'generate-latest-json.mjs failed' }

# 6b. S65-003/S65-004: offline validation of the FULL staged asset set.
#     Fails BEFORE any publication if names, signatures, or required assets
#     are missing/misaligned (single source of truth: release-contract.mjs,
#     which also verifies latest.json targets the payload, never the NSIS
#     installer, and that the payload has a valid minisign signature).
#     The contract expects the SIGNED shell in the bundle dir — stage it.
Copy-Item $shellCached (Join-Path $bundleDir $setupName) -Force
Write-Host '==> validating staged assets (release-contract dry-run + check)...' -ForegroundColor Cyan
node (Join-Path $root 'scripts\release-contract.mjs') dry-run $Version $payloadZip $payloadSig
if ($LASTEXITCODE -ne 0) { throw 'release-contract dry-run failed — staging aborted' }
node (Join-Path $root 'scripts\release-contract.mjs') check $Version $bundleDir
if ($LASTEXITCODE -ne 0) { throw 'release-contract check failed — staging aborted' }

# 7. Extract the release body from CHANGELOG.md.
Write-Host '==> extracting changelog body...' -ForegroundColor Cyan
$bodyPath = Join-Path $env:TEMP "micontrol-$Version-body.md"
node (Join-Path $root 'scripts\extract-changelog.mjs') $Version $bodyPath
if ($LASTEXITCODE -ne 0) { throw 'extract-changelog.mjs failed' }

# 8. Create the GitHub release with all assets.
Write-Host '==> creating GitHub release...' -ForegroundColor Cyan
$nsisExe = Join-Path $bundleDir "$product`_$($Version)_x64-setup.exe"
$nsisSig = "$nsisExe.sig"
$assets = @($payloadZip, $payloadSig, $latestJson, $shellCached)
if (Test-Path $nsisExe) { $assets += $nsisExe }
if (Test-Path $nsisSig) { $assets += $nsisSig }
gh release create $tag --repo $Repo --title "$product v$Version" --notes-file $bodyPath @assets
if ($LASTEXITCODE -ne 0) { throw 'gh release create failed' }
Write-Host "==> release created: https://github.com/$Repo/releases/tag/$tag" -ForegroundColor Green

# 9. Push the tag so the git history/remote matches the release.
#    NOTE: dispatching release.yml on tag push will re-attempt a cold build on
#    the 2-vCPU runner (historically infeasible) -- cancel that run if it
#    starts (gh run list --workflow=release.yml; gh run cancel <id>).
Write-Host '==> pushing tag...' -ForegroundColor Cyan
git -C $root push origin $tag
if ($LASTEXITCODE -ne 0) { Write-Warning 'git push origin <tag> failed (tag may already exist remotely — fine).' }
else { Write-Host "==> tag pushed: origin/$tag" -ForegroundColor Green }

Write-Host ''
Write-Host "DONE. Release $tag published (decoupled installer)." -ForegroundColor Green
Write-Host "  payload zip : $payloadName (updater artifact)"
Write-Host "  setup shell : $setupName (STATIC — SHA256 $shellHash)"
Write-Host '  latest.json : points at the payload zip'
Write-Host 'If release.yml started on CI, cancel it:'
Write-Host "  gh run list --repo $Repo --workflow=release.yml --limit 3"
Write-Host '  gh run cancel <run-id>'
