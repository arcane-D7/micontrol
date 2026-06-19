<#
.SYNOPSIS
    Checks the IoTDriver DriverStore for unsigned or unexpected binaries.

.DESCRIPTION
    This script is a CI/CD integrity check that scans the active IoTDriver
    DriverStore package for all .exe, .sys, and .dll files, verifies their
    digital signatures, computes SHA256 hashes, and compares against a
    known-good allowlist.

    It was created as part of Sprint 1 (S1-013) after discovering that an
    unsigned `ecram_shim.exe` was silently added to the DriverStore in
    IoTDriver v25.0.0.9.

.PARAMETER DriverStorePath
    Path to the DriverStore package to scan. If omitted, auto-detects by
    searching C:\WINDOWS\System32\DriverStore\FileRepository\ for iotdriver.inf*.

.PARAMETER AllowlistPath
    Path to the JSON allowlist file. Default: scripts/driverstore-allowlist.json

.PARAMETER Fix
    If set, updates the allowlist with the current binary hashes instead of
    checking against it. Use this for initial setup or after a verified
    driver update.

.EXAMPLE
    .\check-driverstore-integrity.ps1
    Scans the active DriverStore and checks against the allowlist.

.EXAMPLE
    .\check-driverstore-integrity.ps1 -Fix
    Updates the allowlist with current hashes (initial setup).

.NOTES
    Exit codes: 0 = all binaries signed and in allowlist, 1 = issues found.
#>
[CmdletBinding()]
param(
    [string]$DriverStorePath,
    [string]$AllowlistPath = (Join-Path $PSScriptRoot "driverstore-allowlist.json"),
    [switch]$Fix
)

$ErrorActionPreference = "Stop"

# ── Auto-detect DriverStore path if not provided ──────────────────────────────
if (-not $DriverStorePath) {
    $repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
    $driverStoreRoot = "C:\WINDOWS\System32\DriverStore\FileRepository"
    $candidates = Get-ChildItem -Path $driverStoreRoot -Directory -Filter "iotdriver.inf*" -ErrorAction SilentlyContinue
    if (-not $candidates -or $candidates.Count -eq 0) {
        Write-Host "ERROR: No IoTDriver DriverStore package found in $driverStoreRoot" -ForegroundColor Red
        Write-Host "Pass -DriverStorePath explicitly to scan a different location." -ForegroundColor Yellow
        exit 1
    }
    # Use the most recently modified (active) package
    $DriverStorePath = ($candidates | Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName
    Write-Host "Auto-detected DriverStore: $DriverStorePath" -ForegroundColor Cyan
}

if (-not (Test-Path $DriverStorePath)) {
    Write-Host "ERROR: DriverStore path not found: $DriverStorePath" -ForegroundColor Red
    exit 1
}

# ── Scan for binaries ─────────────────────────────────────────────────────────
$binaries = Get-ChildItem -Path $DriverStorePath -Recurse -Include *.exe,*.sys,*.dll -ErrorAction SilentlyContinue
if (-not $binaries -or $binaries.Count -eq 0) {
    Write-Host "ERROR: No binaries (.exe/.sys/.dll) found in $DriverStorePath" -ForegroundColor Red
    exit 1
}

Write-Host "`n=== DriverStore Integrity Check ===" -ForegroundColor Cyan
Write-Host "Path: $DriverStorePath"
Write-Host "Binaries found: $($binaries.Count)`n"

# ── Compute signatures and hashes ─────────────────────────────────────────────
$results = @()
foreach ($bin in $binaries) {
    $relPath = $bin.FullName.Substring($DriverStorePath.Length).TrimStart('\')
    $sig = Get-AuthenticodeSignature -FilePath $bin.FullName
    $hash = (Get-FileHash -Path $bin.FullName -Algorithm SHA256).Hash

    $results += [PSCustomObject]@{
        File         = $relPath
        Size         = $bin.Length
        SHA256       = $hash
        Signed       = $sig.Status -eq "Valid"
        Signer       = $sig.SignerCertificate.Subject
        SignatureStatus = $sig.Status
    }
}

# ── Display results ───────────────────────────────────────────────────────────
$results | Format-Table File, Size, Signed, SignatureStatus -AutoSize

# ── Fix mode: write allowlist and exit ────────────────────────────────────────
if ($Fix) {
    $allowlist = @{
        description = "Known-good binaries in the IoTDriver DriverStore"
        generated   = (Get-Date -Format "o")
        driverStore  = $DriverStorePath
        binaries    = @()
    }
    foreach ($r in $results) {
        $allowlist.binaries += @{
            file    = $r.File
            sha256  = $r.SHA256
            size    = $r.Size
            signed  = $r.Signed
            signer  = $r.Signer
        }
    }
    $allowlist | ConvertTo-Json -Depth 5 | Set-Content -Path $AllowlistPath -Encoding UTF8
    Write-Host "`nAllowlist written to: $AllowlistPath" -ForegroundColor Green
    Write-Host "Review the allowlist and commit it to the repository." -ForegroundColor Yellow
    exit 0
}

# ── Check mode: compare against allowlist ──────────────────────────────────────
if (-not (Test-Path $AllowlistPath)) {
    Write-Host "ERROR: Allowlist not found at $AllowlistPath" -ForegroundColor Red
    Write-Host "Run with -Fix to generate an initial allowlist." -ForegroundColor Yellow
    exit 1
}

$allowlist = Get-Content -Path $AllowlistPath -Raw | ConvertFrom-Json
$allowedHashes = @{}
foreach ($entry in $allowlist.binaries) {
    $allowedHashes[$entry.sha256] = $entry.file
}

$issues = @()

# Check 1: unsigned binaries
$unsigned = $results | Where-Object { -not $_.Signed }
foreach ($u in $unsigned) {
    $issues += [PSCustomObject]@{
        Severity = "CRITICAL"
        File     = $u.File
        Issue    = "Unsigned binary"
        Detail   = "No valid Authenticode signature"
    }
}

# Check 2: binaries not in allowlist
foreach ($r in $results) {
    if (-not $allowedHashes.ContainsKey($r.SHA256)) {
        $issues += [PSCustomObject]@{
            Severity = "WARNING"
            File     = $r.File
            Issue    = "Unknown binary (not in allowlist)"
            Detail   = "SHA256: $($r.SHA256)"
        }
    }
}

# Check 3: binaries in allowlist but missing from DriverStore
$actualHashes = @{}
foreach ($r in $results) {
    $actualHashes[$r.SHA256] = $r.File
}
foreach ($entry in $allowlist.binaries) {
    if (-not $actualHashes.ContainsKey($entry.sha256)) {
        $issues += [PSCustomObject]@{
            Severity = "INFO"
            File     = $entry.file
            Issue    = "Allowlisted binary missing from DriverStore"
            Detail   = "May have been removed in a driver update"
        }
    }
}

# ── Report ────────────────────────────────────────────────────────────────────
if ($issues.Count -gt 0) {
    Write-Host "`n=== INTEGRITY ISSUES FOUND ===" -ForegroundColor Red
    $issues | Format-Table Severity, File, Issue, Detail -AutoSize

    $critical = ($issues | Where-Object { $_.Severity -eq "CRITICAL" }).Count
    $warnings = ($issues | Where-Object { $_.Severity -eq "WARNING" }).Count
    Write-Host "`nSummary: $critical critical, $warnings warnings" -ForegroundColor Red
    exit 1
} else {
    Write-Host "`n✓ All binaries are signed and match the allowlist." -ForegroundColor Green
    exit 0
}
