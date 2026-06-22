#Requires -RunAsAdministrator
#Requires -Version 5.1
<#
    S1-007 - Force I2C D0
    Sets the Intel Quick I2C controller IdleTimerPeriod to 0 so it never enters
    D3hot idle. If ghost touches vanish, D0↔D3hot transitions are a factor.
#>
[CmdletBinding()]
param(
    [switch]$ResumeFrom,
    [switch]$WithMiPC
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot "..\_common\TestFramework.psm1") -Force

$TicketId = "S1-007"
$Title    = "Force I2C D0"
$session  = Initialize-TestEnvironment
$logFile  = Join-Path $session.LogsDir "$($TicketId)-$($session.Timestamp).log"
Write-TestHeader -TicketId $TicketId -Title $Title -LogFile $logFile

# State tracked for revert
$deviceId = $null
$originalIdle = $null

try {
    if (-not $ResumeFrom) {
        # ---- Phase 1: Read current IdleTimerPeriod, set to 0, prompt reboot ----
        Write-TestLog "PHASE 1: Locating I2C controller and reading IdleTimerPeriod." -LogFile $logFile
        $deviceId = Get-I2cControllerDeviceId
        if (-not $deviceId) {
            throw "Intel Quick I2C controller (PCI\VEN_8086&DEV_E448) not found."
        }
        Write-TestLog "I2C controller device ID: $deviceId" -LogFile $logFile
        Save-State -Name "$($TicketId)-deviceid" -Value $deviceId -LogFile $logFile

        $regPath = "HKLM:\SYSTEM\CurrentControlSet\Enum\$deviceId\Device Parameters"
        $existing = Get-ItemProperty -Path $regPath -Name IdleTimerPeriod -ErrorAction SilentlyContinue
        if ($null -ne $existing -and $null -ne $existing.IdleTimerPeriod) {
            $originalIdle = [int]$existing.IdleTimerPeriod
        } else {
            $originalIdle = $null  # value did not exist
        }
        Write-TestLog "Original IdleTimerPeriod: $(if ($null -eq $originalIdle) {'<not set>'} else {$originalIdle})" -LogFile $logFile
        Save-State -Name "$($TicketId)-original-idletimer" -Value $originalIdle -LogFile $logFile

        $preSnap = Get-SystemSnapshot -Label "pre-d0-force" -LogFile $logFile
        Save-State -Name "$($TicketId)-pre-snapshot" -Value $preSnap -LogFile $logFile

        # Set IdleTimerPeriod = 0 (disable D3hot idle)
        Write-TestLog "Setting IdleTimerPeriod=0 (force D0)..." -LogFile $logFile
        reg add "HKLM\SYSTEM\CurrentControlSet\Enum\$deviceId\Device Parameters" /v IdleTimerPeriod /t REG_DWORD /d 0 /f 2>&1 |
            ForEach-Object { Write-TestLog $_ -LogFile $logFile }

        Set-RebootResume -ScriptPath $PSCommandPath -Phase "PostReboot" -LogFile $logFile

        Write-Host ""
        Write-Host "  IdleTimerPeriod set to 0 (D0 forced)." -ForegroundColor Yellow
        Write-Host "  A reboot is required for the change to take effect." -ForegroundColor Yellow
        Write-Host "  After reboot + login, this script will resume automatically." -ForegroundColor Yellow
        Write-Host ""
        Write-Host "  Press ENTER, then reboot the machine." -ForegroundColor White
        Read-Host
        return
    }
    elseif ($ResumeFrom -eq "PostReboot") {
        # ---- Phase 2: Confirm D0, capture, then revert ----
        Write-TestLog "PHASE 2: Resumed after reboot. Confirming IdleTimerPeriod=0." -LogFile $logFile
        Clear-RebootResume -LogFile $logFile

        $deviceId = Load-State -Name "$($TicketId)-deviceid" -LogFile $logFile
        $originalIdle = Load-State -Name "$($TicketId)-original-idletimer" -LogFile $logFile

        if ($deviceId) {
            $regPath = "HKLM:\SYSTEM\CurrentControlSet\Enum\$deviceId\Device Parameters"
            $cur = (Get-ItemProperty -Path $regPath -Name IdleTimerPeriod -ErrorAction SilentlyContinue).IdleTimerPeriod
            Write-TestLog "Current IdleTimerPeriod: $cur (expected 0)" -LogFile $logFile
        }

        $postSnap = Get-SystemSnapshot -Label "post-d0-force" -LogFile $logFile

        $resultsDir = Join-Path $session.ResultsDir "$($TicketId)-$($session.Timestamp)"
        New-Item -ItemType Directory -Path $resultsDir -Force | Out-Null

        $miJob = $null
        if ($WithMiPC) { $miJob = Start-MiPCDevMode -LogFile $logFile }

        $hidPath = Join-Path $resultsDir "hid-reports.log"
        $hidJob = Start-HidMonitor -OutputPath $hidPath -DurationSec $script:CaptureDurationSec -LogFile $logFile

        Invoke-CaptureWindow -DurationSec $script:CaptureDurationSec `
            -Instruction "I2C forced D0: use the touchpad actively with the charger connected." `
            -LogFile $logFile | Out-Null

        $reportCount = Stop-HidMonitor -HidJob $hidJob -LogFile $logFile
        Write-TestLog "HID reports captured: $reportCount -> $hidPath" -LogFile $logFile

        if ($WithMiPC -and $miJob) { Stop-MiPCDevMode -MiPCJob $miJob -LogFile $logFile }

        Write-TestFooter -TicketId $TicketId -LogFile $logFile -Result "INCONCLUSIVE"
        Write-Host "  Results saved to: $resultsDir" -ForegroundColor Green
    }
    elseif ($ResumeFrom -eq "Cleanup") {
        Write-TestLog "CLEANUP phase: restoring IdleTimerPeriod." -LogFile $logFile
        Clear-RebootResume -LogFile $logFile
        $deviceId = Load-State -Name "$($TicketId)-deviceid" -LogFile $logFile
        $originalIdle = Load-State -Name "$($TicketId)-original-idletimer" -LogFile $logFile
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
    # ALWAYS restore the original IdleTimerPeriod
    if (-not $deviceId) { $deviceId = Load-State -Name "$($TicketId)-deviceid" -LogFile $logFile }
    if (-not $originalIdle) { $originalIdle = Load-State -Name "$($TicketId)-original-idletimer" -LogFile $logFile }
    if ($deviceId) {
        Revert-IdleTimer -DeviceId $deviceId -OriginalValue $originalIdle -LogFile $logFile
        Clear-State -Name "$($TicketId)-deviceid"
        Clear-State -Name "$($TicketId)-original-idletimer"
    }
    $finalSnap = Get-SystemSnapshot -Label "post-revert" -LogFile $logFile
    Write-TestLog "S1-007 cleanup complete." -LogFile $logFile
}
