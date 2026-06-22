#Requires -RunAsAdministrator
#Requires -Version 5.1
<#
    S1-002 - HID Reports
    Two-phase comparison: Phase A (IoTService running) vs Phase B (IoTService stopped).
    No reboot required. Generates a comparison summary at the end.
#>
[CmdletBinding()]
param(
    [switch]$WithMiPC
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot "..\_common\TestFramework.psm1") -Force

$TicketId = "S1-002"
$Title    = "HID Reports (A: svc on / B: svc off)"
$session  = Initialize-TestEnvironment
$logFile  = Join-Path $session.LogsDir "$($TicketId)-$($session.Timestamp).log"
Write-TestHeader -TicketId $TicketId -Title $Title -LogFile $logFile

$svcWasRunning = $false

try {
    $preSnap = Get-SystemSnapshot -Label "pre-hid-comparison" -LogFile $logFile

    $resultsDir = Join-Path $session.ResultsDir "$($TicketId)-$($session.Timestamp)"
    New-Item -ItemType Directory -Path $resultsDir -Force | Out-Null

    # ---------- Phase A: IoTService RUNNING ----------
    Write-TestLog "PHASE A: IoTService RUNNING - capturing HID reports." -LogFile $logFile
    $svc = Get-Service -Name IoTSvc -ErrorAction SilentlyContinue
    if ($svc -and $svc.Status -eq "Running") {
        $svcWasRunning = $true
        Write-TestLog "IoTService is running (will restore to running after test)." -LogFile $logFile
    } else {
        Write-TestLog "IoTService is NOT running - starting it for Phase A." -Level "WARN" -LogFile $logFile
        sc.exe start IoTSvc 2>&1 | Out-Null
        Start-Sleep -Seconds 3
    }

    $miJobA = $null
    if ($WithMiPC) { $miJobA = Start-MiPCDevMode -LogFile $logFile }

    $hidPathA = Join-Path $resultsDir "phaseA-svc-running-hid.log"
    $hidJobA = Start-HidMonitor -OutputPath $hidPathA -DurationSec $script:CaptureDurationSec -LogFile $logFile

    Invoke-CaptureWindow -DurationSec $script:CaptureDurationSec `
        -Instruction "PHASE A (IoTService RUNNING): use the touchpad actively with the charger connected." `
        -LogFile $logFile | Out-Null

    $countA = Stop-HidMonitor -HidJob $hidJobA -LogFile $logFile
    $sizeA = if (Test-Path $hidPathA) { (Get-Item $hidPathA).Length } else { 0 }
    Write-TestLog "Phase A: $countA reports, $sizeA bytes -> $hidPathA" -LogFile $logFile

    if ($WithMiPC -and $miJobA) { Stop-MiPCDevMode -MiPCJob $miJobA -LogFile $logFile }

    # ---------- Phase B: IoTService STOPPED ----------
    Write-TestLog "PHASE B: Stopping IoTService - capturing HID reports." -LogFile $logFile
    sc.exe stop IoTSvc 2>&1 | ForEach-Object { Write-TestLog $_ -LogFile $logFile }
    Start-Sleep -Seconds 5
    $svcB = Get-Service -Name IoTSvc -ErrorAction SilentlyContinue
    Write-TestLog "IoTService status after stop: $($svcB.Status)" -LogFile $logFile

    $miJobB = $null
    if ($WithMiPC) { $miJobB = Start-MiPCDevMode -LogFile $logFile }

    $hidPathB = Join-Path $resultsDir "phaseB-svc-stopped-hid.log"
    $hidJobB = Start-HidMonitor -OutputPath $hidPathB -DurationSec $script:CaptureDurationSec -LogFile $logFile

    Invoke-CaptureWindow -DurationSec $script:CaptureDurationSec `
        -Instruction "PHASE B (IoTService STOPPED): use the touchpad actively with the charger connected." `
        -LogFile $logFile | Out-Null

    $countB = Stop-HidMonitor -HidJob $hidJobB -LogFile $logFile
    $sizeB = if (Test-Path $hidPathB) { (Get-Item $hidPathB).Length } else { 0 }
    Write-TestLog "Phase B: $countB reports, $sizeB bytes -> $hidPathB" -LogFile $logFile

    if ($WithMiPC -and $miJobB) { Stop-MiPCDevMode -MiPCJob $miJobB -LogFile $logFile }

    # ---------- Comparison summary ----------
    Write-TestLog "===== COMPARISON SUMMARY =====" -LogFile $logFile
    $summary = @"
Phase A (IoTService RUNNING): $countA reports, $sizeA bytes
  File: $hidPathA
Phase B (IoTService STOPPED): $countB reports, $sizeB bytes
  File: $hidPathB
Delta (A - B): $($countA - $countB) reports, $($sizeA - $sizeB) bytes
"@
    Write-TestLog $summary -LogFile $logFile
    $summaryPath = Join-Path $resultsDir "comparison-summary.txt"
    $summary | Out-File -FilePath $summaryPath -Encoding utf8
    Write-TestLog "Summary written to $summaryPath" -LogFile $logFile

    Write-Host ""
    Write-Host "  Phase A (svc running): $countA reports, $sizeA bytes" -ForegroundColor Cyan
    Write-Host "  Phase B (svc stopped): $countB reports, $sizeB bytes" -ForegroundColor Cyan
    Write-Host ""

    Write-TestFooter -TicketId $TicketId -LogFile $logFile -Result "INCONCLUSIVE"
    Write-Host "  Results saved to: $resultsDir" -ForegroundColor Green
}
catch {
    Write-TestLog "FATAL: $($_.Exception.Message)" -Level "ERROR" -LogFile $logFile
    Write-TestLog $_.ScriptStackTrace -Level "ERROR" -LogFile $logFile
    throw
}
finally {
    # Always restart IoTService if it was running before (or was started for Phase A)
    Write-TestLog "Ensuring IoTService is running again..." -LogFile $logFile
    try {
        $svc = Get-Service -Name IoTSvc -ErrorAction SilentlyContinue
        if ($svc -and $svc.Status -ne "Running") {
            sc.exe start IoTSvc 2>&1 | Out-Null
            Write-TestLog "IoTService restarted." -Level "OK" -LogFile $logFile
        }
    } catch {
        Write-TestLog "Error restarting IoTService: $($_.Exception.Message)" -Level "ERROR" -LogFile $logFile
    }
    $finalSnap = Get-SystemSnapshot -Label "post-revert" -LogFile $logFile
    Write-TestLog "S1-002 cleanup complete." -LogFile $logFile
}
