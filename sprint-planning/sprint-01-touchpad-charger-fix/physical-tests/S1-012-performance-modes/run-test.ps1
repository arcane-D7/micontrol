#Requires -RunAsAdministrator
#Requires -Version 5.1
<#
    S1-012 - Performance Modes
    Iterates through Silence (16W), Balance (32W), Smart (60W) TDP modes.
    For each: tries powercfg to switch, runs a 3-min capture, saves results.
    Reverts to the original power plan at the end.
#>
[CmdletBinding()]
param(
    [switch]$WithMiPC
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot "..\_common\TestFramework.psm1") -Force

$TicketId = "S1-012"
$Title    = "Performance Modes (TDP correlation)"
$session  = Initialize-TestEnvironment
$logFile  = Join-Path $session.LogsDir "$($TicketId)-$($session.Timestamp).log"
Write-TestHeader -TicketId $TicketId -Title $Title -LogFile $logFile

# 3-min capture per mode
$perModeSec = 180

# Mode definitions: name + keywords to match power plan names
$modes = @(
    @{ Id="silence"; Name="Silence"; Tdp="16W"; Keywords=@("silence","silent","quiet","节能") }
    @{ Id="balance"; Name="Balance"; Tdp="32W"; Keywords=@("balance","balanced","平衡","高性能") }
    @{ Id="smart";   Name="Smart";   Tdp="60W"; Keywords=@("smart","performance","turbo","野兽","极速") }
)

$originalPlanGuid = $null

