#Requires -RunAsAdministrator
#Requires -Version 5.1
<#
    S1-010 - Charger/Grounding Matrix
    Iterates through physical conditions (charger type, outlet, grounding, battery
    level). For each condition: prompts the user to set up, runs a 2-min capture,
    saves per-condition results. Generates a summary table at the end.
#>
[CmdletBinding()]
param(
    [switch]$WithMiPC
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot "..\_common\TestFramework.psm1") -Force

$TicketId = "S1-010"
$Title    = "Charger/Grounding Matrix"
$session  = Initialize-TestEnvironment
$logFile  = Join-Path $session.LogsDir "$($TicketId)-$($session.Timestamp).log"
Write-TestHeader -TicketId $TicketId -Title $Title -LogFile $logFile

# Shorter capture per condition (many conditions to test)
$perConditionSec = 120

# Define the matrix of conditions
$conditions = @(
    @{ Id="01-original-xiaomi";   Name="Original Xiaomi charger" }
    @{ Id="02-thirdparty-usbc";   Name="Third-party USB-C charger" }
    @{ Id="03-grounded-3prong";   Name="3-prong grounded charger" }
    @{ Id="04-different-outlet";  Name="Different wall outlet" }
    @{ Id="05-ferrite-core";      Name="Ferrite core on DC cable" }
    @{ Id="06-battery-low";       Name="Battery < 20%" }
    @{ Id="07-battery-high";      Name="Battery > 90%" }
)

try {
    $preSnap = Get-SystemSnapshot -Label "pre-matrix" -LogFile $logFile

    $resultsDir = Join-Path $session.ResultsDir "$($TicketId)-$($session.Timestamp)"
    New-Item -ItemType Directory -Path $resultsDir -Force | Out-Null

    # Optional miPC dev mode (start once for all conditions)
    $miJob = $null
    if ($WithMiPC) { $miJob = Start-MiPCDevMode -LogFile $logFile }

    $summaryRows = @()

    foreach ($cond in $conditions) {
        Write-Host ""
        Write-Host "  ===== CONDITION: $($cond.Name) =====" -ForegroundColor Cyan
        Write-Host "  Please set up the physical condition now:" -ForegroundColor Yellow
        Write-Host "    - $($cond.Name)" -ForegroundColor Yellow
        Write-Host "  When ready, press ENTER to start the 2-minute capture." -ForegroundColor White
        Read-Host "  Press ENTER when ready"

        $condDir = Join-Path $resultsDir $cond.Id
        New-Item -ItemType Directory -Path $condDir -Force | Out-Null

        # Snapshot for this condition
        $condSnap = Get-SystemSnapshot -Label $cond.Id -LogFile $logFile
        $condSnap | ConvertTo-Json -Depth 5 | Out-File (Join-Path $condDir "snapshot.json") -Encoding utf8

        # HID monitor for this condition
        $hidPath = Join-Path $condDir "hid-reports.log"
        $hidJob = Start-HidMonitor -OutputPath $hidPath -DurationSec $perConditionSec -LogFile $logFile

        Invoke-CaptureWindow -DurationSec $perConditionSec `
            -Instruction "CONDITION: $($cond.Name) - use the touchpad actively with the charger connected." `
            -LogFile $logFile | Out-Null

        $reportCount = Stop-HidMonitor -HidJob $hidJob -LogFile $logFile
        $sizeBytes = if (Test-Path $hidPath) { (Get-Item $hidPath).Length } else { 0 }
        Write-TestLog "Condition '$($cond.Name)': $reportCount reports, $sizeBytes bytes" -LogFile $logFile

        $summaryRows += [ordered]@{
            Condition   = $cond.Name
            Reports     = $reportCount
            Bytes       = $sizeBytes
            BatteryPct  = $condSnap.BatteryPercent
            HidFile     = $hidPath
        }
    }

    if ($WithMiPC -and $miJob) { Stop-MiPCDevMode -MiPCJob $miJob -LogFile $logFile }

    # ---------- Summary table ----------
    Write-TestLog "===== MATRIX SUMMARY =====" -LogFile $logFile
    Write-Host ""
    Write-Host "  ===== MATRIX SUMMARY =====" -ForegroundColor Cyan
    $header = "{0,-32} {1,8} {2,10} {3,10}" -f "Condition","Reports","Bytes","Battery%"
    Write-TestLog $header -LogFile $logFile
    Write-Host $header -ForegroundColor White
    foreach ($r in $summaryRows) {
        $line = "{0,-32} {1,8} {2,10} {3,10}" -f $r.Condition, $r.Reports, $r.Bytes, $r.BatteryPct
        Write-TestLog $line -LogFile $logFile
        Write-Host $line
    }

    $summaryPath = Join-Path $resultsDir "matrix-summary.csv"
    $csv = "Condition,Reports,Bytes,BatteryPct,HidFile`r`n"
    foreach ($r in $summaryRows) {
        $csv += "$($r.Condition),$($r.Reports),$($r.Bytes),$($r.BatteryPct),$($r.HidFile)`r`n"
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
    $finalSnap = Get-SystemSnapshot -Label "post-matrix" -LogFile $logFile
    Write-TestLog "S1-010 complete. No config to revert (physical conditions only)." -LogFile $logFile
}
