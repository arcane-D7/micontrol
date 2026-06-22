#Requires -RunAsAdministrator
#Requires -Version 5.1
<#
    S1-004 - ETW Trace
    Captures kernel-level ETW events from hidi2c, acpi, and HidClass providers
    during a 5-min capture window. Distinguishes I2C bus errors from analog
    corruption. No config to revert (ETW is read-only).
#>
[CmdletBinding()]
param(
    [switch]$WithMiPC
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot "..\_common\TestFramework.psm1") -Force

$TicketId = "S1-004"
$Title    = "ETW Trace"
$session  = Initialize-TestEnvironment
$logFile  = Join-Path $session.LogsDir "$($TicketId)-$($session.Timestamp).log"
Write-TestHeader -TicketId $TicketId -Title $Title -LogFile $logFile

try {
    $preSnap = Get-SystemSnapshot -Label "pre-etw" -LogFile $logFile

    $resultsDir = Join-Path $session.ResultsDir "$($TicketId)-$($session.Timestamp)"
    New-Item -ItemType Directory -Path $resultsDir -Force | Out-Null

    # Start ETW capture
    $etlPath = Join-Path $resultsDir "MiPC_EMI_Trace"
    $etwStarted = Start-EtwCapture -OutputPath $etlPath -LogFile $logFile
    if (-not $etwStarted) {
        Write-TestLog "ETW capture could not be started. Aborting test." -Level "ERROR" -LogFile $logFile
        Write-TestFooter -TicketId $TicketId -LogFile $logFile -Result "FAIL"
        return
    }

    # Optional miPC dev mode
    $miJob = $null
    if ($WithMiPC) { $miJob = Start-MiPCDevMode -LogFile $logFile }

    # Also start HID monitor for cross-reference
    $hidPath = Join-Path $resultsDir "hid-reports.log"
    $hidJob = Start-HidMonitor -OutputPath $hidPath -DurationSec $script:CaptureDurationSec -LogFile $logFile

    # 5-min capture window - user MUST reproduce ghost touches actively
    Invoke-CaptureWindow -DurationSec $script:CaptureDurationSec `
        -Instruction "ETW CAPTURE: actively reproduce ghost touches - use touchpad with charger connected." `
        -LogFile $logFile | Out-Null

    $reportCount = Stop-HidMonitor -HidJob $hidJob -LogFile $logFile
    Write-TestLog "HID reports captured: $reportCount -> $hidPath" -LogFile $logFile

    if ($WithMiPC -and $miJob) { Stop-MiPCDevMode -MiPCJob $miJob -LogFile $logFile }

    # Stop ETW capture
    Stop-EtwCapture -LogFile $logFile

    # Locate the .etl file(s)
    $etlFiles = Get-ChildItem -Path $resultsDir -Filter "*.etl" -ErrorAction SilentlyContinue
    if ($etlFiles) {
        foreach ($f in $etlFiles) {
            Write-TestLog "ETL file: $($f.FullName) ($($f.Length) bytes)" -LogFile $logFile
        }
    } else {
        Write-TestLog "No .etl file found in $resultsDir - check WPR default output." -Level "WARN" -LogFile $logFile
    }

    # Analysis instructions
    $analysisNotes = @"
ETW Trace Analysis - $TicketId
================================
Captured providers: Microsoft-Windows-HidClass, Microsoft-Windows-Hidi2c, Microsoft-Windows-Kernel-PnP

To analyze:
1. Install Windows Performance Toolkit (WPT) via Windows ADK.
2. Open the .etl file in Windows Performance Analyzer (WPA).
3. Look for:
   - I2C transfer errors / NACKs in the hidi2c provider events.
   - ACPI power transition events around the ghost-touch timestamps.
   - HidClass input report gaps or bursts.
   - Correlation between IoTService EC writes (if visible) and HID anomalies.

If I2C errors appear at ghost-touch moments -> digital bus problem (D3hot, signal integrity).
If HID reports are clean but pointer jumps -> analog corruption before the ADC.
"@
    $notesPath = Join-Path $resultsDir "ANALYSIS-INSTRUCTIONS.txt"
    $analysisNotes | Out-File -FilePath $notesPath -Encoding utf8
    Write-TestLog "Analysis instructions written to $notesPath" -LogFile $logFile

    Write-TestFooter -TicketId $TicketId -LogFile $logFile -Result "INCONCLUSIVE"
    Write-Host "  Results saved to: $resultsDir" -ForegroundColor Green
    Write-Host "  Open the .etl in WPA to analyze." -ForegroundColor Green
}
catch {
    Write-TestLog "FATAL: $($_.Exception.Message)" -Level "ERROR" -LogFile $logFile
    Write-TestLog $_.ScriptStackTrace -Level "ERROR" -LogFile $logFile
    # Best-effort stop ETW on error
    try { Stop-EtwCapture -LogFile $logFile } catch { }
    Write-TestFooter -TicketId $TicketId -LogFile $logFile -Result "FAIL"
    throw
}
finally {
    $finalSnap = Get-SystemSnapshot -Label "post-etw" -LogFile $logFile
    Write-TestLog "S1-004 complete. No config to revert (ETW is read-only)." -LogFile $logFile
}
