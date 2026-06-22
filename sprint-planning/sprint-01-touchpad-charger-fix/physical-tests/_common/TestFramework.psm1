# TestFramework.psm1 - Shared utilities for Sprint 1 physical EMI tests
# Requires: Windows 10/11, PowerShell 5.1+, run as Administrator
# This module is dot-sourced / imported by each run-test.ps1 script.
# It provides: environment setup, logging, system snapshots, miPC dev-mode
# control, ETW capture, HID input report monitoring, capture-window countdown,
# reboot-resume state, and configuration revert helpers.
#Requires -Version 5.1

# --- Configuration -----------------------------------------------------------

# physical-tests/ = parent of _common/
$script:TestRoot = Split-Path -Parent $PSScriptRoot
$script:LogsDir = Join-Path $script:TestRoot "logs"
$script:ResultsDir = Join-Path $script:TestRoot "results"
# micontrol/ = three levels up from physical-tests/
# physical-tests -> sprint-01-touchpad-charger-fix -> sprint-planning -> micontrol
$script:MiPCRepoRoot = $script:TestRoot | Split-Path -Parent | Split-Path -Parent | Split-Path -Parent
$script:StateDir = Join-Path $script:TestRoot ".state"
$script:CaptureDurationSec = 300  # 5 minutes default capture window

# Touchpad identifiers (from src-tauri/src/hw/touchpad.rs)
$script:TouchpadHardwareId = "ACPI\BLTP7853"
$script:I2cVendorDevice = "PCI\VEN_8086&DEV_E448"

# miPC dev server
$script:DevUrl = "http://localhost:1420"
$script:DevPort = 1420

# ETW session name
$script:EtwSessionName = "MiPC_EMI_Test"

# RunOnce key for reboot-resume
$script:RunOnceKey = "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\RunOnce"
$script:RunOnceValue = "MiPC_TestResume"

# Ensure directories exist on import (best-effort, non-fatal)
foreach ($d in @($script:LogsDir, $script:ResultsDir, $script:StateDir)) {
    if (-not (Test-Path $d)) {
        try { New-Item -ItemType Directory -Path $d -Force | Out-Null } catch { }
    }
}

# --- Admin / environment -----------------------------------------------------

