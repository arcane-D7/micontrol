# Reverse Engineering Analysis Report: IoTService.exe & IoTDriver.sys

## Executive Summary

This report documents the complete reverse engineering analysis of Xiaomi's
IoTService.exe and IoTDriver.sys kernel driver, performed to understand their
architecture, security model, and communication protocols. A custom replacement
binary (`ecram_service.exe`) was built and successfully tested against the
existing driver, confirming all findings.

---

## 1. File Identification

### IoTService.exe

| Property         | Value                                                                                                |
| ---------------- | ---------------------------------------------------------------------------------------------------- |
| **Location**     | `C:\Windows\System32\DriverStore\FileRepository\iotdriver.inf_amd64_a0672b04d766f7de\IoTService.exe` |
| **Size**         | 747,112 bytes                                                                                        |
| **SHA256**       | 29E89BA0DAA83D29F2A9083B8704FFF25D668B736664B5D6B27312971B0D4D2E                                     |
| **Format**       | PE32+ x86-64, Windows CUI subsystem                                                                  |
| **Compiled**     | Mon Mar 2 06:20:19 2026                                                                              |
| **PDB Path**     | `D:\Work\IoTDriver\x64\Release\IoTService.pdb`                                                       |
| **Signing**      | DigiCert/Microsoft WHCP                                                                              |
| **Manifest**     | asInvoker (does not self-elevate)                                                                    |
| **Service Name** | IoTSvc (Automatic, LocalSystem)                                                                      |

### IoTDriver.sys

| Property         | Value                                                            |
| ---------------- | ---------------------------------------------------------------- |
| **Size**         | 45,672 bytes                                                     |
| **SHA256**       | F5654384DE7EA4F1E4B1BBA0FE4278DE5E2B9FCA74646A08C420DFC028892395 |
| **Format**       | PE32+ x86-64, Native subsystem                                   |
| **Compiled**     | Tue Mar 3 14:46:52 2026                                          |
| **PDB Path**     | `D:\Work\IoTDriver\x64\Release\IoTDriver.pdb`                    |
| **Framework**    | KMDF driver v1.0.0.3, product 25.0.0.4                           |
| **Service Name** | IoTDriver (Running)                                              |
| **Secure Boot**  | Enabled (driver is signed)                                       |

---

## 2. IoTDriver.sys Architecture

### 2.1 Device Interface GUID

```
{AB7924A1-3162-4010-B33B-837E87E25FBC}
```

This GUID is used by `SetupDiGetClassDevsW` to locate the device path for
`CreateFileW` communication.

### 2.2 IOCTL Codes

| IOCTL Code | Purpose                       | Buffer Size       |
| ---------- | ----------------------------- | ----------------- |
| `0x22E000` | READ ECRAM (physical memory)  | 0x110 (272) bytes |
| `0x22E004` | WRITE ECRAM (physical memory) | 0x110 (272) bytes |

### 2.3 Buffer Layout (0x110 bytes = 272 bytes)

```c
struct EcramBuf {       // 0x110 bytes total
    uint64_t physical_address;  // offset 0x00: target physical address
    uint64_t byte_count;        // offset 0x08: number of bytes to read/write
    uint8_t  data[0x100];      // offset 0x10: data (write) / output (read)
};
```

**Compile-time assertion**: `size_of::<EcramBuf>() == 0x110`

### 2.4 Allowed Physical Address Ranges

The driver validates that requested physical addresses fall within one of
three hardcoded ranges. Any address outside these ranges returns
`STATUS_ACCESS_DENIED (0xC0000022)`.

**Address ranges table** (at offset `0x140004370`, 3 entries × 16 bytes):

| #   | Base Address | Size                | Description                          |
| --- | ------------ | ------------------- | ------------------------------------ |
| 1   | `0xFE0B0F00` | `0x80` (128 bytes)  | IoTDriver status block + sensor data |
| 2   | `0xFE0B0AB8` | `0x08` (8 bytes)    | Small status region                  |
| 3   | `0xFE0B0E00` | `0x100` (256 bytes) | ECRAM sensor block                   |

**CRITICAL**: The following regions are NOT accessible:

- **ERAM** (`0xFE0B0300`, 256 bytes) — ❌ NOT in allowed ranges
- **SMA2** (`0xFE0B0A00`, 256 bytes) — ❌ NOT in allowed ranges

This means the AC adapter wattage (ADPW at ERAM+0x81 = `0xFE0B0381`) is
**NOT readable** through the existing driver, even with a correctly named
process.

### 2.5 Security Check (Process Name Validation)

**This is the KEY BLOCKER for direct access from MiControl.exe.**

The driver's IOCTL dispatch function (`fcn.1400019f0`) performs:

