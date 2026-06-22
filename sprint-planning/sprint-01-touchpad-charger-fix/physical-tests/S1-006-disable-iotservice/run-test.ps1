#Requires -RunAsAdministrator
#Requires -Version 5.1
<#
    S1-006 - Disable IoTService
    Decisive test: if ghost touches vanish with IoTService disabled, the service
    is an amplifier. Requires a reboot for the service to fully stop.
#>
[CmdletBinding()]
param(
    [switch]$ResumeFrom,
    [switch]$WithMiPC
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot "..\_common\TestFramework.psm1") -Force

$TicketId = "S1-006"
$Title    = "Disable IoTService"
$session  = Initialize-TestEnvironment
$logFile  = Join-Path $session.LogsDir "$($TicketId)-$($session.Timestamp).log"
Write-TestHeader -TicketId $TicketId -Title $Title -LogFile $logFile

# Track original config for revert (loaded from state on resume)
$originalStartType = $null

try {
    if (-not $ResumeFrom) {
        # ---- Phase 1: Save config, disable service, prompt reboot ----
        Write-TestLog "PHASE 1: Saving IoTService config and disabling." -LogFile $logFile
        $svc = Get-Service -Name IoTSvc -ErrorAction Stop
        $originalStartType = $svc.StartType.ToString()
        Write-TestLog "Original IoTService StartType: $originalStartType" -LogFile $logFile
        Save-State -Name "$($TicketId)-original-starttype" -Value $originalStartType -LogFile $logFile

        $preSnap = Get-SystemSnapshot -Label "pre-disable" -LogFile $logFile
        Save-State -Name "$($TicketId)-pre-snapshot" -Value $preSnap -LogFile $logFile

        # Disable the service
        Write-TestLog "Disabling IoTService (sc.exe config start= disabled)..." -LogFile $logFile
        sc.exe config IoTSvc start= disabled 2>&1 | ForEach-Object { Write-TestLog $_ -LogFile $logFile }
        # Try to stop it now (may not fully stop until reboot)
        sc.exe stop IoTSvc 2>&1 | ForEach-Object { Write-TestLog $_ -LogFile $logFile }

        # Set reboot-resume
        Set-RebootResume -ScriptPath $PSCommandPath -Phase "PostReboot" -LogFile $logFile

        Write-Host ""
        Write-Host "  IoTService has been DISABLED." -ForegroundColor Yellow
        Write-Host "  A reboot is required for the service to fully stop." -ForegroundColor Yellow
        Write-Host "  After reboot + login, this script will resume automatically." -ForegroundColor Yellow
        Write-Host ""
        Write-Host "  Press ENTER, then reboot the machine." -ForegroundColor White
        Read-Host
        return
    }
    elseif ($ResumeFrom -eq "PostReboot") {
        # ---- Phase 2: Confirm disabled, capture, then revert ----
        Write-TestLog "PHASE 2: Resumed after reboot. Confirming IoTService is disabled." -LogFile $logFile
        Clear-RebootResume -LogFile $logFile

        $svc = Get-Service -Name IoTSvc -ErrorAction SilentlyContinue
        Write-TestLog "IoTService status: $($svc.Status) / StartType: $($svc.StartType)" -LogFile $logFile
        if ($svc.Status -eq "Running") {
            Write-TestLog "WARNING: IoTService is still running after reboot!" -Level "WARN" -LogFile $logFile
        }

        $postSnap = Get-SystemSnapshot -Label "post-disable" -LogFile $logFile

        # Results dir
        $resultsDir = Join-Path $session.ResultsDir "$($TicketId)-$($session.Timestamp)"
        New-Item -ItemType Directory -Path $resultsDir -Force | Out-Null

        # Optional miPC dev mode
        $miJob = $null
        if ($WithMiPC) { $miJob = Start-MiPCDevMode -LogFile $logFile }

        # 5-min capture window
        $hidPath = Join-Path $resultsDir "hid-reports.log"
        $hidJob = Start-HidMonitor -OutputPath $hidPath -DurationSec $script:CaptureDurationSec -LogFile $logFile

        Invoke-CaptureWindow -DurationSec $script:CaptureDurationSec `
            -Instruction "IoTService DISABLED: use the touchpad actively with the charger connected." `
            -LogFile $logFile | Out-Null

        $reportCount = Stop-HidMonitor -HidJob $hidJob -LogFile $logFile
        Write-TestLog "HID reports captured: $reportCount -> $hidPath" -LogFile $logFile

        if ($WithMiPC -and $miJob) { Stop-MiPCDevMode -MiPCJob $miJob -LogFile $logFile }

        Write-TestFooter -TicketId $TicketId -LogFile $logFile -Result "INCONCLUSIVE"
        Write-Host "  Results saved to: $resultsDir" -ForegroundColor Green
    }
    elseif ($ResumeFrom -eq "Cleanup") {
        Write-TestLog "CLEANUP phase: restoring IoTService." -LogFile $logFile
        Clear-RebootResume -LogFile $logFile
    }
    else {
        Write-TestLog "Unknown ResumeFrom phase: $ResumeFrom" -Level "ERROR" -LogFile $logFile
    }
}
catch {
    Write-TestLog "FATAL: $($_.Exception.Message)" -Level "ERROR" -LogFile $logFile
    Write-TestLog $_.ScriptStackTrace -Level "ERROR" -LogFile $logFile
    throw
}
finally {
    # ALWAYS revert IoTService to its original start type
    $orig = $originalStartType
    if (-not $orig) {
        $orig = Load-State -Name "$($TicketId)-original-starttype" -LogFile $logFile
    }
    if ($orig) {
        Revert-IoTService -OriginalStartType $orig -LogFile $logFile
        Clear-State -Name "$($TicketId)-original-starttype"
    }
    $finalSnap = Get-SystemSnapshot -Label "post-revert" -LogFile $logFile
    Write-TestLog "S1-006 cleanup complete." -LogFile $logFile
}
