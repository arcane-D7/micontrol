#Requires -RunAsAdministrator
#Requires -Version 5.1
<#
    S1-008 - EC Reset (baseline)
    Mostly manual: user holds power button 30s. Script captures pre/post snapshots
    and runs a 5-min capture window after the reset. No config to revert.
#>
[CmdletBinding()]
param(
    [switch]$ResumeFrom,
    [switch]$WithMiPC
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot "..\_common\TestFramework.psm1") -Force

$TicketId = "S1-008"
$Title    = "EC Reset (baseline)"
$session  = Initialize-TestEnvironment
$logFile  = Join-Path $session.LogsDir "$($TicketId)-$($session.Timestamp).log"
Write-TestHeader -TicketId $TicketId -Title $Title -LogFile $logFile

try {
    if (-not $ResumeFrom) {
        # ---- Phase 1: Pre-reset snapshot + instructions ----
        Write-TestLog "PHASE 1: Pre-reset snapshot and user instructions." -LogFile $logFile
        $preSnap = Get-SystemSnapshot -Label "pre-ec-reset" -LogFile $logFile
        Save-State -Name "$($TicketId)-pre-snapshot" -Value $preSnap -LogFile $logFile

        Write-Host ""
        Write-Host "  ===== EC RESET - MANUAL STEPS =====" -ForegroundColor Cyan
        Write-Host "  1. UNPLUG the charger from the laptop." -ForegroundColor Yellow
        Write-Host "  2. Shut down Windows normally (Start > Power > Shut down)." -ForegroundColor Yellow
        Write-Host "  3. Once powered off, PRESS AND HOLD the power button for 30 seconds." -ForegroundColor Yellow
        Write-Host "     (This performs a hard EC reset / static discharge.)" -ForegroundColor Yellow
        Write-Host "  4. Release the power button and WAIT 10 seconds." -ForegroundColor Yellow
        Write-Host "  5. PLUG the charger back in." -ForegroundColor Yellow
        Write-Host "  6. Power the laptop back on." -ForegroundColor Yellow
        Write-Host "  7. Windows will auto-launch this script after login (RunOnce)." -ForegroundColor Yellow
        Write-Host ""
        Write-Host "  Press ENTER after reading to set the reboot-resume hook," -ForegroundColor White
        Write-Host "  then proceed with the shutdown." -ForegroundColor White
        Read-Host "  Press ENTER to continue"

        # Set RunOnce to resume after reboot
        Set-RebootResume -ScriptPath $PSCommandPath -Phase "PostReboot" -LogFile $logFile
        Write-TestLog "RunOnce hook set. User will now shut down and perform the 30s power hold." -LogFile $logFile
        Write-Host ""
        Write-Host "  Reboot-resume hook installed. You may now shut down and perform the reset." -ForegroundColor Green
        Write-Host "  After reboot + login, this script will resume automatically." -ForegroundColor Green
        return
    }
    elseif ($ResumeFrom -eq "PostReboot") {
        # ---- Phase 2: Post-reset snapshot + capture ----
        Write-TestLog "PHASE 2: Resumed after reboot. Taking post-reset snapshot." -LogFile $logFile
        Clear-RebootResume -LogFile $logFile

        $postSnap = Get-SystemSnapshot -Label "post-ec-reset" -LogFile $logFile
        Save-State -Name "$($TicketId)-post-snapshot" -Value $postSnap -LogFile $logFile

        # Compare key fields
        $pre = Load-State -Name "$($TicketId)-pre-snapshot" -LogFile $logFile
        if ($pre) {
            Write-TestLog "--- Pre/Post comparison ---" -LogFile $logFile
            foreach ($k in @("BatteryPercent","PowerPlanGuid","IoTServiceStatus","IdleTimerPeriod")) {
                $pv = $pre.$k; $qv = $postSnap.$k
                $same = if ($pv -eq $qv) { "SAME" } else { "CHANGED" }
                Write-TestLog ("  {0,-22} pre={1} post={2} [{3}]" -f $k,$pv,$qv,$same) -LogFile $logFile
            }
        }

        # Results dir for this run
        $resultsDir = Join-Path $session.ResultsDir "$($TicketId)-$($session.Timestamp)"
        New-Item -ItemType Directory -Path $resultsDir -Force | Out-Null

        # Optional miPC dev mode
        $miJob = $null
        if ($WithMiPC) {
            $miJob = Start-MiPCDevMode -LogFile $logFile
        }

        # 5-min capture window
        $hidPath = Join-Path $resultsDir "hid-reports.log"
        $hidJob = Start-HidMonitor -OutputPath $hidPath -DurationSec $script:CaptureDurationSec -LogFile $logFile

        $completed = Invoke-CaptureWindow -DurationSec $script:CaptureDurationSec `
            -Instruction "POST EC-RESET: use the touchpad actively with the charger connected." `
            -LogFile $logFile `
            -OnStart { } `
            -OnStop { }

        $reportCount = Stop-HidMonitor -HidJob $hidJob -LogFile $logFile
        Write-TestLog "HID reports captured: $reportCount -> $hidPath" -LogFile $logFile

        if ($WithMiPC -and $miJob) { Stop-MiPCDevMode -MiPCJob $miJob -LogFile $logFile }

        # Final snapshot
        $finalSnap = Get-SystemSnapshot -Label "post-ec-reset-final" -LogFile $logFile
        Write-TestFooter -TicketId $TicketId -LogFile $logFile -Result "INCONCLUSIVE"
        Write-Host "  Results saved to: $resultsDir" -ForegroundColor Green
        Write-Host "  Review HID report log for ghost-touch evidence." -ForegroundColor Green
    }
    else {
        Write-TestLog "Unknown ResumeFrom phase: $ResumeFrom" -Level "ERROR" -LogFile $logFile
        Write-TestFooter -TicketId $TicketId -LogFile $logFile -Result "FAIL"
    }
}
catch {
    Write-TestLog "FATAL: $($_.Exception.Message)" -Level "ERROR" -LogFile $logFile
    Write-TestLog $_.ScriptStackTrace -Level "ERROR" -LogFile $logFile
    Write-TestFooter -TicketId $TicketId -LogFile $logFile -Result "FAIL"
    throw
}