1. **`IoGetCurrentProcess()`** — Gets the calling process's EPROCESS
2. **`SeLocateProcessImageName()`** — Gets the full NT path of the process image
3. **`fcn.140001738`** — Resolves the NT path to a DOS path by iterating
   drive letters C: through Z: (`\??\%wc:` format), opening each as a
   symbolic link via `ZwOpenSymbolicLinkObject` + `ZwQuerySymbolicLinkObject`,
   and checking if the process path starts with the resolved drive path
   (`RtlPrefixUnicodeString`)
4. **`RtlCompareUnicodeString`** — Compares the filename portion against
   the hardcoded string `"IoTService.exe"` (at offset `0x140003c50`)

The check is: **process must be named "IoTService.exe" AND located in the
driver's own directory** (the DriverStore path).

If the check fails, the driver logs `"Access denied, Path: %wZ"` via
`DbgPrint` and returns `STATUS_ACCESS_DENIED`.

### 2.6 Read/Write Mechanism

Both read and write handlers use:

1. **`MmMapIoSpace`** — Maps the physical EC RAM address into kernel virtual memory
2. **`RtlCopyMemory`** (`fcn.140003640`) — Copies data between user buffer and mapped memory
3. **`MmUnmapIoSpace`** — Unmaps the memory

**IRQL Check**: `KeGetCurrentIrql() <= 1` (PASSIVE_LEVEL or APC_LEVEL)

### 2.7 Handshake Mechanism

The original IoTService.exe sends a "ReportLaptopStatus(IOT_WIN_READY)"
handshake before performing ECRAM operations. This is done by sending a
zeroed 0x110-byte buffer via IOCTL 0x22E000 (READ).

**Testing showed**: The handshake is NOT strictly required for ECRAM reads
to succeed when the process name check passes. The driver sets a global flag
but does not block reads if the flag is unset (at least for the allowed
address ranges).

---

## 3. IoTService.exe Architecture

### 3.1 Source Files (from debug strings)

- `IoT.cpp` — Main entry point
- `IoTDriver.cpp` — Driver communication
- `RamIO.cpp` — RamDevice class (read/write ECRAM)
- `OnServiceEvent.cpp` — Service event handling
- `RunAsHelper.cpp` — Elevated process launching
- `Util_Driver.cpp` — Driver utility functions
- `Util_Dump.cpp` — Diagnostic dump utilities
- `worker.cpp` — Worker thread management
- `Worker_IPCBroker.cpp` — IPC broker worker
- `Worker_WMI.cpp` — WMI event monitoring worker
- `Worker_IPC.cpp` — IPC communication worker

### 3.2 Key Functionality

1. **Windows Service** — Registered with SCM as "IoTSvc", runs as LocalSystem
2. **WMI Event Sink** — `CWmiEventSink` implements `IWbemObjectSink` for WMI events
3. **Named Pipe IPC** — `\\.\pipe\LOCAL\IoTService_IPC_Broker` for inter-process communication
4. **Power Event Handling** — Shutdown/sleep/hibernate via `CreateProcessAsUserW`/`SetSuspendState`
5. **WiFi Status Monitoring** — Tracks WiFi connection state
6. **Device Bind/Unbind** — Manages device state
7. **Laptop Status Reporting** — Reports status to driver via IOCTL

### 3.3 DeviceIoControl Call Sites

Two call sites found, both in `RamIO.cpp`:

| Function           | Address         | IOCTL      | Purpose     |
| ------------------ | --------------- | ---------- | ----------- |
| `RamDevice::Read`  | `fcn.14002e350` | `0x22E000` | Read ECRAM  |
| `RamDevice::Write` | `fcn.14002e660` | `0x22E004` | Write ECRAM |

Both use the 0x110-byte `EcramBuf` structure and open the device via
`SetupDiGetClassDevsW` with GUID `{AB7924A1-3162-4010-B33B-837E87E25FBC}`.

### 3.4 Registry Keys

- `SOFTWARE\MI\IoTDriver` — Enable, FwVersion, Uid, SSID, WiFiStatus
- `SOFTWARE\MI\IoTService` — LogLevel

### 3.5 Logging

Uses `spdlog` with rotating file sink at:
`C:\ProgramData\MI\IoTService\service.log`

### 3.6 Key Imports

- **KERNEL32**: DeviceIoControl, CreateFileW, CreateNamedPipeW, ConnectNamedPipe,
  CreateServiceW, StartServiceCtrlDispatcherW
- **USER32**: PostMessageW, SendInput, RegisterPowerSettingNotification, ExitWindowsEx
- **ADVAPI32**: Service management, registry, token privileges, CreateProcessAsUserW
- **ole32**: COM/WMI
- **OLEAUT32**: SafeArray
- **SETUPAPI**: Device enumeration
- **WTSAPI32**: WTSQueryUserToken
- **USERENV**: CreateEnvironmentBlock
- **POWRPROF**: SetSuspendState

