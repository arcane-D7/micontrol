# Simpler test - just EC RAM IOCTL with proper error handling
Write-Host "=== ADMIN CHECK ==="
$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object System.Security.Principal.WindowsPrincipal($identity)
Write-Host "IsAdmin: $($principal.IsInRole([System.Security.Principal.WindowsBuiltInRole]::Administrator))"

Write-Host ""
Write-Host "=== EC RAM IOCTL Test ==="

# Find IoT device
$guid = [Guid]"AB7924A1-3162-4010-B33B-837E87E25FBC"

# Use .NET to find device
$devicePath = $null

# Try using WMI to find the device
try {
    $devQuery = "SELECT * FROM Win32_PnPEntity WHERE DeviceID LIKE '%IOTD%'"
    $devs = Get-CimInstance -Query $devQuery -ErrorAction SilentlyContinue
    foreach ($d in $devs) {
        Write-Host "Found device: $($d.Name) - $($d.DeviceID)"
    }
} catch {
    Write-Host "WMI device query error: $_"
}

# Try using SetupAPI with proper struct sizes
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public class IoTSetup {
    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr SetupDiGetClassDevsW(ref Guid ClassGuid, IntPtr Enumerator, IntPtr hwndParent, uint Flags);
    
    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool SetupDiEnumDeviceInterfaces(IntPtr DeviceInfoSet, IntPtr DeviceInfoData, ref Guid InterfaceClassGuid, int MemberIndex, ref SP_DEVICE_INTERFACE_DATA DeviceInterfaceData);
    
    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool SetupDiGetDeviceInterfaceDetailW(IntPtr DeviceInfoSet, ref SP_DEVICE_INTERFACE_DATA DeviceInterfaceData, IntPtr DeviceInterfaceDetailData, uint DeviceInterfaceDetailDataSize, ref uint RequiredSize, IntPtr DeviceInfoData);
    
    [DllImport("setupapi.dll", SetLastError = true)]
    public static extern bool SetupDiDestroyDeviceInfoList(IntPtr DeviceInfoSet);
    
    [StructLayout(LayoutKind.Sequential)]
    public struct SP_DEVICE_INTERFACE_DATA {
        public int cbSize;
        public Guid InterfaceClassGuid;
        public uint Flags;
        public IntPtr Reserved;
    }
    
    public const uint DIGCF_DEVICEINTERFACE = 0x00000010;
    public const uint DIGCF_PRESENT = 0x00000002;
}
"@ -ErrorAction SilentlyContinue

$hDevInfo = [IoTSetup]::SetupDiGetClassDevsW([ref]$guid, [IntPtr]::Zero, [IntPtr]::Zero, [IoTSetup]::DIGCF_DEVICEINTERFACE -bor [IoTSetup]::DIGCF_PRESENT)
Write-Host "SetupDiGetClassDevs handle: $($hDevInfo.ToInt64())"

if ($hDevInfo -ne [IntPtr]::new(-1)) {
    $did = New-Object IoTSetup+SP_DEVICE_INTERFACE_DATA
    $did.cbSize = [System.Runtime.InteropServices.Marshal]::SizeOf($did)
    Write-Host "SP_DEVICE_INTERFACE_DATA size: $($did.cbSize)"
    
    $idx = 0
    while ([IoTSetup]::SetupDiEnumDeviceInterfaces($hDevInfo, [IntPtr]::Zero, [ref]$guid, $idx, [ref]$did)) {
        Write-Host "Found interface $idx"
        
        $requiredSize = [uint32]0
        [IoTSetup]::SetupDiGetDeviceInterfaceDetailW($hDevInfo, [ref]$did, [IntPtr]::Zero, 0, [ref]$requiredSize, [IntPtr]::Zero) | Out-Null
        $err = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
        Write-Host "  Required size: $requiredSize (err=$err, expected since we passed 0)"
        
        if ($requiredSize -gt 0) {
            $detailPtr = [System.Runtime.InteropServices.Marshal]::AllocHGlobal([int]$requiredSize)
            # On x64, SP_DEVICE_INTERFACE_DETAIL_DATA_W.cbSize = 8 (4 for cbSize + padding)
            [System.Runtime.InteropServices.Marshal]::WriteInt32($detailPtr, 8)
            
            if ([IoTSetup]::SetupDiGetDeviceInterfaceDetailW($hDevInfo, [ref]$did, $detailPtr, $requiredSize, [ref]$requiredSize, [IntPtr]::Zero)) {
                # Read the device path string (starts at offset 4 in the struct, after cbSize)
                $devicePath = [System.Runtime.InteropServices.Marshal]::PtrToStringUni($detailPtr, 4)
                Write-Host "  Device path: $devicePath"
            } else {
                $err2 = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
                Write-Host "  SetupDiGetDeviceInterfaceDetail failed: $err2"
            }
            [System.Runtime.InteropServices.Marshal]::FreeHGlobal($detailPtr)
        }
        $idx++
    }
    [IoTSetup]::SetupDiDestroyDeviceInfoList($hDevInfo) | Out-Null
}

