# Comprehensive IoT hardware access test - NOW WITH ADMIN PRIVILEGES
# Tests: EC RAM IOCTL, WMI MiInterface, ACPI WMAA, all WMI classes

Write-Host "=== ADMIN PRIVILEGE CHECK ==="
$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object System.Security.Principal.WindowsPrincipal($identity)
$isAdmin = $principal.IsInRole([System.Security.Principal.WindowsBuiltInRole]::Administrator)
Write-Host "IsAdmin: $isAdmin"
Write-Host "User: $($identity.Name)"
Write-Host ""

# ============================================================
# 1. Test EC RAM Direct IOCTL Access (IoTDriver.sys)
# ============================================================
Write-Host "=== 1. EC RAM Direct IOCTL (IoTDriver.sys) ==="

# Find the IoT device path
$deviceGuid = [Guid]::new("AB7924A1-3162-4010-B33B-837E87E25FBC")
$deviceGuidBytes = $deviceGuid.ToByteArray()
$deviceGuidStr = $deviceGuid.ToString("B")

# Use SetupAPI to find the device
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public class SetupAPI {
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
    
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct SP_DEVICE_INTERFACE_DETAIL_DATA_W {
        public int cbSize;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)]
        public string DevicePath;
    }
    
    public const uint DIGCF_DEVICEINTERFACE = 0x00000010;
    public const uint DIGCF_PRESENT = 0x00000002;
}
"@ -ErrorAction SilentlyContinue

$guid = [Guid]"AB7924A1-3162-4010-B33B-837E87E25FBC"
$hDevInfo = [SetupAPI]::SetupDiGetClassDevsW([ref]$guid, [IntPtr]::Zero, [IntPtr]::Zero, [SetupAPI]::DIGCF_DEVICEINTERFACE -bor [SetupAPI]::DIGCF_PRESENT)

