# Check IoTService log file
Write-Host "=== IoTService Log File ==="
$logPath = "C:\ProgramData\MI\IoTService\service.log"
if (Test-Path $logPath) {
    $logContent = Get-Content $logPath -Tail 100
    $logContent | ForEach-Object { Write-Host $_ }
} else {
    Write-Host "Log file not found at: $logPath"
    # Check if directory exists
    $dir = Split-Path $logPath
    Write-Host "Directory exists: $(Test-Path $dir)"
    if (Test-Path $dir) {
        Write-Host "Directory contents:"
        Get-ChildItem $dir -Recurse | Select-Object FullName, Length, LastWriteTime | Format-Table -AutoSize
    }
}

Write-Host ""
Write-Host "=== Registry: SOFTWARE\MI\IoTDriver ==="
$reg = Get-ItemProperty "HKLM:\SOFTWARE\MI\IoTDriver" -ErrorAction SilentlyContinue
if ($reg) {
    $reg | Format-List
} else {
    Write-Host "Registry key not found"
}

Write-Host ""
Write-Host "=== Registry: SOFTWARE\MI\IoTService ==="
$reg2 = Get-ItemProperty "HKLM:\SOFTWARE\MI\IoTService" -ErrorAction SilentlyContinue
if ($reg2) {
    $reg2 | Format-List
} else {
    Write-Host "Registry key not found"
}

Write-Host ""
Write-Host "=== Registry: SOFTWARE\MI (all subkeys) ==="
$miKey = Get-ChildItem "HKLM:\SOFTWARE\MI" -ErrorAction SilentlyContinue
if ($miKey) {
    $miKey | ForEach-Object {
        Write-Host "Key: $($_.PSChildName)"
        $props = Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue
        if ($props) {
            $props.PSObject.Properties | Where-Object { $_.Name -notmatch "^PS" } | ForEach-Object {
                Write-Host "  $($_.Name) = $($_.Value)"
            }
        }
    }
} else {
    Write-Host "HKLM\SOFTWARE\MI not found"
}
