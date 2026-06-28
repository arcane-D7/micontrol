# Test MiInterface with correct InData format from ACPI AML analysis
# The method signature is: MiInterface(System.Byte[] InData)
# Returns: ManagementBaseObject with OutData (Byte[]) and ReturnCode (UInt16)

$m = Get-WmiObject -Namespace ROOT\WMI -Class MICommonInterface
Write-Output "Instance: $($m.InstanceName)"

# Get method parameters class
$scope = New-Object System.Management.ManagementScope("ROOT\WMI")
$path = New-Object System.Management.ManagementPath("MICommonInterface")
$opts = New-Object System.Management.ObjectGetOptions
$class = New-Object System.Management.ManagementClass($scope, $path, $opts)

# Test with different buffer sizes
Write-Output ""
Write-Output "Testing different buffer sizes:"

# 1-byte buffer
$buf1 = [byte[]](0x00)
$inParams = $class.GetMethodParameters("MiInterface")
$inParams["InData"] = $buf1
try {
    $out = $m.InvokeMethod("MiInterface", $inParams, $null)
    Write-Output "  1-byte: RC=$($out['ReturnCode']) OutData=$($out['OutData'])"
} catch {
    Write-Output "  1-byte ERROR: $_"
}

# 4-byte buffer
$buf4 = [byte[]](0x00, 0x00, 0x00, 0x00)
$inParams = $class.GetMethodParameters("MiInterface")
$inParams["InData"] = $buf4
try {
    $out = $m.InvokeMethod("MiInterface", $inParams, $null)
    Write-Output "  4-byte: RC=$($out['ReturnCode']) OutData=$($out['OutData'])"
} catch {
    Write-Output "  4-byte ERROR: $_"
}

# 8-byte buffer
$buf8 = [byte[]](0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00)
$inParams = $class.GetMethodParameters("MiInterface")
$inParams["InData"] = $buf8
try {
    $out = $m.InvokeMethod("MiInterface", $inParams, $null)
    Write-Output "  8-byte: RC=$($out['ReturnCode']) OutData=$($out['OutData'])"
} catch {
    Write-Output "  8-byte ERROR: $_"
}

# 16-byte buffer
$buf16 = [byte[]](0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00)
$inParams = $class.GetMethodParameters("MiInterface")
$inParams["InData"] = $buf16
try {
    $out = $m.InvokeMethod("MiInterface", $inParams, $null)
    Write-Output "  16-byte: RC=$($out['ReturnCode']) OutData=$($out['OutData'])"
} catch {
    Write-Output "  16-byte ERROR: $_"
}

# 32-byte buffer (all zeros)
$buf32 = New-Object byte[] 32
$inParams = $class.GetMethodParameters("MiInterface")
$inParams["InData"] = $buf32
try {
    $out = $m.InvokeMethod("MiInterface", $inParams, $null)
    Write-Output "  32-byte zeros: RC=$($out['ReturnCode']) OutData=$($out['OutData'])"
} catch {
    Write-Output "  32-byte zeros ERROR: $_"
}

# 32-byte buffer with 0x55 pattern
$buf32b = [byte[]]@(0x55) * 32
$inParams = $class.GetMethodParameters("MiInterface")
$inParams["InData"] = $buf32b
try {
    $out = $m.InvokeMethod("MiInterface", $inParams, $null)
    Write-Output "  32-byte 0x55: RC=$($out['ReturnCode']) OutData=$($out['OutData'])"
} catch {
    Write-Output "  32-byte 0x55 ERROR: $_"
}

# Try using SWbemObject directly
Write-Output ""
Write-Output "Using SWbemObject ExecMethod:"
$wbem = New-Object -ComObject WbemScripting.SWbemLocator
$svc = $wbem.ConnectServer(".", "root\wmi")
$miObj = $svc.Get('MICommonInterface.InstanceName="ACPI\\PNP0C14\\MIFS_0"')
$miMethod = $miObj.Methods_.Item("MiInterface")
$miParams = $miMethod.InParameters.SpawnInstance_()
$miParams.Properties_.Item("InData").Value = [byte[]](0x00, 0x01, 0x02, 0x03)
try {
    $out = $miObj.ExecMethod_("MiInterface", $miParams)
    Write-Output "  SWbem 4-byte: RC=$($out.Properties_.Item('ReturnCode').Value) OutData=$($out.Properties_.Item('OutData').Value)"
} catch {
    Write-Output "  SWbem 4-byte ERROR: $_"
}

# Try with 32-byte via SWbemObject
$miParams2 = $miMethod.InParameters.SpawnInstance_()
$miParams2.Properties_.Item("InData").Value = New-Object byte[] 32
try {
    $out = $miObj.ExecMethod_("MiInterface", $miParams2)
    Write-Output "  SWbem 32-byte: RC=$($out.Properties_.Item('ReturnCode').Value) OutData=$($out.Properties_.Item('OutData').Value)"
} catch {
    Write-Output "  SWbem 32-byte ERROR: $_"
}