if ($hDevInfo -eq [IntPtr]::new(-1)) {
    Write-Host "SetupDiGetClassDevs failed: $([System.Runtime.InteropServices.Marshal]::GetLastWin32Error())"
} else {
    $did = New-Object SetupAPI+SP_DEVICE_INTERFACE_DATA
    $did.cbSize = [System.Runtime.InteropServices.Marshal]::SizeOf($did)
    
    if ([SetupAPI]::SetupDiEnumDeviceInterfaces($hDevInfo, [IntPtr]::Zero, [ref]$guid, 0, [ref]$did)) {
        $requiredSize = 0
        [SetupAPI]::SetupDiGetDeviceInterfaceDetailW($hDevInfo, [ref]$did, [IntPtr]::Zero, 0, [ref]$requiredSize, [IntPtr]::Zero) | Out-Null
        
        $detailPtr = [System.Runtime.InteropServices.Marshal]::AllocHGlobal($requiredSize)
        $detail = New-Object SetupAPI+SP_DEVICE_INTERFACE_DETAIL_DATA_W
        $detail.cbSize = 8  # For x64, cbSize is 8 (4 bytes for cbSize + 4 bytes padding for alignment)
        
        [System.Runtime.InteropServices.Marshal]::StructureToPtr($detail, $detailPtr, $false)
        
        if ([SetupAPI]::SetupDiGetDeviceInterfaceDetailW($hDevInfo, [ref]$did, $detailPtr, $requiredSize, [ref]$requiredSize, [IntPtr]::Zero)) {
            $devicePath = [System.Runtime.InteropServices.Marshal]::PtrToStringUni($detailPtr, 8)
            Write-Host "Device path: $devicePath"
            
            # Try to open the device
            $handle = [SetupAPI]::SetupDiDestroyDeviceInfoList($hDevInfo)  # cleanup first
            
            # Use CreateFileW to open the device
            Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public class FileNative {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr CreateFileW(string lpFileName, uint dwDesiredAccess, uint dwShareMode, IntPtr lpSecurityAttributes, uint dwCreationDisposition, uint dwFlagsAndAttributes, IntPtr hTemplateFile);
    
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool DeviceIoControl(IntPtr hDevice, uint dwIoControlCode, IntPtr lpInBuffer, uint nInBufferSize, IntPtr lpOutBuffer, uint nOutBufferSize, ref uint lpBytesReturned, IntPtr lpOverlapped);
    
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool DeviceIoControlBuf(IntPtr hDevice, uint dwIoControlCode, byte[] lpInBuffer, uint nInBufferSize, byte[] lpOutBuffer, uint nOutBufferSize, ref uint lpBytesReturned, IntPtr lpOverlapped);
    
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool CloseHandle(IntPtr hObject);
    
    public const uint GENERIC_READ = 0x80000000;
    public const uint GENERIC_WRITE = 0x40000000;
    public const uint OPEN_EXISTING = 3;
    public const uint FILE_ATTRIBUTE_NORMAL = 0x80;
}
"@ -ErrorAction SilentlyContinue

            # Try to open the IoT device
            $devHandle = [FileNative]::CreateFileW($devicePath, [FileNative]::GENERIC_READ -bor [FileNative]::GENERIC_WRITE, 0, [IntPtr]::Zero, [FileNative]::OPEN_EXISTING, [FileNative]::FILE_ATTRIBUTE_NORMAL, [IntPtr]::Zero)
            
            if ($devHandle -eq [IntPtr]::new(-1)) {
                $err = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
                Write-Host "CreateFile FAILED: Error $err"
                
                # Try read-only
                $devHandle = [FileNative]::CreateFileW($devicePath, [FileNative]::GENERIC_READ, 0, [IntPtr]::Zero, [FileNative]::OPEN_EXISTING, [FileNative]::FILE_ATTRIBUTE_NORMAL, [IntPtr]::Zero)
                if ($devHandle -eq [IntPtr]::new(-1)) {
                    $err2 = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
                    Write-Host "CreateFile READ-ONLY also FAILED: Error $err2"
                } else {
                    Write-Host "CreateFile READ-ONLY succeeded!"
                    # Try ECRAM_READ IOCTL (0x22E000)
                    $outBuf = New-Object byte[] 0x110
                    $inBuf = New-Object byte[] 0x110
                    # Set ERAM base address at offset 0
                    $baseAddr = 0xFE0B0300
                    [BitConverter]::GetBytes([uint32]$baseAddr).CopyTo($inBuf, 0)
                    
                    $bytesReturned = [uint32]0
                    $ok = [FileNative]::DeviceIoControlBuf($devHandle, 0x22E000, $inBuf, [uint32]$inBuf.Length, $outBuf, [uint32]$outBuf.Length, [ref]$bytesReturned, [IntPtr]::Zero)
                    
                    if ($ok) {
                        Write-Host "ECRAM_READ SUCCESS! Bytes returned: $bytesReturned"
                        # Show first 64 bytes
                        $hex = ($outBuf[0..63] | ForEach-Object { '{0:X2}' -f $_ }) -join ' '
                        Write-Host "First 64 bytes: $hex"
                    } else {
                        $err3 = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
                        Write-Host "DeviceIoControl FAILED: Error $err3"
                    }
                    [FileNative]::CloseHandle($devHandle) | Out-Null
                }
            } else {
                Write-Host "CreateFile SUCCESS! Handle=$($devHandle.ToInt64())"
                
                # Try ECRAM_READ IOCTL (0x22E000)
                $outBuf = New-Object byte[] 0x110
                $inBuf = New-Object byte[] 0x110
                # Set ERAM base address at offset 0
                $baseAddr = 0xFE0B0300
                [BitConverter]::GetBytes([uint32]$baseAddr).CopyTo($inBuf, 0)
                
                $bytesReturned = [uint32]0
                $ok = [FileNative]::DeviceIoControlBuf($devHandle, 0x22E000, $inBuf, [uint32]$inBuf.Length, $outBuf, [uint32]$outBuf.Length, [ref]$bytesReturned, [IntPtr]::Zero)
                
                if ($ok) {
                    Write-Host "ECRAM_READ SUCCESS! Bytes returned: $bytesReturned"
                    $hex = ($outBuf[0..63] | ForEach-Object { '{0:X2}' -f $_ }) -join ' '
                    Write-Host "First 64 bytes: $hex"
                } else {
                    $err3 = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
                    Write-Host "DeviceIoControl ECRAM_READ FAILED: Error $err3"
                }
                
                [FileNative]::CloseHandle($devHandle) | Out-Null
            }
        } else {
            Write-Host "SetupDiGetDeviceInterfaceDetail failed: $([System.Runtime.InteropServices.Marshal]::GetLastWin32Error())"
        }
        [System.Runtime.InteropServices.Marshal]::FreeHGlobal($detailPtr)
    } else {
        Write-Host "SetupDiEnumDeviceInterfaces failed: $([System.Runtime.InteropServices.Marshal]::GetLastWin32Error())"
    }
    [SetupAPI]::SetupDiDestroyDeviceInfoList($hDevInfo) | Out-Null
}