# Now try to open the device and do IOCTL
if ($devicePath) {
    Write-Host ""
    Write-Host "=== Opening device: $devicePath ==="
    
    Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public class IoTFile {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr CreateFileW(string lpFileName, uint dwDesiredAccess, uint dwShareMode, IntPtr lpSecurityAttributes, uint dwCreationDisposition, uint dwFlagsAndAttributes, IntPtr hTemplateFile);
    
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool DeviceIoControl(IntPtr hDevice, uint dwIoControlCode, byte[] lpInBuffer, uint nInBufferSize, byte[] lpOutBuffer, uint nOutBufferSize, ref uint lpBytesReturned, IntPtr lpOverlapped);
    
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool CloseHandle(IntPtr hObject);
}
"@ -ErrorAction SilentlyContinue

    $GENERIC_READ = 0x80000000
    $GENERIC_WRITE = 0x40000000
    $OPEN_EXISTING = 3
    
    # Try read-write first
    $handle = [IoTFile]::CreateFileW($devicePath, $GENERIC_READ -bor $GENERIC_WRITE, 0, [IntPtr]::Zero, $OPEN_EXISTING, 0, [IntPtr]::Zero)
    $lastErr = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
    
    if ($handle -eq [IntPtr]::new(-1)) {
        Write-Host "CreateFileW RW failed: Error $lastErr"
        
        # Try read-only
        $handle = [IoTFile]::CreateFileW($devicePath, $GENERIC_READ, 0, [IntPtr]::Zero, $OPEN_EXISTING, 0, [IntPtr]::Zero)
        $lastErr = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
        
        if ($handle -eq [IntPtr]::new(-1)) {
            Write-Host "CreateFileW RO failed: Error $lastErr"
        } else {
            Write-Host "CreateFileW RO succeeded! Handle=$($handle.ToInt64())"
        }
    } else {
        Write-Host "CreateFileW RW succeeded! Handle=$($handle.ToInt64())"
    }
    
    if ($handle -ne [IntPtr]::new(-1)) {
        # Try ECRAM_READ IOCTL (0x22E000)
        # Input: 0x110 bytes, first 4 bytes = base address
        $inBuf = New-Object byte[] 0x110
        $outBuf = New-Object byte[] 0x110
        
        # ERAM base = 0xFE0B0300
        [BitConverter]::GetBytes([uint32]0xFE0B0300).CopyTo($inBuf, 0)
        
        $bytesReturned = [uint32]0
        Write-Host "Trying ECRAM_READ (0x22E000) with base=0xFE0B0300..."
        $ok = [IoTFile]::DeviceIoControl($handle, 0x22E000, $inBuf, [uint32]$inBuf.Length, $outBuf, [uint32]$outBuf.Length, [ref]$bytesReturned, [IntPtr]::Zero)
        $err = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
        
        if ($ok) {
            Write-Host "ECRAM_READ SUCCESS! Bytes returned: $bytesReturned"
            # Show first 64 bytes
            $hex = ($outBuf[0..63] | ForEach-Object { '{0:X2}' -f $_ }) -join ' '
            Write-Host "First 64 bytes: $hex"
        } else {
            Write-Host "ECRAM_READ FAILED: Error $err"
        }
        
        # Try different IOCTL codes
        foreach ($ioctl in @(0x22E004, 0x22E008, 0x22E00C, 0x222000, 0x222004, 0x222008, 0x224000)) {
            $bytesReturned2 = [uint32]0
            $ok2 = [IoTFile]::DeviceIoControl($handle, [uint32]$ioctl, $inBuf, [uint32]$inBuf.Length, $outBuf, [uint32]$outBuf.Length, [ref]$bytesReturned2, [IntPtr]::Zero)
            $err2 = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
            if ($ok2) {
                Write-Host "IOCTL 0x$($ioctl.ToString('X')) SUCCESS! Bytes: $bytesReturned2"
            } else {
                Write-Host "IOCTL 0x$($ioctl.ToString('X')) failed: Error $err2"
            }
        }
        
        [IoTFile]::CloseHandle($handle) | Out-Null
    }
}

Write-Host ""
Write-Host "=== WMI MICommonInterface Test (Admin) ==="

try {
    $scope = New-Object System.Management.ManagementScope("\\.\ROOT\WMI")
    $scope.Connect()
    
    $query = "SELECT * FROM MICommonInterface"
    $searcher = New-Object System.Management.ManagementObjectSearcher($scope, [System.Management.ObjectQuery]$query)
    $instances = $searcher.Get()
    
    foreach ($inst in $instances) {
        Write-Host "Instance: $($inst.InstanceName)"
        
        # Try MiInterface with 32-byte buffer
        $inData = New-Object byte[] 32
        $inData[0] = 0x0A  # GetFwVersion
        
        $inParams = $inst.GetMethodParameters("MiInterface")
        $inParams["InData"] = $inData
        
        Write-Host "  Trying MiInterface with 32-byte buffer (cmd=0x0A)..."
        
        try {
            $outParams = $inst.InvokeMethod("MiInterface", $inParams, $null)
            $returnCode = $outParams["ReturnCode"]
            $outData = $outParams["OutData"]
            Write-Host "    ReturnCode: $returnCode"
            if ($outData) {
                $hex = ($outData | ForEach-Object { '{0:X2}' -f $_ }) -join ' '
                Write-Host "    OutData ($($outData.Length) bytes): $hex"
            }
        } catch {
            Write-Host "    Error: $($_.Exception.Message)"
        }
    }
} catch {
    Write-Host "WMI Error: $_"
}
