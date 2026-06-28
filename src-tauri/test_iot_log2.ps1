# Check IoTService log for our IPC message
$logPath = "C:\ProgramData\MI\IoTService\service.log"
if (Test-Path $logPath) {
    Write-Host "=== Last 20 lines of IoTService log ==="
    Get-Content $logPath -Tail 20 | ForEach-Object { Write-Host $_ }
} else {
    Write-Host "Log not found"
}