try {
    $preSnap = Get-SystemSnapshot -Label "pre-modes" -LogFile $logFile
    # Save original active power plan GUID
    $activeRaw = powercfg /getactivescheme 2>&1 | Out-String
    if ($activeRaw -match 'GUID:\s*([0-9a-fA-F\-]+)') {
        $originalPlanGuid = $matches[1]
        Write-TestLog "Original active power plan GUID: $originalPlanGuid" -LogFile $logFile
        Save-State -Name "$($TicketId)-original-plan" -Value $originalPlanGuid -LogFile $logFile
    } else {
        Write-TestLog "Could not determine active power plan GUID." -Level "WARN" -LogFile $logFile
    }

    # Enumerate available power plans
    Write-TestLog "Available power plans:" -LogFile $logFile
    $plansRaw = powercfg /list 2>&1 | Out-String
    Write-TestLog $plansRaw -LogFile $logFile

    $resultsDir = Join-Path $session.ResultsDir "$($TicketId)-$($session.Timestamp)"
    New-Item -ItemType Directory -Path $resultsDir -Force | Out-Null

    # Optional miPC dev mode (start once for all modes)
    $miJob = $null
    if ($WithMiPC) { $miJob = Start-MiPCDevMode -LogFile $logFile }

    $summaryRows = @()

    foreach ($mode in $modes) {
        Write-Host ""
        Write-Host "  ===== MODE: $($mode.Name) ($($mode.Tdp)) =====" -ForegroundColor Cyan

        # Try to find a matching power plan GUID
        $matchedGuid = $null
        foreach ($line in ($plansRaw -split "`r?`n")) {
            foreach ($kw in $mode.Keywords) {
                if ($line -match "(?i)$kw" -and $line -match '([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})') {
                    $matchedGuid = $matches[1]
                    break
                }
            }
            if ($matchedGuid) { break }
        }

        if ($matchedGuid) {
            Write-TestLog "Switching to '$($mode.Name)' via powercfg (GUID $matchedGuid)." -LogFile $logFile
            powercfg /setactive $matchedGuid 2>&1 | Out-Null
            Start-Sleep -Seconds 2
        } else {
            Write-TestLog "No matching Windows power plan for '$($mode.Name)'." -Level "WARN" -LogFile $logFile
            Write-Host "  No matching Windows power plan found for '$($mode.Name)'." -ForegroundColor Yellow
            Write-Host "  Please switch to $($mode.Name) mode via the Xiaomi Fn shortcut," -ForegroundColor Yellow
            Write-Host "  then press ENTER to continue." -ForegroundColor Yellow
            Read-Host
        }

        # Confirm active plan
        $curActive = powercfg /getactivescheme 2>&1 | Out-String
        Write-TestLog "Active plan now: $($curActive.Trim())" -LogFile $logFile

        $modeDir = Join-Path $resultsDir $mode.Id
        New-Item -ItemType Directory -Path $modeDir -Force | Out-Null
        $modeSnap = Get-SystemSnapshot -Label $mode.Id -LogFile $logFile
        $modeSnap | ConvertTo-Json -Depth 5 | Out-File (Join-Path $modeDir "snapshot.json") -Encoding utf8

        # HID monitor for this mode
        $hidPath = Join-Path $modeDir "hid-reports.log"
        $hidJob = Start-HidMonitor -OutputPath $hidPath -DurationSec $perModeSec -LogFile $logFile

        Invoke-CaptureWindow -DurationSec $perModeSec `
            -Instruction "MODE: $($mode.Name) ($($mode.Tdp)) - use the touchpad actively with the charger connected." `
            -LogFile $logFile | Out-Null

        $reportCount = Stop-HidMonitor -HidJob $hidJob -LogFile $logFile
        $sizeBytes = if (Test-Path $hidPath) { (Get-Item $hidPath).Length } else { 0 }
        Write-TestLog "Mode '$($mode.Name)': $reportCount reports, $sizeBytes bytes" -LogFile $logFile

        $summaryRows += [ordered]@{
            Mode     = $mode.Name
            Tdp      = $mode.Tdp
            Reports  = $reportCount
            Bytes    = $sizeBytes
            HidFile  = $hidPath
        }
    }

    if ($WithMiPC -and $miJob) { Stop-MiPCDevMode -MiPCJob $miJob -LogFile $logFile }

    # ---------- Summary table ----------
    Write-TestLog "===== MODES SUMMARY =====" -LogFile $logFile
    Write-Host ""
    Write-Host "  ===== MODES SUMMARY =====" -ForegroundColor Cyan
    $header = "{0,-12} {1,6} {2,8} {3,10}" -f "Mode","TDP","Reports","Bytes"
    Write-TestLog $header -LogFile $logFile
    Write-Host $header -ForegroundColor White
    foreach ($r in $summaryRows) {
        $line = "{0,-12} {1,6} {2,8} {3,10}" -f $r.Mode, $r.Tdp, $r.Reports, $r.Bytes
        Write-TestLog $line -LogFile $logFile
        Write-Host $line
    }

    $summaryPath = Join-Path $resultsDir "modes-summary.csv"
    $csv = "Mode,TDP,Reports,Bytes,HidFile`r`n"
    foreach ($r in $summaryRows) {
        $csv += "$($r.Mode),$($r.Tdp),$($r.Reports),$($r.Bytes),$($r.HidFile)`r`n"
    }
    $csv | Out-File -FilePath $summaryPath -Encoding utf8
    Write-TestLog "Summary CSV written to $summaryPath" -LogFile $logFile

    Write-TestFooter -TicketId $TicketId -LogFile $logFile -Result "INCONCLUSIVE"
    Write-Host "  Results saved to: $resultsDir" -ForegroundColor Green
}
catch {
    Write-TestLog "FATAL: $($_.Exception.Message)" -Level "ERROR" -LogFile $logFile
    Write-TestLog $_.ScriptStackTrace -Level "ERROR" -LogFile $logFile
    Write-TestFooter -TicketId $TicketId -LogFile $logFile -Result "FAIL"
    throw
}
finally {
    # Always restore the original power plan
    if (-not $originalPlanGuid) {
        $originalPlanGuid = Load-State -Name "$($TicketId)-original-plan" -LogFile $logFile
    }
    if ($originalPlanGuid) {
        Revert-PowerPlan -OriginalGuid $originalPlanGuid -LogFile $logFile
        Clear-State -Name "$($TicketId)-original-plan"
    }
    $finalSnap = Get-SystemSnapshot -Label "post-revert" -LogFile $logFile
    Write-TestLog "S1-012 cleanup complete." -LogFile $logFile
}
