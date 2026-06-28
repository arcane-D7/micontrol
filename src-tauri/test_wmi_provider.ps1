# Check WMI provider details
Write-Output "=== WMI Classes ==="
Get-WmiObject -Namespace ROOT\WMI -List | Where-Object { $_.Name -match "MICommon|HQWmi|Esif" } | ForEach-Object { Write-Output $_.Name }

Write-Output ""
Write-Output "=== MICommonInterface Provider ==="
$m = Get-WmiObject -Namespace ROOT\WMI -Class MICommonInterface
Write-Output "Path: $($m.__PATH)"
Write-Output "Server: $($m.__SERVER)"
Write-Output "Namespace: $($m.__NAMESPACE)"

Write-Output ""
Write-Output "=== WDM Provider Info ==="
# Check if the provider is a WDM driver
$prov = Get-WmiObject -Namespace ROOT\WMI -List | Where-Object { $_.Name -eq "MICommonInterface" }
Write-Output "Provider class: $($prov.Name)"
Write-Output "Derivation: $($prov.Derivation)"
foreach ($q in $prov.Qualifiers) {
    Write-Output "Qualifier: $($q.Name) = $($q.Value)"
}

Write-Output ""
Write-Output "=== Driver Service ==="
Get-Service IoTDriver -ErrorAction SilentlyContinue | Format-List Name,DisplayName,Status,StartType

Write-Output ""
Write-Output "=== IoTService Process ==="
Get-Process IoTService -ErrorAction SilentlyContinue | Format-List Id,ProcessName,Path

Write-Output ""
Write-Output "=== Try MiInterface with 32-byte buffer ==="
$scope = New-Object System.Management.ManagementScope("ROOT\WMI")
$path = New-Object System.Management.ManagementPath("MICommonInterface")
$opts = New-Object System.Management.ObjectGetOptions
$class = New-Object System.Management.ManagementClass($scope, $path, $opts)
$inParams = $class.GetMethodParameters("MiInterface")
$buf = New-Object byte[] 32
$buf[0] = 0xFA
$buf[4] = 0x05
$inParams["InData"] = $buf
try {
    $out = $m.InvokeMethod("MiInterface", $inParams, $null)
    Write-Output "ReturnCode: $($out['ReturnCode'])"
    $od = $out['OutData']
    if ($od) {
        $hex = ($od | ForEach-Object { '{0:02x}' -f $_ }) -join ' '
        Write-Output "OutData: $hex"
    } else {
        Write-Output "OutData: null"
    }
} catch {
    Write-Output "Error: $_"
}
