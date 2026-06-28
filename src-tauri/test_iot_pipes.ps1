# List only IoT/Mi related named pipes
Write-Host "=== IoT/Mi Related Named Pipes ==="
$pipes = [System.IO.Directory]::GetFiles("\\.\pipe\")
$iotPipes = $pipes | Where-Object { $_ -match "IoT|Mi|IPC|Broker|bt_ipc" }
$iotPipes | ForEach-Object { Write-Host $_ }

Write-Host ""
Write-Host "=== IoTService Process ==="
$svc = Get-CimInstance Win32_Process -Filter "Name='IoTService.exe'"
$svc | Select-Object ProcessId, CommandLine, ExecutablePath | Format-List

Write-Host ""
Write-Host "=== IoTService Modules ==="
$proc = Get-Process -Name IoTService -ErrorAction SilentlyContinue
if ($proc) {
    $proc.Modules | Select-Object ModuleName, FileName | Format-Table -AutoSize
}

Write-Host ""
Write-Host "=== IoTService Handles (pipes) ==="
# Use handle.exe if available, otherwise skip
$handleExe = Get-Command handle.exe -ErrorAction SilentlyContinue
if ($handleExe) {
    $handleOutput = & handle.exe -p $proc.Id -accepteula 2>&1
    $handleOutput | Select-String "pipe" | ForEach-Object { Write-Host $_ }
} else {
    Write-Host "handle.exe not available"
}

Write-Host ""
Write-Host "=== IoTService Network Connections ==="
Get-NetTCPConnection -OwningProcess $proc.Id -ErrorAction SilentlyContinue | Format-Table -AutoSize
