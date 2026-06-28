# List all named pipes and filter for IoT/Mi related ones
Write-Host "=== Named Pipes (IoT/Mi related) ==="
$pipes = [System.IO.Directory]::GetFiles("\\.\pipe\")
$pipes | Where-Object { $_ -match "IoT|Mi|IPC|Broker" } | ForEach-Object { Write-Host $_ }

Write-Host ""
Write-Host "=== All Named Pipes ==="
$pipes | ForEach-Object { Write-Host $_ }

Write-Host ""
Write-Host "=== IoTService Process Info ==="
Get-Process -Name IoTService -ErrorAction SilentlyContinue | Select-Object Id, ProcessName, Path, StartTime | Format-List

Write-Host ""
Write-Host "=== IoTService Modules ==="
$proc = Get-Process -Name IoTService -ErrorAction SilentlyContinue
if ($proc) {
    $proc.Modules | Select-Object ModuleName, FileName | Format-Table -AutoSize
}

Write-Host ""
Write-Host "=== IoTService Command Line ==="
Get-CimInstance Win32_Process -Filter "Name='IoTService.exe'" | Select-Object ProcessId, CommandLine | Format-List