---

## 4. Custom IoTService.exe Replacement

### 4.1 Implementation

A custom replacement binary was built in Rust (`ecram_service.rs`) that:

1. **Named `IoTService.exe`** — Passes the driver's process name security check
2. **Placed in driver directory** — Passes the path prefix check
3. **Provides named pipe IPC** — `\\.\pipe\ecram_service` for MiControl to communicate
4. **Proxies ECRAM read/write** — Forwards requests to IoTDriver.sys via IOCTLs
5. **Supports CLI mode** — For testing and direct operation
6. **Supports service mode** — Can register with SCM

### 4.2 IPC Protocol (JSON over named pipe)

**Requests**:

```json
{"op":"read","addr":"0xFE0B0F00","size":8}
{"op":"write","addr":"0xFE0B0F00","data":"DEADBEEF"}
{"op":"read_region","region":"IOT_STATUS"}
{"op":"ping"}
```

**Responses**:

```json
{"ok":true,"addr":"0xFE0B0F00","size":8,"data":"0010010301000000"}
{"ok":true,"addr":"0xFE0B0F00","bytes_written":8}
{"ok":true,"region":"IOT_STATUS","addr":"0xFE0B0F00","size":8,"data":"0010010301000000"}
{"ok":true,"pong":true}
```

### 4.3 Known ECRAM Regions

```rust
pub const REGIONS: &[(&str, u64, usize)] = &[
    ("ERAM",        0xFE0B0300, 0x100),  // ❌ NOT accessible (not in allowed ranges)
    ("SMA2",        0xFE0B0A00, 0x100),  // ❌ NOT accessible (not in allowed ranges)
    ("IOT_STATUS",  0xFE0B0F00, 0x08),   // ✅ Accessible (allowed range 1)
    ("IOT_SENSORS", 0xFE0B0F08, 0x78),   // ✅ Accessible (allowed range 1)
];
```

### 4.4 Test Results

All tests performed with the binary named `IoTService.exe` and placed in the
driver's DriverStore directory:

| Test             | Address    | Size | Result                                   |
| ---------------- | ---------- | ---- | ---------------------------------------- |
| Handshake        | 0x00000000 | 0    | ⚠️ Parameter error (non-critical)        |
| Read IOT_STATUS  | 0xFE0B0F00 | 8    | ✅ `0010010301000000`                    |
| Read IOT_SENSORS | 0xFE0B0F08 | 120  | ✅ All zeros (sensors not active)        |
| Read range 2     | 0xFE0B0AB8 | 8    | ✅ All zeros                             |
| Read ECRAM block | 0xFE0B0E00 | 256  | ✅ All zeros                             |
| Read ERAM        | 0xFE0B0300 | 256  | ❌ Access denied (not in allowed ranges) |
| Read SMA2        | 0xFE0B0A00 | 256  | ❌ Access denied (not in allowed ranges) |
| Write IOT_STATUS | 0xFE0B0F00 | 8    | ✅ 8 bytes written                       |
| Pipe ping        | —          | —    | ✅ `{"ok":true,"pong":true}`             |
| Pipe read_region | IOT_STATUS | 8    | ✅ Data returned                         |

### 4.5 Limitations

1. **ERAM region not accessible via IoTDriver** — The driver's allowed address ranges do not
   include ERAM (0xFE0B0300). This means AC adapter wattage (ADPW at ERAM+0x81)
   cannot be read through the existing driver. However, AC adapter wattage IS
   available via WMI (ACPI WMAA method `read_adapter_power()` in `wmi_ec.rs`),
   which is the primary path used by MiControl.
2. **SMA2 region not accessible via IoTDriver** — Similarly not in allowed ranges.
3. **Secure Boot prevents driver modification** — Modifying IoTDriver.sys to
   add ERAM/SMA2 to the allowed ranges would require re-signing the driver,
   which is not possible with Secure Boot enabled.
4. **Process name must match** — The binary must be named `IoTService.exe`
   and placed in the driver's directory.

---

## 5. MiControl Integration Strategy

### 5.1 What Works Without IoTDriver

Most MiControl features work via WMI (`MICommonInterface` in `root\WMI`):

- ✅ Performance mode (ACPI WMAA method)
- ✅ Battery health
- ✅ Adapter power status
- ✅ Fan speed monitoring

### 5.2 What Requires IoTDriver (via custom IoTService.exe)

- ✅ IOT_STATUS region (0xFE0B0F00) — Driver status
- ✅ IOT_SENSORS region (0xFE0B0F08) — Sensor data
- ✅ ECRAM sensor block (0xFE0B0E00) — EC sensor readings
- ❌ ERAM region (0xFE0B0300) — AC adapter wattage (ADPW) — NOT accessible via IoTDriver (but available via WMI)
- ❌ SMA2 region (0xFE0B0A00) — NOT accessible via IoTDriver