Write-Host ""

# ============================================================
# 2. Test WMI MICommonInterface.MiInterface (NOW WITH ADMIN)
# ============================================================
Write-Host "=== 2. WMI MICommonInterface.MiInterface (Admin) ==="

try {
    $scope = New-Object System.Management.ManagementScope("\\.\ROOT\WMI")
    $scope.Connect()
    
    # Get MICommonInterface class
    $query = "SELECT * FROM MICommonInterface WHERE InstanceName='ACPI\\PNP0C14\\MIFS_0'"
    $searcher = New-Object System.Management.ManagementObjectSearcher($scope, [System.Management.ObjectQuery]$query)
    $instances = $searcher.Get()
    
    foreach ($inst in $instances) {
        Write-Host "Instance: $($inst.InstanceName)"
        
        # Get the class to access method parameters
        $class = $inst.ClassPath
        
        # Try MiInterface with different buffer sizes
        foreach ($size in @(4, 8, 16, 32)) {
            $inData = New-Object byte[] $size
            # Try with command byte at position 0
            if ($size -ge 4) {
                $inData[0] = 0x0A  # GetFwVersion command
            }
            
            $inParams = $inst.GetMethodParameters("MiInterface")
            $inParams["InData"] = $inData
            
            Write-Host "  Trying $size-byte buffer (cmd=0x0A)..."
            
            try {
                $outParams = $inst.InvokeMethod("MiInterface", $inParams, $null)
                $returnCode = $outParams["ReturnCode"]
                $outData = $outParams["OutData"]
                Write-Host "    ReturnCode: $returnCode"
                if ($outData -ne $null) {
                    $hex = ($outData | ForEach-Object { '{0:X2}' -f $_ }) -join ' '
                    Write-Host "    OutData ($($outData.Length) bytes): $hex"
                }
            } catch {
                Write-Host "    Error: $_"
            }
        }
        
        # Try with different command bytes
        foreach ($cmd in @(0x01, 0x02, 0x03, 0x05, 0x06, 0x0A, 0x10, 0x65, 0x66, 0x67, 0x68, 0x6E)) {
            $inData = New-Object byte[] 32
            $inData[0] = $cmd
            
            $inParams = $inst.GetMethodParameters("MiInterface")
            $inParams["InData"] = $inData
            
            Write-Host "  Trying cmd=0x$($cmd.ToString('X2'))..."
            
            try {
                $outParams = $inst.InvokeMethod("MiInterface", $inParams, $null)
                $returnCode = $outParams["ReturnCode"]
                $outData = $outParams["OutData"]
                Write-Host "    ReturnCode: $returnCode"
                if ($outData -ne $null -and $outData.Length -gt 0) {
                    $hex = ($outData | ForEach-Object { '{0:X2}' -f $_ }) -join ' '
                    Write-Host "    OutData ($($outData.Length) bytes): $hex"
                }
            } catch {
                Write-Host "    Error: $($_.Exception.Message)"
            }
        }
    }
} catch {
    Write-Host "WMI Error: $_"
}

Write-Host ""

# ============================================================
# 3. Test ALL WMI Classes in ROOT\WMI
# ============================================================
Write-Host "=== 3. All WMI Classes in ROOT\WMI ==="

