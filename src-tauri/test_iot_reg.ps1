# Get IoTService log and registry - write to separate files
$logPath = "C:\ProgramData\MI\IoTService\service.log"
if (Test-Path $logPath) {
    Get-Content $logPath -Tail 50 | Out-File "c:\Users\mafsc\Documents\Projects\miPC\iot_log_tail.txt" -Encoding UTF8
    Write-Host "Log saved to iot_log_tail.txt"
} else {
    Write-Host "Log not found: $logPath"
}

# Registry
$out = "c:\Users\mafsc\Documents\Projects\miPC\iot_registry.txt"
"=== HKLM\SOFTWARE\MI ===" | Out-File $out -Encoding UTF8
$miKey = Get-ChildItem "HKLM:\SOFTWARE\MI" -ErrorAction SilentlyContinue
if ($miKey) {
    foreach ($key in $miKey) {
        "`nKey: $($key.PSChildName)" | Out-File $out -Append -Encoding UTF8
        $props = Get-ItemProperty $key.PSPath -ErrorAction SilentlyContinue
        if ($props) {
            $props.PSObject.Properties | Where-Object { $_.Name -notmatch "^PS" } | ForEach-Object {
                "  $($_.Name) = $($_.Value)" | Out-File $out -Append -Encoding UTF8
            }
        }
        $subKeys = Get-ChildItem $key.PSPath -ErrorAction SilentlyContinue
        foreach ($sub in $subKeys) {
            "  Sub: $($sub.PSChildName)" | Out-File $out -Append -Encoding UTF8
            $subProps = Get-ItemProperty $sub.PSPath -ErrorAction SilentlyContinue
            if ($subProps) {
                $subProps.PSObject.Properties | Where-Object { $_.Name -notmatch "^PS" } | ForEach-Object {
                    "    $($_.Name) = $($_.Value)" | Out-File $out -Append -Encoding UTF8
                }
            }
        }
    }
}
Write-Host "Registry saved to iot_registry.txt"