### 5.3 Deployment

1. Build `ecram_service.exe` from `micontrol/src-tauri/src/bin/ecram_service.rs`
2. Rename to `IoTService.exe`
3. Copy to `C:\Windows\System32\DriverStore\FileRepository\iotdriver.inf_amd64_a0672b04d766f7de\`
4. The existing IoTSvc service will automatically use the new binary on next start
5. MiControl connects to `\\.\pipe\ecram_service` for ECRAM access

---

## 6. Security Considerations

### 6.1 Driver Security Model

The driver's security model is based on:

1. **Process name validation** — Must be "IoTService.exe"
2. **Path validation** — Must be in the driver's directory
3. **Address range validation** — Only 3 hardcoded ranges are accessible
4. **IRQL check** — Must be at PASSIVE_LEVEL or APC_LEVEL

### 6.2 Bypassing the Process Name Check

The process name check can be bypassed by:

1. Naming the binary `IoTService.exe` ✅ (tested and confirmed)
2. Placing it in the driver's DriverStore directory ✅ (tested and confirmed)

No admin token check was found in the IOCTL dispatch path (the `SeTokenIsAdmin`
call mentioned in the summary appears to be in a different code path, not in
the main IOCTL dispatch).

### 6.3 Secure Boot

Secure Boot is enabled, which prevents:

- Loading unsigned drivers
- Loading modified signed drivers (signature verification fails)

This means IoTDriver.sys cannot be modified to add ERAM/SMA2 to the allowed
address ranges without disabling Secure Boot.

---

## 7. Conclusions

1. **Custom IoTService.exe works** — Successfully built, tested, and verified
   to communicate with IoTDriver.sys for all allowed address ranges.
2. **ERAM/SMA2 are fundamentally blocked** — The driver's hardcoded address
   ranges do not include these regions, and Secure Boot prevents driver
   modification.
3. **Most MiControl features work via WMI** — The WMI interface provides
   performance mode, battery health, and adapter power without needing
   IoTDriver at all.
4. **The custom binary provides additional sensor data** — IOT_STATUS,
   IOT_SENSORS, and the ECRAM sensor block (0xFE0B0E00) are accessible
   through the custom IoTService.exe.
5. **Named pipe IPC is functional** — MiControl can communicate with the
   custom service via JSON over named pipe.

---

## Appendix A: radare2 Analysis Files

All analysis files are stored in:
`C:\Users\mafsc\Documents\Projects\miPC\backups\IoTService_original_backup\`

| File                            | Content                                        |
| ------------------------------- | ---------------------------------------------- |
| `r2_analysis.txt`               | IoTService.exe basic info, imports, strings    |
| `r2_functions.txt`              | IoTService.exe function list (1,512 functions) |
| `r2_ram_read.txt`               | RamDevice::Read disassembly                    |
| `r2_ram_write.txt`              | RamDevice::Write disassembly                   |
| `r2_driver_analysis.txt`        | IoTDriver.sys basic info, imports, strings     |
| `r2_driver_ioctl_handler.txt`   | Driver IOCTL dispatch function                 |
| `r2_driver_ioctl_dispatch*.txt` | Driver IOCTL dispatch (multiple parts)         |
| `r2_driver_ioctl_read*.txt`     | Driver read handler                            |
| `r2_driver_ioctl_write.txt`     | Driver write handler                           |
| `r2_driver_addr_check.txt`      | Address validation function                    |
| `r2_driver_page.txt`            | Address ranges table dump                      |
| `r2_driver_device_control.txt`  | Device control function                        |
| `r2_driver_ioctl_main.txt`      | Main IOCTL handler                             |
| `r2_driver_ioctl_handler2.txt`  | IOCTL handler (part 2)                         |
| `ascii_strings.txt`             | All ASCII strings from IoTService.exe          |
| `r2_all_strings.txt`            | All strings from IoTService.exe                |
| `r2_key_strings.txt`            | Key strings from IoTService.exe                |
| `r2_filtered_strings.txt`       | Filtered strings from IoTService.exe           |

## Appendix B: Backups

| File                 | Description                    |
| -------------------- | ------------------------------ |
| `IoTService.exe.bak` | Original IoTService.exe backup |
| `IoTDriver.sys.bak`  | Original IoTDriver.sys backup  |

## Appendix C: Source Code

Custom IoTService.exe replacement source:
`micontrol/src-tauri/src/bin/ecram_service.rs`

Existing ECRAM access module (used by MiControl directly):
`micontrol/src-tauri/src/hw/ecram.rs`