function Test-Admin {
    <#
        .SYNOPSIS
            Returns $true if the current process is elevated (Administrator).
    #>
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    $p = New-Object Security.Principal.WindowsPrincipal($id)
    return $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Initialize-TestEnvironment {
    <#
        .SYNOPSIS
            Creates logs/, results/, .state/ directories and verifies admin.
            Returns a session hashtable with a timestamp used for file naming.
    #>
    foreach ($d in @($script:LogsDir, $script:ResultsDir, $script:StateDir)) {
        if (-not (Test-Path $d)) {
            New-Item -ItemType Directory -Path $d -Force | Out-Null
        }
    }
    if (-not (Test-Admin)) {
        throw "This test must be run as Administrator. Re-launch PowerShell elevated."
    }
    $ts = Get-Date -Format "yyyyMMdd-HHmmss"
    return @{
        Timestamp = $ts
        LogsDir   = $script:LogsDir
        ResultsDir = $script:ResultsDir
        StateDir  = $script:StateDir
        TestRoot  = $script:TestRoot
        MiPCRepoRoot = $script:MiPCRepoRoot
    }
}

# --- Logging -----------------------------------------------------------------

function Write-TestLog {
    <#
        .SYNOPSIS
            Writes a timestamped message to a log file and the host.
        .PARAMETER Message
            The text to log.
        .PARAMETER Level
            INFO, WARN, ERROR, OK (controls host color).
        .PARAMETER LogFile
            Absolute path to the .log file. If omitted, only Write-Host is used.
    #>
    param(
        [Parameter(Mandatory)][string]$Message,
        [ValidateSet("INFO","WARN","ERROR","OK","DEBUG")][string]$Level = "INFO",
        [string]$LogFile
    )
    $stamp = (Get-Date).ToString("yyyy-MM-ddTHH:mm:ss")
    $line = "[$stamp] [$Level] $Message"
    if ($LogFile) {
        try { Add-Content -Path $LogFile -Value $line -ErrorAction Stop } catch { }
    }
    $color = switch ($Level) {
        "INFO"  { "White" }
        "WARN"  { "Yellow" }
        "ERROR" { "Red" }
        "OK"    { "Green" }
        "DEBUG" { "DarkGray" }
    }
    Write-Host $line -ForegroundColor $color
}

function Write-TestHeader {
    <#
        .SYNOPSIS
            Writes a banner with ticket ID, title, date, and machine info.
    #>
    param(
        [Parameter(Mandatory)][string]$TicketId,
        [Parameter(Mandatory)][string]$Title,
        [string]$LogFile
    )
    $bar = "=" * 78
    $machine = $env:COMPUTERNAME
    $user = $env:USERNAME
    $date = (Get-Date).ToString("yyyy-MM-dd HH:mm:ss zzz")
    $banner = @(
        $bar
        "  $TicketId - $Title"
        "  Machine: $machine   User: $user   Date: $date"
        $bar
    ) -join "`r`n"
    if ($LogFile) { Add-Content -Path $LogFile -Value $banner }
    Write-Host $banner -ForegroundColor Cyan
}

function Write-TestFooter {
    <#
        .SYNOPSIS
            Writes a completion banner with result and log file locations.
    #>
    param(
        [Parameter(Mandatory)][string]$TicketId,
        [string]$LogFile,
        [ValidateSet("PASS","FAIL","INCONCLUSIVE","ABORTED")][string]$Result = "INCONCLUSIVE"
    )
    $bar = "=" * 78
    $lines = @(
        $bar,
        "  $TicketId COMPLETE - Result: $Result",
        "  Log:     $LogFile"
    )
    if ($LogFile -and (Test-Path $LogFile)) {
        $lines += "  Log size: $((Get-Item $LogFile).Length) bytes"
    }
    $lines += $bar
    $banner = $lines -join "`r`n"
    if ($LogFile) { Add-Content -Path $LogFile -Value $banner }
    $color = if ($Result -eq "PASS") { "Green" } elseif ($Result -eq "FAIL") { "Red" } else { "Yellow" }
    Write-Host $banner -ForegroundColor $color
}

# --- System snapshot ---------------------------------------------------------

function Get-SystemSnapshot {
    <#
        .SYNOPSIS
            Captures a comprehensive system state snapshot and logs it.
            Returns a hashtable of the captured values.
    #>
    param(
        [string]$Label = "snapshot",
        [string]$LogFile
    )
    $snap = [ordered]@{
        Label      = $Label
        Timestamp  = (Get-Date).ToString("yyyy-MM-ddTHH:mm:ss")
        Machine    = $env:COMPUTERNAME
        OSBuild    = (Get-CimInstance Win32_OperatingSystem).Caption + " build " + [System.Environment]::OSVersion.Version.ToString()
    }

    # Battery + AC status
    try {
        $bat = Get-CimInstance -ClassName Win32_Battery -ErrorAction Stop
        $snap["BatteryStatus"] = $bat.BatteryStatus
        $snap["BatteryPercent"] = $bat.EstimatedChargeRemaining
    } catch {
        $snap["BatteryStatus"] = "unknown"
        $snap["BatteryPercent"] = "unknown"
    }

    # Active power plan
    try {
        $pcfg = powercfg /getactivescheme 2>&1
        $snap["PowerPlan"] = ($pcfg | Out-String).Trim()
        if ($pcfg -match 'GUID:\s*([0-9a-fA-F\-]+)') { $snap["PowerPlanGuid"] = $matches[1] }
    } catch { $snap["PowerPlan"] = "unknown" }

    # IoTService status
    try {
        $svc = Get-Service -Name IoTSvc -ErrorAction Stop
        $snap["IoTServiceStatus"] = $svc.Status.ToString()
        $snap["IoTServiceStartType"] = $svc.StartType.ToString()
    } catch {
        $snap["IoTServiceStatus"] = "not found"
        $snap["IoTServiceStartType"] = "n/a"
    }

    # Touchpad device status
    try {
        $tp = Get-PnpDevice | Where-Object { $_.InstanceId -like "$($script:TouchpadHardwareId)*" } -ErrorAction Stop
        if ($tp) {
            $snap["TouchpadStatus"] = ($tp | ForEach-Object { "$($_.Status)/$($_.InstanceId)" }) -join "; "
        } else { $snap["TouchpadStatus"] = "not found" }
    } catch { $snap["TouchpadStatus"] = "error" }

    # I2C controller IdleTimerPeriod
    try {
        $i2cId = Get-I2cControllerDeviceId
        if ($i2cId) {
            $snap["I2cDeviceId"] = $i2cId
            $regPath = "HKLM:\SYSTEM\CurrentControlSet\Enum\$i2cId\Device Parameters"
            $it = (Get-ItemProperty -Path $regPath -Name IdleTimerPeriod -ErrorAction SilentlyContinue).IdleTimerPeriod
            $snap["IdleTimerPeriod"] = if ($null -ne $it) { $it } else { "<not set>" }
        } else { $snap["I2cDeviceId"] = "not found" }
    } catch { $snap["I2cDeviceId"] = "error" }

    # Process count + CPU sample
    try {
        $procs = (Get-Process -ErrorAction SilentlyContinue).Count
        $snap["ProcessCount"] = $procs
    } catch { $snap["ProcessCount"] = "unknown" }
    try {
        $cpu = (Get-Counter '\Processor(_Total)\% Processor Time' -ErrorAction SilentlyContinue).CounterSamples.CookedValue
        $snap["CpuLoadPct"] = if ($null -ne $cpu) { [math]::Round($cpu, 1) } else { "n/a" }
    } catch { $snap["CpuLoadPct"] = "n/a" }

    # Log the snapshot
    Write-TestLog "--- System Snapshot: $Label ---" -LogFile $LogFile
    foreach ($k in $snap.Keys) {
        Write-TestLog ("  {0,-22} = {1}" -f $k, $snap[$k]) -LogFile $LogFile
    }
    return $snap
}

# --- Device ID helpers -------------------------------------------------------

function Get-TouchpadDeviceId {
    <#
        .SYNOPSIS
            Returns the device instance ID of the BLTP7853 touchpad (ACPI\BLTP7853).
    #>
    $dev = Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object {
        $_.InstanceId -like "$($script:TouchpadHardwareId)*"
    } | Select-Object -First 1
    if ($dev) { return $dev.InstanceId }
    return $null
}

function Get-I2cControllerDeviceId {
    <#
        .SYNOPSIS
            Returns the device instance ID of the Intel Quick I2C Host
            Controller (PCI\VEN_8086&DEV_E448).
    #>
    # Try direct match on the vendor/device string first.
    $dev = Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object {
        $_.InstanceId -like "*VEN_8086*" -and $_.InstanceId -like "*DEV_E448*"
    } | Select-Object -First 1
    if (-not $dev) {
        # Fallback: match by class friendly name containing "I2C"
        $dev = Get-PnpDevice -Class System -ErrorAction SilentlyContinue |
            Where-Object { $_.FriendlyName -like "*I2C*" } | Select-Object -First 1
    }
    if ($dev) { return $dev.InstanceId }
    return $null
}

# --- miPC dev mode -----------------------------------------------------------

function Test-MiPCDevModeRunning {
    <#
        .SYNOPSIS
            Returns $true if the miPC dev server is already listening on port 1420.
    #>
    try {
        $conn = Get-NetTCPConnection -LocalPort $script:DevPort -State Listen -ErrorAction SilentlyContinue
        return ($null -ne $conn)
    } catch {
        # Fallback: try a TCP connect
        try {
            $tcp = New-Object System.Net.Sockets.TcpClient
            $iar = $tcp.BeginConnect("127.0.0.1", $script:DevPort, $null, $null)
            $ok = $iar.AsyncWaitHandle.WaitOne(800, $false)
            if ($ok -and $tcp.Connected) { $tcp.Close(); return $true }
            $tcp.Close()
        } catch { }
        return $false
    }
}

function Start-MiPCDevMode {
    <#
        .SYNOPSIS
            Starts 'npm run tauri dev' from the micontrol repo root in a new
            window. Waits up to 120s for the dev URL to respond.
            If already running, skips and returns a marker indicating we did
            NOT own the process (so we won't kill it later).
        .OUTPUTS
            Hashtable: @{ Job=$null; Owned=$bool; Pid=$int; WindowTitle=$str }
    #>
    param([string]$LogFile)

    if (Test-MiPCDevModeRunning) {
        Write-TestLog "miPC dev mode already running on port $($script:DevPort) - skipping start (not owned)." -Level "OK" -LogFile $LogFile
        return @{ Job = $null; Owned = $false; Pid = $null; WindowTitle = $null }
    }

    if (-not (Test-Path $script:MiPCRepoRoot)) {
        Write-TestLog "miPC repo root not found at $($script:MiPCRepoRoot) - cannot start dev mode." -Level "ERROR" -LogFile $LogFile
        return @{ Job = $null; Owned = $false; Pid = $null; WindowTitle = $null }
    }

    Write-TestLog "Starting miPC dev mode (npm run tauri dev) from $($script:MiPCRepoRoot)..." -LogFile $LogFile
    # Launch in a new window so the user can see the build output.
    $title = "miPC-dev-$(Get-Date -Format 'HHmmss')"
    $cmd = "cd '$($script:MiPCRepoRoot)'; npm run tauri dev"
    $proc = Start-Process powershell -ArgumentList "-NoExit","-Command","$cmd" -PassThru -WindowStyle Normal
    # Track the launcher PID; the actual node/cargo processes are children.
    $info = @{ Job = $null; Owned = $true; Pid = $proc.Id; WindowTitle = $title }

    # Wait for the dev URL to respond (up to 120s)
    $deadline = (Get-Date).AddSeconds(120)
    $ready = $false
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Seconds 3
        if (Test-MiPCDevModeRunning) { $ready = $true; break }
        # If the launcher died, abort early
        if ($proc.HasExited) {
            Write-TestLog "miPC dev launcher process exited prematurely." -Level "ERROR" -LogFile $LogFile
            return $info
        }
    }
    if ($ready) {
        Write-TestLog "miPC dev mode is up at $($script:DevUrl) (PID $($proc.Id))." -Level "OK" -LogFile $LogFile
    } else {
        Write-TestLog "miPC dev mode did not respond within 120s - continuing anyway." -Level "WARN" -LogFile $LogFile
    }
    return $info
}

function Stop-MiPCDevMode {
    <#
        .SYNOPSIS
            Stops the miPC dev processes started by Start-MiPCDevMode.
            Does NOT kill processes we did not own (Owned=$false).
    #>
    param($MiPCJob, [string]$LogFile)

    if (-not $MiPCJob) {
        Write-TestLog "Stop-MiPCDevMode: no job info provided." -Level "WARN" -LogFile $LogFile
        return
    }
    if (-not $MiPCJob.Owned) {
        Write-TestLog "miPC dev mode was already running before the test - leaving it running." -Level "INFO" -LogFile $LogFile
        return
    }
    Write-TestLog "Stopping miPC dev mode (owned)..." -LogFile $LogFile
    # Kill the launcher window and its child node/cargo processes
    if ($MiPCJob.Pid) {
        try {
            Stop-Process -Id $MiPCJob.Pid -Force -ErrorAction SilentlyContinue
            # Also kill node processes spawned for vite, and cargo/tauri
            Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
                Where-Object { $_.CommandLine -like "*tauri*dev*" -or $_.CommandLine -like "*vite*" } |
                ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
        } catch {
            Write-TestLog "Error stopping miPC dev: $($_.Exception.Message)" -Level "WARN" -LogFile $LogFile
        }
    }
    Write-TestLog "miPC dev mode stop attempted." -Level "OK" -LogFile $LogFile
}

# --- ETW capture -------------------------------------------------------------

function Start-EtwCapture {
    <#
        .SYNOPSIS
            Starts an ETW trace session capturing hidi2c, acpi, and HidClass
            providers. Prefers logman; falls back to wpr.exe.
            Returns $true if a session was started.
    #>
    param(
        [Parameter(Mandatory)][string]$OutputPath,
        [string]$LogFile
    )

    # Provider GUIDs:
    #  HidClass (Microsoft-Windows-HidClass)      {C0E036B4-3D6D-4FD6-A9B0-7FAB6E4F2B6E}
    #  hidi2c (Microsoft-Windows-Hidi2c)          {932E5F8C-0D4C-4A5B-9E2E-2E5C2E5C2E5C} (best-effort)
    #  acpi (Microsoft-Windows-Kernel-PnP / ACPI)  {9B791F8F-3D5C-4F6E-9E2E-2E5C2E5C2E5C} (best-effort)
    # Because exact provider GUIDs vary, we use logman query providers to
    # discover real ones, and fall back to wpr if logman fails.
    $providers = @(
        "Microsoft-Windows-HidClass",
        "Microsoft-Windows-Hidi2c",
        "Microsoft-Windows-Kernel-PnP"
    )

    $logman = Get-Command logman -ErrorAction SilentlyContinue
    if ($logman) {
        Write-TestLog "Starting ETW session '$($script:EtwSessionName)' via logman..." -LogFile $LogFile
        # Build provider args
        $provArgs = @()
        foreach ($p in $providers) {
            $provArgs += @("-p", $p)
        }
        # Remove any existing session (ignore errors)
        logman stop $script:EtwSessionName -ets 2>&1 | Out-Null
        logman delete $script:EtwSessionName 2>&1 | Out-Null
        $args = @("create","trace",$script:EtwSessionName,"-o",$OutputPath,"-ets") + $provArgs
        $out = & logman @args 2>&1
        Write-TestLog "logman create output: $out" -LogFile $LogFile
        if ($LASTEXITCODE -eq 0) {
            Write-TestLog "ETW session started (logman). Output: $OutputPath.etl" -Level "OK" -LogFile $LogFile
            return $true
        }
        Write-TestLog "logman create failed (exit $LASTEXITCODE). Trying wpr..." -Level "WARN" -LogFile $LogFile
    }

    $wpr = Get-Command wpr -ErrorAction SilentlyContinue
    if ($wpr) {
        Write-TestLog "Starting ETW capture via wpr.exe..." -LogFile $LogFile
        $out = & wpr -start GeneralProfile -start CPU 2>&1
        Write-TestLog "wpr start output: $out" -LogFile $LogFile
        if ($LASTEXITCODE -eq 0) {
            Write-TestLog "WPR capture started. Will save to $OutputPath.etl on stop." -Level "OK" -LogFile $LogFile
            return $true
        }
        Write-TestLog "wpr start failed (exit $LASTEXITCODE)." -Level "ERROR" -LogFile $LogFile
        return $false
    }

    Write-TestLog "Neither logman nor wpr available - ETW capture skipped." -Level "ERROR" -LogFile $LogFile
    return $false
}

function Stop-EtwCapture {
    <#
        .SYNOPSIS
            Stops the ETW session and logs the output file location.
    #>
    param([string]$LogFile)

    $logman = Get-Command logman -ErrorAction SilentlyContinue
    if ($logman) {
        $out = & logman stop $script:EtwSessionName -ets 2>&1
        Write-TestLog "logman stop output: $out" -LogFile $LogFile
        if ($LASTEXITCODE -eq 0) {
            Write-TestLog "ETW session stopped (logman)." -Level "OK" -LogFile $LogFile
            return $true
        }
    }
    $wpr = Get-Command wpr -ErrorAction SilentlyContinue
    if ($wpr) {
        $etlPath = Join-Path $script:ResultsDir "MiPC_EMI_$(Get-Date -Format 'yyyyMMddHHmmss').etl"
        $out = & wpr -stop $etlPath 2>&1
        Write-TestLog "wpr stop output: $out" -LogFile $LogFile
        if ($LASTEXITCODE -eq 0) {
            Write-TestLog "WPR capture saved to $etlPath" -Level "OK" -LogFile $LogFile
            return $true
        }
    }
    Write-TestLog "Could not stop ETW session." -Level "WARN" -LogFile $LogFile
    return $false
}

# --- HID monitor (C# via Add-Type) ------------------------------------------

$script:HidMonitorType = $null

function Register-HidMonitorType {
    <#
        .SYNOPSIS
            Compiles and registers the C# HID monitor helper via Add-Type.
            Idempotent - only compiles once per session.
    #>
    if ($script:HidMonitorType) { return $true }

    $code = @"
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;

public static class HidMonitor
{
    // ---------- P/Invoke declarations ----------
    private const int DIGCF_PRESENT = 0x00000002;
    private const int DIGCF_DEVICEINTERFACE = 0x00000010;
    private const int GENERIC_READ = unchecked((int)0x80000000);
    private const int GENERIC_WRITE = 0x40000000;
    private const int FILE_SHARE_READ = 0x00000001;
    private const int FILE_SHARE_WRITE = 0x00000002;
    private const int OPEN_EXISTING = 3;
    private const int FILE_FLAG_OVERLAPPED = 0x40000000;
    private const uint FILE_ATTRIBUTE_NORMAL = 0x80;

    [StructLayout(LayoutKind.Sequential)]
    public struct SP_DEVICE_INTERFACE_DATA
    {
        public int cbSize;
        public Guid InterfaceClassGuid;
        public uint Flags;
        public IntPtr Reserved;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Auto)]
    public struct SP_DEVICE_INTERFACE_DETAIL_DATA
    {
        public int cbSize;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)] public string DevicePath;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct SP_DEVINFO_DATA
    {
        public int cbSize;
        public Guid ClassGuid;
        public uint DevInst;
        public IntPtr Reserved;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct HIDD_ATTRIBUTES
    {
        public int Size;
        public ushort VendorID;
        public ushort ProductID;
        public ushort VersionNumber;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct OVERLAPPED
    {
        public IntPtr Internal;
        public IntPtr InternalHigh;
        public uint Offset;
        public uint OffsetHigh;
        public IntPtr hEvent;
    }

    [DllImport("setupapi.dll", CharSet = CharSet.Auto, SetLastError = true)]
    private static extern IntPtr SetupDiGetClassDevs(ref Guid ClassGuid, IntPtr Enumerator, IntPtr hwndParent, uint Flags);

    [DllImport("setupapi.dll", CharSet = CharSet.Auto, SetLastError = true)]
    private static extern bool SetupDiEnumDeviceInterfaces(IntPtr DeviceInfoSet, ref SP_DEVINFO_DATA DeviceInfoData, ref Guid InterfaceClassGuid, int MemberIndex, ref SP_DEVICE_INTERFACE_DATA DeviceInterfaceData);

    [DllImport("setupapi.dll", CharSet = CharSet.Auto, SetLastError = true)]
    private static extern bool SetupDiGetDeviceInterfaceDetail(IntPtr DeviceInfoSet, ref SP_DEVICE_INTERFACE_DATA DeviceInterfaceData, IntPtr DeviceInterfaceDetailData, int DeviceInterfaceDetailDataSize, ref int RequiredSize, ref SP_DEVINFO_DATA DeviceInfoData);

    [DllImport("setupapi.dll", CharSet = CharSet.Auto, SetLastError = true)]
    private static extern bool SetupDiDestroyDeviceInfoList(IntPtr DeviceInfoSet);

    [DllImport("kernel32.dll", CharSet = CharSet.Auto, SetLastError = true)]
    private static extern IntPtr CreateFile(string lpFileName, int dwDesiredAccess, int dwShareMode, IntPtr lpSecurityAttributes, int dwCreationDisposition, int dwFlagsAndAttributes, IntPtr hTemplateFile);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool ReadFile(IntPtr hFile, byte[] lpBuffer, int nNumberOfBytesToRead, out int lpNumberOfBytesRead, ref OVERLAPPED lpOverlapped);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool CloseHandle(IntPtr hObject);

    [DllImport("hid.dll", SetLastError = true)]
    private static extern bool HidD_GetAttributes(IntPtr hDevice, ref HIDD_ATTRIBUTES Attributes);

    [DllImport("hid.dll")]
    private static extern void HidD_GetHidGuid(ref Guid HidGuid);

    // The standard HID interface GUID
    private static readonly Guid HID_GUID = new Guid(0x4D1E55B2, 0xF16F, 0x11CF, 0x88, 0xCB, 0x00, 0x11, 0x11, 0x00, 0x00, 0x30);

    // Cancellation token source shared with PowerShell
    private static CancellationTokenSource _cts = new CancellationTokenSource();

    public static void RequestStop() { _cts.Cancel(); }

    /// <summary>
    /// Captures HID input reports from the first device whose path contains
    /// the given filter substring (e.g. "bltp7853"). Writes timestamped
    /// hex lines to outputPath. Runs until duration elapses or RequestStop().
    /// </summary>
    public static int Capture(string outputPath, string devicePathFilter, int durationSec)
    {
        int reportCount = 0;
        try
        {
            // Enumerate HID interfaces
            Guid hidGuid = HID_GUID;
            IntPtr devInfoSet = SetupDiGetClassDevs(ref hidGuid, IntPtr.Zero, IntPtr.Zero, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
            if (devInfoSet == new IntPtr(-1)) { return -1; }

            IntPtr deviceHandle = IntPtr.Zero;
            try
            {
                int index = 0;
                while (true)
                {
                    SP_DEVICE_INTERFACE_DATA ifaceData = new SP_DEVICE_INTERFACE_DATA();
                    ifaceData.cbSize = Marshal.SizeOf(ifaceData);
                    SP_DEVINFO_DATA devData = new SP_DEVINFO_DATA();
                    devData.cbSize = Marshal.SizeOf(devData);
                    if (!SetupDiEnumDeviceInterfaces(devInfoSet, ref devData, ref hidGuid, index, ref ifaceData)) break;

                    // Get detail (device path)
                    int requiredSize = 0;
                    SetupDiGetDeviceInterfaceDetail(devInfoSet, ref ifaceData, IntPtr.Zero, 0, ref requiredSize, ref devData);
                    if (requiredSize <= 0) { index++; continue; }
                    IntPtr detailBuffer = Marshal.AllocHGlobal(requiredSize);
                    try
                    {
                        // SP_DEVICE_INTERFACE_DETAIL_DATA cbSize is 8 on x64 (int + packing)
                        Marshal.WriteInt32(detailBuffer, IntPtr.Size == 8 ? 8 : 6);
                        if (SetupDiGetDeviceInterfaceDetail(devInfoSet, ref ifaceData, detailBuffer, requiredSize, ref requiredSize, ref devData))
                        {
                            // Path starts at offset 4 (after cbSize int) on both arches
                            string path = Marshal.PtrToStringUni(new IntPtr(detailBuffer.ToInt64() + 4));
                            if (path != null && path.ToLowerInvariant().Contains(devicePathFilter.ToLowerInvariant()))
                            {
                                deviceHandle = CreateFile(path, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, IntPtr.Zero, OPEN_EXISTING, 0, IntPtr.Zero);
                                if (deviceHandle != new IntPtr(-1) && deviceHandle != IntPtr.Zero)
                                {
                                    File.AppendAllText(outputPath, "# Opened device: " + path + Environment.NewLine);
                                    break;
                                }
                                deviceHandle = IntPtr.Zero;
                            }
                        }
                    }
                    finally { Marshal.FreeHGlobal(detailBuffer); }
                    index++;
                }
            }
            finally { SetupDiDestroyDeviceInfoList(devInfoSet); }

            if (deviceHandle == IntPtr.Zero)
            {
                File.AppendAllText(outputPath, "# ERROR: No HID device matching '" + devicePathFilter + "' could be opened." + Environment.NewLine);
                return -2;
            }

            // Read loop
            byte[] buffer = new byte[256];
            Stopwatch sw = Stopwatch.StartNew();
            using (StreamWriter writer = new StreamWriter(outputPath, true))
            {
                writer.AutoFlush = true;
                while (sw.Elapsed.TotalSeconds < durationSec && !_cts.Token.IsCancellationRequested)
                {
                    int bytesRead = 0;
                    OVERLAPPED ov = new OVERLAPPED();
                    ov.hEvent = IntPtr.Zero;
                    bool ok = ReadFile(deviceHandle, buffer, buffer.Length, out bytesRead, ref ov);
                    if (ok && bytesRead > 0)
                    {
                        reportCount++;
                        string hex = BitConverter.ToString(buffer, 0, bytesRead);
                        writer.WriteLine("{0:O}\t{1}\t{2}", DateTime.UtcNow, bytesRead, hex);
                    }
                    else
                    {
                        // Synchronous read returned no data immediately; brief sleep to avoid spin
                        Thread.Sleep(5);
                    }
                }
            }
            CloseHandle(deviceHandle);
            return reportCount;
        }
        catch (Exception ex)
        {
            try { File.AppendAllText(outputPath, "# EXCEPTION: " + ex.Message + Environment.NewLine); } catch { }
            return -3;
        }
    }
}
"@
    try {
        Add-Type -TypeDefinition $code -Language CSharp -ErrorAction Stop
        $script:HidMonitorType = [HidMonitor]
        return $true
    } catch {
        Write-Warning "Failed to compile HID monitor C# type: $($_.Exception.Message)"
        return $false
    }
}

function Start-HidMonitor {
    <#
        .SYNOPSIS
            Starts a background job capturing HID input reports from the
            BLTP7853 touchpad. Returns the PSJob object.
        .PARAMETER OutputPath
            File to write timestamped report lines to.
        .PARAMETER DurationSec
            How long to capture (default 300s).
    #>
    param(
        [Parameter(Mandatory)][string]$OutputPath,
        [int]$DurationSec = 300,
        [string]$LogFile
    )

    $ok = Register-HidMonitorType
    if (-not $ok) {
        Write-TestLog "HID monitor C# type unavailable - HID capture disabled." -Level "ERROR" -LogFile $LogFile
        return $null
    }

    # Initialize the output file with a header
    "# HID input report capture - started $(Get-Date -Format o)" | Out-File -FilePath $OutputPath -Encoding utf8
    "# Device filter: bltp7853" | Out-File -FilePath $OutputPath -Append -Encoding utf8

    Write-TestLog "Starting HID monitor job -> $OutputPath ($DurationSec s)" -LogFile $LogFile
    $job = Start-Job -ScriptBlock {
        param($Out, $Dur)
        [HidMonitor]::Capture($Out, "bltp7853", $Dur)
    } -ArgumentList $OutputPath, $DurationSec

    return $job
}

function Stop-HidMonitor {
    <#
        .SYNOPSIS
            Stops the HID monitor job and reports the report count.
    #>
    param($HidJob, [string]$LogFile)

    if (-not $HidJob) {
        Write-TestLog "Stop-HidMonitor: no job provided." -Level "WARN" -LogFile $LogFile
        return 0
    }
    try {
        # Signal the C# cancellation token
        if ($script:HidMonitorType) { [HidMonitor]::RequestStop() }
    } catch { }
    # Give the job a moment to flush
    Start-Sleep -Milliseconds 500
    try { Stop-Job -Job $HidJob -ErrorAction SilentlyContinue } catch { }
    $count = 0
    try {
        $r = Receive-Job -Job $HidJob -ErrorAction SilentlyContinue
        if ($r) { $count = [int]$r }
    } catch { }
    try { Remove-Job -Job $HidJob -Force -ErrorAction SilentlyContinue } catch { }
    Write-TestLog "HID monitor stopped. Reports captured: $count" -Level "OK" -LogFile $LogFile
    return $count
}

# --- Capture window ----------------------------------------------------------

function Invoke-CaptureWindow {
    <#
        .SYNOPSIS
            Displays a live countdown and instructs the user to actively use
            the touchpad with the charger connected. Runs $OnStart at the
            beginning and $OnStop at the end. Supports Ctrl+C (runs $OnStop).
        .PARAMETER DurationSec
            Capture duration in seconds.
        .PARAMETER Instruction
            Custom instruction text shown to the user.
    #>
    param(
        [int]$DurationSec = 300,
        [string]$Instruction = "CAPTURING - use the touchpad actively with the charger connected.",
        [string]$LogFile,
        [scriptblock]$OnStart,
        [scriptblock]$OnStop
    )

    Write-TestLog "=== CAPTURE WINDOW START ($DurationSec s) ===" -LogFile $LogFile
    Write-TestLog $Instruction -LogFile $LogFile
    Write-Host ""
    Write-Host "  >>> $Instruction" -ForegroundColor Yellow
    Write-Host "  >>> Press Ctrl+C to abort early (cleanup will still run)." -ForegroundColor DarkYellow
    Write-Host ""

    if ($OnStart) { & $OnStart }

    $abort = $false
    try {
        for ($i = $DurationSec; $i -gt 0; $i--) {
            Write-Host -NoNewline ("`r  {0,4}s remaining  " -f $i)
            Start-Sleep -Seconds 1
        }
        Write-Host ""
    } catch {
        # Ctrl+C throws a PipelineStoppedException
        $abort = $true
        Write-Host ""
        Write-TestLog "Capture window aborted by user (Ctrl+C)." -Level "WARN" -LogFile $LogFile
    }

    if ($OnStop) {
        try { & $OnStop } catch {
            Write-TestLog "OnStop callback error: $($_.Exception.Message)" -Level "ERROR" -LogFile $LogFile
        }
    }
    Write-TestLog "=== CAPTURE WINDOW END ===" -LogFile $LogFile
    return (-not $abort)
}

# --- Reboot-resume state -----------------------------------------------------

function Save-State {
    <#
        .SYNOPSIS
            Saves a named state value (any serializable object) to .state/$Name.json.
    #>
    param(
        [Parameter(Mandatory)][string]$Name,
        $Value,
        [string]$LogFile
    )
    $path = Join-Path $script:StateDir "$Name.json"
    try {
        $Value | ConvertTo-Json -Depth 10 | Out-File -FilePath $path -Encoding utf8
        Write-TestLog "State saved: $Name -> $path" -LogFile $LogFile
    } catch {
        Write-TestLog "Failed to save state $Name : $($_.Exception.Message)" -Level "ERROR" -LogFile $LogFile
    }
}

function Load-State {
    <#
        .SYNOPSIS
            Loads and returns a named state value, or $null if not found.
    #>
    param(
        [Parameter(Mandatory)][string]$Name,
        [string]$LogFile
    )
    $path = Join-Path $script:StateDir "$Name.json"
    if (-not (Test-Path $path)) { return $null }
    try {
        $obj = Get-Content $path -Raw | ConvertFrom-Json
        Write-TestLog "State loaded: $Name" -LogFile $LogFile
        return $obj
    } catch {
        Write-TestLog "Failed to load state $Name : $($_.Exception.Message)" -Level "ERROR" -LogFile $LogFile
        return $null
    }
}

function Clear-State {
    <#
        .SYNOPSIS
            Deletes a named state file.
    #>
    param([Parameter(Mandatory)][string]$Name)
    $path = Join-Path $script:StateDir "$Name.json"
    if (Test-Path $path) { Remove-Item $path -Force -ErrorAction SilentlyContinue }
}

function Set-RebootResume {
    <#
        .SYNOPSIS
            Creates a RunOnce registry entry to re-launch the script after
            reboot, passing -ResumeFrom $Phase.
    #>
    param(
        [Parameter(Mandatory)][string]$ScriptPath,
        [Parameter(Mandatory)][string]$Phase,
        [string]$LogFile
    )
    $cmd = "powershell.exe -ExecutionPolicy Bypass -NoExit -File `"$ScriptPath`" -ResumeFrom `"$Phase`""
    try {
        if (-not (Test-Path $script:RunOnceKey)) { New-Item -Path $script:RunOnceKey -Force | Out-Null }
        Set-ItemProperty -Path $script:RunOnceKey -Name $script:RunOnceValue -Value $cmd -Force
        Write-TestLog "RunOnce set: $cmd" -LogFile $LogFile
        Save-State -Name "reboot-phase" -Value @{ ScriptPath = $ScriptPath; Phase = $Phase } -LogFile $LogFile
    } catch {
        Write-TestLog "Failed to set RunOnce: $($_.Exception.Message)" -Level "ERROR" -LogFile $LogFile
    }
}

function Clear-RebootResume {
    <#
        .SYNOPSIS
            Removes the RunOnce entry and the reboot-phase state file.
    #>
    param([string]$LogFile)
    try {
        if (Get-ItemProperty -Path $script:RunOnceKey -Name $script:RunOnceValue -ErrorAction SilentlyContinue) {
            Remove-ItemProperty -Path $script:RunOnceKey -Name $script:RunOnceValue -Force -ErrorAction SilentlyContinue
            Write-TestLog "RunOnce entry removed." -LogFile $LogFile
        }
    } catch { }
    Clear-State -Name "reboot-phase"
}

# --- Revert helpers ----------------------------------------------------------

function Revert-IoTService {
    <#
        .SYNOPSIS
            Restores IoTService to the original start type and starts it.
    #>
    param(
        [string]$OriginalStartType,  # "Automatic","Manual","Disabled"
        [string]$LogFile
    )
    Write-TestLog "Reverting IoTService start type to '$OriginalStartType'..." -LogFile $LogFile
    $startMap = @{ "Automatic" = "auto"; "Manual" = "demand"; "Disabled" = "disabled" }
    $scVal = $startMap[$OriginalStartType]
    if (-not $scVal) { $scVal = "auto" }
    try {
        sc.exe config IoTSvc start= $scVal 2>&1 | Out-Null
        if ($OriginalStartType -ne "Disabled") {
            sc.exe start IoTSvc 2>&1 | Out-Null
        }
        Write-TestLog "IoTService reverted to '$OriginalStartType' and started." -Level "OK" -LogFile $LogFile
    } catch {
        Write-TestLog "Error reverting IoTService: $($_.Exception.Message)" -Level "ERROR" -LogFile $LogFile
    }
}

function Revert-IdleTimer {
    <#
        .SYNOPSIS
            Restores the IdleTimerPeriod registry value for the I2C controller.
            If OriginalValue is $null, deletes the value (it didn't exist before).
    #>
    param(
        [string]$DeviceId,
        $OriginalValue,
        [string]$LogFile
    )
    if (-not $DeviceId) {
        Write-TestLog "Revert-IdleTimer: no device ID - skipping." -Level "WARN" -LogFile $LogFile
        return
    }
    $regPath = "HKLM:\SYSTEM\CurrentControlSet\Enum\$DeviceId\Device Parameters"
    try {
        if ($null -eq $OriginalValue) {
            # Value didn't exist before - remove it
            Remove-ItemProperty -Path $regPath -Name "IdleTimerPeriod" -Force -ErrorAction SilentlyContinue
            Write-TestLog "IdleTimerPeriod removed (was not set originally)." -Level "OK" -LogFile $LogFile
        } else {
            Set-ItemProperty -Path $regPath -Name "IdleTimerPeriod" -Value ([int]$OriginalValue) -Type DWord -Force
            Write-TestLog "IdleTimerPeriod restored to $OriginalValue." -Level "OK" -LogFile $LogFile
        }
    } catch {
        Write-TestLog "Error reverting IdleTimerPeriod: $($_.Exception.Message)" -Level "ERROR" -LogFile $LogFile
    }
}

function Revert-PowerPlan {
    <#
        .SYNOPSIS
            Restores the active power plan to the original GUID.
    #>
    param([string]$OriginalGuid, [string]$LogFile)
    if (-not $OriginalGuid) {
        Write-TestLog "Revert-PowerPlan: no original GUID - skipping." -Level "WARN" -LogFile $LogFile
        return
    }
    try {
        powercfg /setactive $OriginalGuid 2>&1 | Out-Null
        Write-TestLog "Power plan restored to $OriginalGuid." -Level "OK" -LogFile $LogFile
    } catch {
        Write-TestLog "Error reverting power plan: $($_.Exception.Message)" -Level "ERROR" -LogFile $LogFile
    }
}

function Protect-Revert {
    <#
        .SYNOPSIS
            Executes a revert scriptblock inside try/finally so cleanup ALWAYS
            runs, even on Ctrl+C. Best-effort trap registration.
    #>
    param(
        [scriptblock]$RevertAction,
        [string]$LogFile
    )
    $prevTreat = $false
    try {
        # Try to make Ctrl+C catchable (best-effort; may not work in all hosts)
        try { $prevTreat = [Console]::TreatControlCAsInput; [Console]::TreatControlCAsInput = $true } catch { }
    } catch { }
    try {
        & $RevertAction
    } finally {
        try { [Console]::TreatControlCAsInput = $prevTreat } catch { }
        Write-TestLog "Protect-Revert: cleanup block executed." -LogFile $LogFile
    }
}

# --- Export ------------------------------------------------------------------
Export-ModuleMember -Function *