try {
    $scope = New-Object System.Management.ManagementScope("\\.\ROOT\WMI")
    $scope.Connect()
    
    $enumQuery = New-Object System.Management.ObjectQuery("SELECT * FROM meta_class")
    $searcher = New-Object System.Management.ManagementObjectSearcher($scope, $enumQuery)
    
    foreach ($class in $searcher.Get()) {
        $className = $class["__CLASS"].ToString()
        if ($className -match "IoT|MI|Mi|HQ|Esif|ACPI|WMI|Fan|Battery|Charge|Power|Therm|Sensor|Backlight|Keyboard|Brightness|Display") {
            Write-Host "  Class: $className"
            
            # Try to get instances
            try {
                $instQuery = "SELECT * FROM $className"
                $instSearcher = New-Object System.Management.ManagementObjectSearcher($scope, [System.Management.ObjectQuery]$instQuery)
                $insts = $instSearcher.Get()
                foreach ($i in $insts) {
                    $instName = $i["InstanceName"]
                    if ($instName) { Write-Host "    Instance: $instName" }
                    
                    # List methods
                    $methods = $i.ClassDefinition.Methods
                    foreach ($m in $methods) {
                        Write-Host "    Method: $($m.Name)"
                    }
                    
                    # List properties
                    foreach ($prop in $i.Properties) {
                        if ($prop.Name -notmatch "^__") {
                            $val = $prop.Value
                            if ($val -is [array]) {
                                $val = ($val | ForEach-Object { $_.ToString() }) -join ','
                            }
                            Write-Host "    Property: $($prop.Name) = $val"
                        }
                    }
                }
            } catch {
                Write-Host "    (no instances or error)"
            }
        }
    }
} catch {
    Write-Host "WMI Enum Error: $_"
}

Write-Host ""

# ============================================================
# 4. Test IoTService IPC Pipe (with admin)
# ============================================================
Write-Host "=== 4. IoTService IPC Pipe (Admin) ==="

try {
    $pipe = New-Object System.IO.Pipes.NamedPipeClientStream(".", "LOCAL\IoTService_IPC_Broker", [System.IO.Pipes.PipeDirection]::InOut, [System.IO.Pipes.PipeOptions]::Asynchronous, [System.Security.Principal.TokenImpersonationLevel]::Impersonation)
    $pipe.Connect(5000)
    $pipe.ReadMode = [System.IO.Pipes.PipeTransmissionMode]::Message
    Write-Host "Connected to pipe!"
    
    # Try all message types
    foreach ($msgType in @(0x1001, 0x1002, 0x1003, 0x1004, 0x1005, 0x1006, 0x2001, 0x2002, 0x3001, 0x4001, 0x4004, 0x4005)) {
        $msg = New-Object byte[] 16
        # MCPI magic
        $msg[0] = 0x4D; $msg[1] = 0x43; $msg[2] = 0x50; $msg[3] = 0x49
        # src_id=1, dst_id=2
        $msg[4] = 0x01; $msg[5] = 0x00; $msg[6] = 0x02; $msg[7] = 0x00
        # type_lo
        $typeLo = $msgType -band 0xFFFF
        $msg[8] = $typeLo -band 0xFF; $msg[9] = ($typeLo -shr 8) -band 0xFF
        # routing=0, field=0, payload_len=16
        $msg[14] = 0x10; $msg[15] = 0x00
        
        $pipe.Write($msg, 0, $msg.Length)
        
        # Try to read response with 2s timeout
        $buf = New-Object byte[] 8192
        $task = $pipe.ReadAsync($buf, 0, 8192)
        $ok = $task.Wait(2000)
        
        if ($ok -and $task.Result -gt 0) {
            $bytesRead = $task.Result
            $hex = ($buf[0..([Math]::Min($bytesRead-1, 63))] | ForEach-Object { '{0:X2}' -f $_ }) -join ' '
            Write-Host "  Type 0x$($msgType.ToString('X4')): RESPONSE $bytesRead bytes: $hex"
        } else {
            Write-Host "  Type 0x$($msgType.ToString('X4')): no response (2s timeout)"
        }
    }
    
    $pipe.Close()
} catch {
    Write-Host "Pipe error: $_"
}

Write-Host ""
Write-Host "=== DONE ==="
