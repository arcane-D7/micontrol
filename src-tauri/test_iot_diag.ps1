# Write IoTService log and registry info to file
$outFile = "c:\Users\mafsc\Documents\Projects\miPC\iot_diag_output.txt"
"=== IoTService Log File (last 200 lines) ===" | Out-File $outFile

$logPath = "C:\ProgramData\MI\IoTService\service.log"
if (Test-Path $logPath) {
    $logContent = Get-Content $logPath -Tail 200
    $logContent | Out-File $outFile -Append
} else {
    "Log file not found at: $logPath" | Out-File $outFile -Append
}

"" | Out-File $outFile -Append
"=== Registry: HKLM\SOFTWARE\MI ===" | Out-File $outFile -Append
$miKey = Get-ChildItem "HKLM:\SOFTWARE\MI" -ErrorAction SilentlyContinue
if ($miKey) {
    $miKey | ForEach-Object {
        "Key: $($_.PSChildName)" | Out-File $outFile -Append
        $props = Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue
        if ($props) {
            $props.PSObject.Properties | Where-Object { $_.Name -notmatch "^PS" } | ForEach-Object {
                "  $($_.Name) = $($_.Value)" | Out-File $outFile -Append
            }
        }
        # Check subkeys
        $subKeys = Get-ChildItem $_.PSPath -ErrorAction SilentlyContinue
        $subKeys | ForEach-Object {
            "  SubKey: $($_.PSChildName)" | Out-File $outFile -Append
            $subProps = Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue
            if ($subProps) {
                $subProps.PSObject.Properties | Where-Object { $_.Name -notmatch "^PS" } | ForEach-Object {
                    "    $($_.Name) = $($_.Value)" | Out-File $outFile -Append
                }
            }
        }
    }
} else {
    "HKLM\SOFTWARE\MI not found" | Out-File $outFile -Append
}

"" | Out-File $outFile -Append
"=== IoTService Config Files ===" | Out-File $outFile -Append
$progDataMI = "C:\ProgramData\MI"
if (Test-Path $progDataMI) {
    Get-ChildItem $progDataMI -Recurse -ErrorAction SilentlyContinue | Select-Object FullName, Length, LastWriteTime | Format-Table -AutoSize | Out-File $outFile -Append
} else {
    "C:\ProgramData\MI not found" | Out-File $outFile -Append
}

Write-Host "Output written to $outFile"
