# Hardware Investigation Report — Xiaomi Book Pro 14 (TM2424)

> **✅ SINGLE SOURCE OF TRUTH — Last updated 2026-06-30 (Session 6: custom IoTService.exe + RE report)**
>
> This document consolidates all findings from Sessions 1–6, including:
>
> - ACPI DSDT/SSDT decompilation (keyboard backlight, fan, ERAM field map)
> - Ghidra decompilation of IoTService.exe and IoTDriver.sys
> - MCPI IPC protocol reverse engineering
> - WMI WMAA runtime verification (17 commands + backlight read)
> - WMI WMAA write verification (all 13 write commands, brightness/MIUT read-back confirmed)
> - HQWmiCommonInterface runtime verification (11 methods)
> - EC RAM direct IOCTL testing (blocked by driver process name check)
> - ACPI \_WDG buffer decode (WMI event GUID discovery)
> - WMI event registration testing (WMIEvent extrinsic events)
> - **HID_EVENT20-23 WMI event subscription — ✅ WORKING (KBLL breakthrough)**
> - DBICommonInterface.GetODMSSID runtime verification
> - IoTService IPC Broker pipe connection testing
> - IoTService.exe string analysis (RamDevice, CWmiEventSink, IPC protocol)
> - EC RAM field map complete decode (LBLM, DBLL, KBLL, all fields)
> - IoTDriver.sys blocks ALL IOCTLs to IOTD0000 (not just EC RAM)
> - Proxy process approach tested and failed (driver checks full path, not just filename)
> - WMAA FUN2/FUN3 full scan — no hidden KBLL command exists
> - Complete hotkey event mapping (Fn+F4, F7, F8, F9, F10, Xiaomi logo key)
> - **Session 6 (30/06/2026): Custom IoTService.exe built and tested — ECRAM IOCTL proxy via named pipe IPC**
> - **Session 6: IoTDriver.sys allowed address ranges confirmed (0xFE0B0F00/0x80, 0xFE0B0AB8/0x08, 0xFE0B0E00/0x100)**
> - **Session 6: ERAM (0xFE0B0300) and SMA2 (0xFE0B0A00) confirmed NOT accessible via IoTDriver — not in allowed ranges (AC adapter wattage available via WMI instead)**
> - **Session 6: Security check bypassed by naming binary IoTService.exe + placing in DriverStore dir**
> - **Session 6: Pipe client added to ecram.rs (read_ecram_via_pipe, is_pipe_broker_available)**

## System Overview

| Field       | Value         |
| ----------- | ------------- |
| Model       | TM2424        |
| BIOS        | XMAPT4B0P0909 |
| BaseBoard   | TM2424 V14E1  |
| EC Firmware | 1.9           |
| OS          | Windows 11    |

## Hardware Access Layers

```
┌─────────────────────────────────────────────────────────────────┐
│                    MiControl App (Rust/Tauri)                    │
├─────────────┬──────────────────┬──────────────────┬──────────────┤
│  IoTService │  EC RAM Direct   │  WMI Classes     │  Registry    │
│  IPC Pipe   │  (IoTDriver.sys  │  (MICommon/HQ)   │  (MI/Reg)    │
│  (MCPI)     │   IOCTLs)        │                  │              │
├─────────────┼──────────────────┼──────────────────┼──────────────┤
│ IoTService  │ IoTDriver.sys    │ WMI Provider     │ HKLM\...     │
│ .exe        │ (KMDF kernel      │ (MICommonInterface│            │
│ v25.0.0.9   │  driver)          │  HQWmiCommonInterface)          │
├─────────────┼──────────────────┼──────────────────┴──────────────┤
│ Named Pipe  │ DeviceIoControl   │ WMI COM (Win32)                 │
│ \\.\pipe\   │ GUID: AB7924A1-.. │                                 │
│ LOCAL\IoT.. │ (BLOCKED)         │                                 │
└─────────────┴──────────────────┴──────────────────────────────────┘
```

---

## 1. WMI WMAA (MICommonInterface) — ✅ WORKING

### Overview

The primary hardware access path. Uses ACPI WMAA method via WMI COM to read/write
EC registers. Requires admin privileges but **bypasses IoTDriver.sys process name check**.

- **WMI class**: `MICommonInterface` (ROOT\WMI)
- **Instance**: `ACPI\PNP0C14\MIFS_0`
- **Method**: `MiInterface(uint8[] InData) → (uint8[] OutData, uint16 ReturnCode)`
- **Limits**: InData MAX=32 bytes, OutData MAX=30 bytes
- **Rust implementation**: `src-tauri/src/hw/wmi_ec.rs`

### WMAA Buffer Format

```
Input (InData, 32 bytes):
  Offset  Size  Field  Description
    0     word  FUN1   0xFA00 = read, 0xFB00 = write
    2     word  FUN2   Sub-command group (0x0800, 0x0A00, 0x0C00, 0x1000)
    4     word  FUN3   Parameter / sub-command ID
    6     dword FUN4   Additional data (for write commands)
   10-31        (padding to 32 bytes)

Output (OutData, 30 bytes):
  Offset  Size  Field  Description
    0     word  SGER   0x8000 = success, 0xE000 = error
    2     word  FUTR   Echoes FUN2
    4     word  FRD0   Echoes FUN3 (or result data)
    6     dword FRD1   Result data (primary return value)
   10     dword FRD2   Extended result data
   14     dword FRD3   Extended result data
```

### Verified WMAA Commands (17 total)

| FUN2   | FUN3 | Name                  | Read | Write | Result                      |
| ------ | ---- | --------------------- | ---- | ----- | --------------------------- |
| 0x0800 | 0x00 | Performance mode      | ✅   | ✅    | FRD0 = current mode (5-0xA) |
| 0x0A00 | 0x05 | MIUT (MI usage type)  | ✅   | ✅    | FRD1 = 1                    |
| 0x0A00 | 0x07 | WMIT (WMID type)      | ✅   | ✅    | FRD1 = 0                    |
| 0x0A00 | 0x08 | AILM (auto illum)     | —    | ✅    | —                           |
| 0x0A00 | 0x09 | LBLM (label mode)     | —    | ✅    | —                           |
| 0x0C00 | 0x02 | LOTS (lid open)       | ✅   | ✅    | FRD1 = 1                    |
| 0x0C00 | 0x03 | RMTS (removable)      | ✅   | ✅    | FRD1 = 0                    |
| 0x0C00 | 0x04 | OD08 (PL1 flag)       | ✅   | ✅    | —                           |
| 0x0C00 | 0x05 | OD09 (EPOF flag)      | ✅   | ✅    | —                           |
| 0x0C00 | 0x06 | PMBD (SAGV mode)      | ✅   | ✅    | —                           |
| 0x1000 | 0x01 | SOH1 (battery health) | ✅   | —     | FRD1 = 100 (%)              |
| 0x1000 | 0x02 | HBDA (brightness)     | ✅   | ✅    | —                           |
| 0x1000 | 0x03 | ADPW (adapter, alt)   | ✅   | —     | FRD1 = 0 (use 0x1000/0x06)  |
| 0x1000 | 0x04 | SCOB (scroll/button)  | ✅   | —     | —                           |
| 0x1000 | 0x05 | OFBI (off brightness) | ✅   | —     | —                           |
| 0x1000 | 0x06 | ADPW (adapter power)  | ✅   | —     | FRD1 = 100 (W)              |

### Performance Modes (FUN3 for FUN2=0x0800 write)

| Mode | Name             |
| ---- | ---------------- |
| 5    | Performance      |
| 6    | Balanced         |
| 7    | Quiet            |
| 8    | SuperQuiet       |
| 9    | UltraPerformance |
| 0x0A | Extreme          |

### WMAA Write Path — Additional Commands (from ACPI DSDT/SSDT analysis)

The WMAA write path (FUN1=0xFB00) also supports these commands discovered in ssdt24.dsl:

| FUN2   | FUN3 | Name         | Description                                                                                                 |
| ------ | ---- | ------------ | ----------------------------------------------------------------------------------------------------------- |
| 0x0800 | 2-4  | QFAN modes   | Set fan mode: 2=Balanced, 3=Perf, 4=Quiet                                                                   |
| 0x0800 | 5    | Smart Mode 1 | SMMT=0x05, SMMD=0x05                                                                                        |
| 0x0800 | 6    | Smart Mode 2 | SMMT=0x06, SMMD=0x06                                                                                        |
| 0x0800 | 7    | Smart Mode 3 | SMMD=0x07, SMMT=0x07                                                                                        |
| 0x0800 | 8    | Smart Mode 4 | SMMD=0x08, SMMT=0x08                                                                                        |
| 0x0800 | 9    | UltraPerf    | QFAN=0x09                                                                                                   |
| 0x0800 | 0x0A | Extreme      | QFAN=0x0A                                                                                                   |
| 0x1000 | 0x02 | Brightness   | FUN4: 0=off, 1=level1(0x50), 4=level2(0x5A), 5=level3(0x46), 6=level4(0x3C), 7=level5(0x32), 8=level6(0x28) |

### Runtime Verification (2026-06-29 Session 5)

All 9 WMAA read commands verified working:

| Command         | FUN2   | FUN3 | Result   | Description           |
| --------------- | ------ | ---- | -------- | --------------------- |
| Fan mode        | 0x0800 | 0x00 | FRD0=9   | UltraPerformance mode |
| MIUT (Turbo)    | 0x0A00 | 0x05 | FRD1=1   | Turbo ON              |
| WMIT (Smart)    | 0x0A00 | 0x07 | FRD1=0   | Mode 1 (WMIT=0x11)    |
| Battery health  | 0x1000 | 0x01 | FRD1=100 | 100% health           |
| Backlight level | 0x1000 | 0x02 | FRD1=0   | Backlight OFF         |
| Adapter power   | 0x1000 | 0x03 | FRD1=0   | Not charging          |
| SCOB            | 0x1000 | 0x04 | FRD1=0   | —                     |
| OFBI            | 0x1000 | 0x05 | FRD1=0   | —                     |
| ADPW raw        | 0x1000 | 0x06 | FRD1=100 | 100W adapter          |
| LOTS            | 0x0C00 | 0x02 | FRD1=1   | Lid open              |
| RMTS            | 0x0C00 | 0x03 | FRD1=0   | —                     |
| OD08            | 0x0C00 | 0x04 | FRD1=0   | —                     |
| OD09            | 0x0C00 | 0x05 | FRD1=0   | —                     |
| PMBD            | 0x0C00 | 0x06 | FRD1=0   | —                     |

**Key finding**: WMAA FUN2=0x1000 FUN3=0x02 reads the **backlight level** (HBDA/LONL).
FRD1=0 means backlight is off. This is NOT the same as KBLL (keyboard backlight),
but rather the **display brightness level** set via hotkeys.

FRD1 values for backlight:

| FRD1 | HBDA | Level        |
| ---- | ---- | ------------ |
| 0    | —    | Off (LONL=0) |
| 1    | 0x50 | Level 1      |
| 4    | 0x5A | Level 2      |
| 5    | 0x46 | Level 3      |
| 6    | 0x3C | Level 4      |
| 7    | 0x32 | Level 5      |
| 8    | 0x28 | Level 6      |

---

## 2. HQWmiCommonInterface — ✅ ALL 11 METHODS WORKING

### Overview

BIOS-level WMI interface for system configuration. All methods take `String req`,
return `String ret`.

- **WMI class**: `HQWmiCommonInterface` (ROOT\WMI)
- **Instance**: `ACPI\PNP0C14\0_0`
- **Rust implementation**: `src-tauri/src/hw/hq_wmi.rs`

### Methods

| Method                | Description           | Test Result                          |
| --------------------- | --------------------- | ------------------------------------ |
| `SetPerformanceMode`  | Set BIOS perf mode    | "Set performance mode Success!"      |
| `ChangeBootOption`    | Change boot device    | "Undefined Boot device!" (responds)  |
| `LoadDefault`         | Load BIOS defaults    | "SET BIOS LOAD DEFAULT SUCCESS!"     |
| `S5RTCWakeEnable`     | Enable S5 RTC wake    | "CONFIG S5 WAKE SUCCESS!"            |
| `EnablePXEBoot`       | Enable PXE boot       | "Enable PXE BOOT Success!"           |
| `LoadDefaultKey`      | Load default key      | (responds)                           |
| `ClearKey`            | Clear security key    | (responds)                           |
| `SetOOBTestMode`      | Set OOB test mode     | (responds)                           |
| `ClearOOBTestMode`    | Clear OOB test mode   | (responds)                           |
| `WifiCountryCode`     | Set WiFi country code | "Set country code Success!"          |
| `ShippingCountryCode` | Set shipping country  | "Set shipping country code Success!" |

**Note**: These methods are NOT called by IoTService.exe (no references in decompilation).
They are likely used by a different Xiaomi application (e.g., Mi PC Suite).

---

## 3. ACPI DSDT/SSDT Analysis — EC RAM Field Map

### EC RAM Regions

| Region    | Base Address | Size       | Description                    |
| --------- | ------------ | ---------- | ------------------------------ |
| ERAM      | 0xFE0B0300   | 256 bytes  | Main EC RAM (battery, fan, KB) |
| SMA2      | 0xFE0B0A00   | 256 bytes  | Secondary EC RAM               |
| IoTStatus | 0xFE0B0F00   | 8 bytes    | IoT status flags               |
| Sensors   | 0xFE0B0F08   | 0x78 bytes | Sensor data block              |

### ERAM Field Map (from DSDT decompilation)

| Offset | Size  | Name | Description                                |
| ------ | ----- | ---- | ------------------------------------------ |
| +0x1B  | 1     | MISC | Safe-write: AILM, LBLM flags               |
| +0x40  | 1     | —    | Touchpad config                            |
| +0x42  | 1     | —    | Touchpad config                            |
| +0x4A  | 1     | SMMD | Smart Mode data                            |
| +0x4B  | 1     | SMMT | Smart Mode type                            |
| +0x68  | 1     | QFAN | Fan control mode (2-0x0A)                  |
| +0x7B  | 1 bit | PL1F | PL1 power limit flag                       |
| +0x7B  | 1 bit | EPOF | EPOF flag                                  |
| +0x7B  | 1 bit | SAGF | SAGV mode flag                             |
| +0x7C  | 1     | OFBI | Off brightness info                        |
| +0x7C  | 1     | ISSS | —                                          |
| +0x7C  | 1     | IOTS | IoT status                                 |
| +0x7C  | 1     | WMIT | WMID type (0x11, 0x22, 0x33)               |
| +0x7D  | 1 bit | ACIN | AC adapter connected                       |
| +0x7D  | 1 bit | BTIN | Battery present                            |
| +0x7D  | 4 bit | BTST | Battery status                             |
| +0x7D  | 1 bit | FCST | Fan control status                         |
| +0x7E  | 1 bit | FNSP | Fan speed (1 bit)                          |
| +0x7E  | 3 bit | FNRV | Fan revision (3 bits)                      |
| +0x7E  | 2 bit | AOUF | —                                          |
| +0x7E  | 1 bit | CALE | —                                          |
| +0x7E  | 1 bit | CAST | —                                          |
| +0x7E  | 1 bit | IKBW | —                                          |
| +0x80  | 1 bit | ACIN | AC connected flag (alt)                    |
| +0x81  | 1     | ADPW | AC wattage (whole Watts, e.g. 100)         |
| +0x8C  | 2     | BTCT | Battery current (u16 LE, mA)               |
| +0x8E  | 2     | BTPR | Battery remaining capacity (u16 LE, mAh)   |
| +0x90  | 2     | BTVT | Battery voltage (u16 LE, mV)               |
| +0x96  | 1     | —    | Safe-write offset                          |
| +0xA7  | 1     | HBDA | Hotkey brightness data                     |
| +0xA7  | 1     | HBNT | Hotkey brightness notification             |
| +0xAB  | 1     | SOH1 | Battery state of health (0-100%)           |
| +0xAD  | 1     | UCBT | —                                          |
| +0xAD  | 1 bit | CHA1 | Charging state (determines KBLL vs DBLL)   |
| +0xAE  | 7 bit | DBLL | Display backlight level (when CHA1=0)      |
| +0xB2  | 7 bit | KBLL | **Keyboard backlight level** (when CHA1=1) |
| +0xB2  | 1 bit | KBMD | Keyboard backlight mode                    |
| +0xB3  | 1     | FEST | —                                          |
| +0xB4  | 1     | CSSD | —                                          |
| +0xB4  | 1 bit | OSDT | —                                          |

### Keyboard Backlight (KBLL) — Detailed Analysis

**Location**: ERAM offset 0xB2, bits 0-6 (7 bits). Bit 7 = KBMD (mode).

**Read path**: ACPI WMAA event (Arg1=2, FUN3=0x05) reads KBLL when CHA1=1,
or DBLL when CHA1=0. This is an **event notification** path, not a query path.
The WMI MiInterface method only supports Arg1=1 (query), so KBLL **cannot be
read via WMI WMAA query**.

**KBLL values** (from ssdt24.dsl Case 0x05):

| KBLL raw | Backlight level | EVBU[2] value |
| -------- | --------------- | ------------- |
| 1        | Off             | 0x00          |
| 2        | Low             | 0x05          |
| 4        | Medium          | 0x0A          |
| 8        | High            | 0x80          |

**Write path**: No WMAA write command exists for KBLL. The ECWT (EC Write)
method can write directly to ERAM offset 0xB2, but ECWT is only callable from
ACPI context (not from WMI). Direct ERAM write via IoTDriver IOCTL is blocked
by process name check.

**Conclusion**: Keyboard backlight level can be **read** via ACPI event
notification (not via WMI query), and can be **written** only via direct ERAM
access (blocked by IoTDriver.sys) or via ACPI ECWT (not accessible from userspace).

### Fan Control — Detailed Analysis

**QFAN** (ERAM +0x68): Fan mode byte. Read via `FUNR(0x16)`, written via
WMAA FUN2=0x0800 FUN3=2-0x0A.

**FNSP** (ERAM +0x7E bit 0): Fan speed. Read via `FUNR(0x17)`.

**Fan RPM**: Available via ACPI `RPMD` method → `H_EC.RPRC()` which returns
a 0x1A-byte buffer. The RPMD method is exposed via WMI in ssdt23.dsl.
However, H_EC (the alternative EC device) may not be present on this system
(uses `CondRefOf`), in which case RPMD returns an empty buffer.

**Fan mode values** (QFAN / FUN3 for FUN2=0x0800 write):

| QFAN | Mode             | Smart Mode (SMMT/SMMD) |
| ---- | ---------------- | ---------------------- |
| 2    | Balanced         | —                      |
| 3    | Performance      | —                      |
| 4    | Quiet            | —                      |
| 5    | Smart Mode 1     | SMMT=0x05, SMMD=0x05   |
| 6    | Smart Mode 2     | SMMT=0x06, SMMD=0x06   |
| 7    | Smart Mode 3     | SMMD=0x07, SMMT=0x07   |
| 8    | Smart Mode 4     | SMMD=0x08, SMMT=0x08   |
| 9    | UltraPerformance | —                      |
| 0x0A | Extreme          | —                      |

### Smart Mode — Detailed Analysis

**SMMT** (ERAM +0x4B): Smart Mode type.
**SMMD** (ERAM +0x4A): Smart Mode data.

Smart Mode is set via WMAA FUN2=0x0800 FUN3=5-8. When FUN3=5, it sets
SMMT=0x05 and SMMD=0x05. When FUN3=6, SMMT=0x06 and SMMD=0x06, etc.

**Access**: Smart Mode CAN be set via WMAA write (FUN2=0x0800, FUN3=5-8).
This is already partially implemented in the performance mode write path.

---

## 4. EC RAM Direct Access (IoTDriver.sys) — ❌ BLOCKED

### Driver Info

- **Name**: IoTDriver.sys
- **Type**: KMDF kernel driver
- **Version**: 25.0.0.9
- **Device**: ACPI\IOTD0000 ("Xiaomi IoT Module")
- **Status**: RUNNING
- **Device Interface GUID**: `{AB7924A1-3162-4010-B33B-837E87E25FBC}`
- **PDB**: `D:\Work\IoTDriver\x64\Release\IoTDriver.pdb`

### IOCTL Codes

| Code     | Name        | Buffer Size       |
| -------- | ----------- | ----------------- |
| 0x22E000 | ECRAM_READ  | 0x110 (272 bytes) |
| 0x22E004 | ECRAM_WRITE | 0x110 (272 bytes) |

### Security Requirements (BLOCKING)

The IoTDriver.sys performs two security checks:

1. **SeTokenIsAdmin**: The calling process must have admin privileges ✅
2. **SeLocateProcessImageName**: The process image path is checked —
   must match `IoTService.exe` in the DriverStore path ❌

**Tested**: Direct IOCTL access returns Error 5 (ACCESS_DENIED) because
the calling process is not IoTService.exe. Even running as SYSTEM via
scheduled task doesn't help — the check is on process image name, not
on privileges.

### Existing Implementation (ecram.rs)

The `ecram.rs` module implements:

- `read_ecram(offset, count)` — Read EC RAM via IOCTL
- `write_ecram(offset, data)` — Write EC RAM via IOCTL (with safe-write allowlist)
- `find_iot_device_path()` — Auto-discover device path via SetupAPI
- `try_get_ac_power_mw()` — Read AC wattage from ERAM+0x81
- `debug_ecram_hex()` — Dump ERAM and sensor block for debugging
- DSDT auto-discovery for ERAM base address (fallback: 0xFE0B0300)
- Safe-write allowlist: 0x1B, 0x40, 0x42, 0x4A, 0x4B, 0x68, 0x96, 0xAE, 0xB2
- Env var override: `MICONTROL_ENABLE_RAW_ECRAM_WRITE=1`

**Status**: Code compiles and device path discovery works, but all IOCTL calls
return ACCESS_DENIED due to process name check.

> **UPDATE 30/06/2026**: This has been **SOLVED**. A custom `IoTService.exe` replacement
> binary was built in Rust (`src-tauri/src/bin/ecram_service.rs`) that:
>
> - Is named `IoTService.exe` and placed in the DriverStore directory → passes security check
> - Provides named pipe IPC at `\\.\pipe\ecram_service` (JSON protocol)
> - Proxies ECRAM read/write IOCTLs to IoTDriver.sys
> - Successfully reads IOT_STATUS (0xFE0B0F00), IOT_SENSORS (0xFE0B0F08), ECRAM block (0xFE0B0E00)
> - **Cannot read ERAM (0xFE0B0300) or SMA2 (0xFE0B0A00)** — not in driver's allowed address ranges
>
> The `ecram.rs` module now includes a pipe client (`read_ecram_via_pipe()`,
> `is_pipe_broker_available()`) that communicates with the custom service.
>
> See [RE_ANALYSIS_REPORT.md](./RE_ANALYSIS_REPORT.md) for complete details.

### Key Discovery: WMI WMAA Bypasses Driver Process Check

The IoTDriver.sys `SeLocateProcessImageName` check only applies to direct IOCTL calls.
WMI method calls via the WDM provider (`MICommonInterface.MiInterface`) are dispatched
through a different kernel path that only checks `SeTokenIsAdmin`. This means EC access
via WMI WMAA works without needing to be IoTService.exe — only admin privileges required.

---

## 5. IoTService IPC Protocol (MCPI)

### Pipe Configuration

- **Path**: `\\.\pipe\LOCAL\IoTService_IPC_Broker`
- **Type**: Named pipe, MESSAGE mode (`PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE`)
- **Buffer**: 8192 bytes (0x2000)
- **Max Instances**: 16
- **Service**: IoTSvc (auto-start)

### Message Format (16-byte header)

```
Offset  Size  Field         Description
──────  ────  ───────────   ──────────────────────────────────────
0       4     magic         MCPI magic: 0x4950434D ("MCPI" in LE)
4       2     src_id        Source client ID (1=MiControl)
6       2     dst_id        Destination ID (2=IoTDriver)
8       2     type_lo       Low 16 bits of message type
10      2     routing       0=normal unicast, 0xFFFF=broadcast
12      2     field         Sub-type/routing, 0 for normal
14      2     payload_len   Total message size (header + payload), 16-8192
```

### Known Message Types

| Type Byte | Name             | JSON Keys                             |
| --------- | ---------------- | ------------------------------------- |
| 0x65      | GetDeviceStatus  | `DeviceStatus` (bool, out)            |
| 0x66      | SetDeviceStatus  | `DeviceStatus` (bool, in)             |
| 0x67      | GetFwVersion     | `FwVersion` (string, out)             |
| 0x68      | GetBindStatus    | `BindStatus` (bool), `UID` (u64, out) |
| 0x69      | SetBindStatus    | —                                     |
| 0x6A      | ResetDevice      | —                                     |
| 0x6B      | WriteWiFiItem    | `SSID`, `Password`, `connect`         |
| 0x6C      | EmptyWiFiItems   | —                                     |
| 0x6D      | DeleteWiFiItem   | `SSID`, `Password`                    |
| 0x6E      | GetModel         | —                                     |
| 0x6F      | ConnectWiFi      | —                                     |
| 0x70+     | SendLaptopStatus | `LaptopStatus` (int)                  |

### Testing Results

ALL messages were accepted by the pipe (no errors), but NONE received a response.
The IoTService IPC broker is a **message router**, not a message processor. It
validates the MCPI header, then looks up `dst_id` in a registered client table.
Since no client is registered for our `dst_id`, messages are silently discarded.

**Conclusion**: The MCPI pipe is for inter-client communication, not for external
hardware control. The only exception is the charging threshold command (0x1003)
which is fire-and-forget and works.

### Error Codes

| Code | Meaning                       |
| ---- | ----------------------------- |
| 0    | No error                      |
| -1   | Invalid input parameter       |
| -2   | Failed to write command data  |
| -3   | Failed to write command       |
| -4   | Failed to read command ack    |
| -5   | Failed to read command return |
| -6   | Command returned an error     |
| -7   | Failed to read command data   |
| -8   | Device is busy                |

---

## 6. WMI Architecture (from Ghidra decompilation)

The IoTService uses WMI via `Worker_WMI.cpp`. The WMI access pattern:

1. **RamIsReady**: Checks EC RAM IoTStatus at `0xFE0B0F00`. Returns error -8 if busy.
2. **WriteCommand**: Writes command to EC RAM at `0xFE0B0F01` (7 bytes).
   Format: `[0x55, param_1, 0x01, 0x01, 0x55, param_1, 0x01, 0x02]`
3. **WaitAck**: Polls EC RAM at `0xFE0B0F00` (up to 100 retries, 5ms apart).
4. **ReadResult**: Reads 8 bytes from EC RAM at `0xFE0B0F00` (up to 60 retries, 45ms apart).

### WMI Command IDs

| ID   | Function           | Description                                           |
| ---- | ------------------ | ----------------------------------------------------- |
| 0x01 | GetBindStatus      | Get device bind status + UID                          |
| 0x02 | SetBindStatus      | Set bind status                                       |
| 0x03 | ResetDevice        | Reset device                                          |
| 0x05 | EmptyWiFiItems     | Clear all WiFi items                                  |
| 0x06 | DeleteWiFiItem     | Delete specific WiFi item                             |
| 0x0A | GetFwVersion       | Get firmware version                                  |
| 0x10 | ReportLaptopStatus | Report laptop status (WIN_READY, SHUTING, SUSPENDING) |

---

## 7. Other WMI Classes

### MSAcpi_ThermalZoneTemperature — ✅ Working

- **Instance**: `ACPI\ThermalZone\TZ00_0`
- **CurrentTemperature**: 3010 (tenths Kelvin = ~27.85°C)
- **CriticalTripPoint**: 3782 (tenths Kelvin = ~104.85°C)
- **Implementation**: `src-tauri/src/hw/thermal.rs`

### EsifDeviceInformation (Intel DPTF) — ✅ Working

- **Instances**: \_0 through \_14
- **Fields**: Temperature (°C), Power (deciwatts), TripPoints

| Instance | Temperature | Power             |
| -------- | ----------- | ----------------- |
| \_0      | 65°C        | 84W (CPU package) |
| \_1      | 65°C        | 51W               |
| \_10     | 46°C        | 28W (GPU)         |

- **Implementation**: `src-tauri/src/hw/fan.rs` (get_esif_readings)

### BatteryStatus — ✅ Working

- **Instance**: `ACPI\PNP0C0A\1_0`
- **Fields**: ChargeRate (mW), DischargeRate (mW), Voltage (mV), PowerOnline, Charging, Discharging
- **Implementation**: `src-tauri/src/hw/battery.rs`

### BatteryStaticData — ✅ Working

- **Fields**: DesignedCapacity=68224, DeviceName="BX70", SerialNumber="GYBX706418002793GMD1R100", Chemistry
- **Implementation**: `src-tauri/src/hw/battery.rs`

### BatteryFullChargedCapacity — ✅ Working

- **FullChargedCapacity**: 69230 (mWh)

### BatteryCycleCount — ✅ Working

- **CycleCount**: 0

### DBICommonInterface — ✅ GetODMSSID WORKING

- **Instance**: `ACPI\PNP0C14\0_0`
- **Methods**: `S5RTCWakeEnable(req)`, `GetODMSSID()`
- **GetODMSSID**: Successfully called, returned `"0x00013100"` ✅
- **S5RTCWakeEnable**: Available but not yet tested

### WMIEvent — ❌ Event Registration Not Supported

The ACPI \_WDG buffer (ssdt24.dsl line 155) contains 6 entries:

| Entry | GUID                                 | ObjID   | Flags | Type              |
| ----- | ------------------------------------ | ------- | ----- | ----------------- |
| 0     | 133EC946-9BEE-6242-8488-563BCA757FEF | (space) | 0x08  | Method            |
| 1     | 45E278FA-0F2C-A14C-91CF-15F34E474850 | !       | 0x08  | Method            |
| 2     | 0AAFCE1D-634D-BB44-BD0C-0D6281BFDDC5 | "       | 0x08  | Method            |
| 3     | 263C9E3F-77B0-864F-91F5-37FF64D8C7ED | #       | 0x08  | Method            |
| 4     | 48FB0BB6-5B3E-E449-A0E9-8CFFE1B3434B | AA      | 0x02  | **Event**         |
| 5     | 21129005-66D5-D111-B2F0-00A0C9062910 | AB      | 0x00  | Standard WMI GUID |

**Entry 4** (ObjID=AA, flags=0x02) is the WMI event notification entry.
When ACPI fires `Notify(WMID, 0x20)`, the `_WED` method (ssdt24.dsl line 1399)
returns the EVBU buffer containing event data.

The EV20 event handler (ssdt24.dsl line 1030) processes events:

- Case 0x05: Reads KBLL (keyboard backlight) — **the only way to read KBLL**
- Case 0x07: Reads FNLK via FUNR(0x17) — fan speed level
- Case 0x09: Reads CPLK via FUNR(0x19) — CPU performance level
- Case 0x16: Reads QFAN via FUNR(0x16) — fan mode
- Case 0x21: Reads MIUT — MI usage type
- Case 0x22: Reads WMIT — WMID type
- Case 0x2A: Reads ISSS via ECRD
- Case 0x2B: Reads IOTS via ECRD

**Testing result**: `Register-WmiEvent -Query "SELECT * FROM WMIEvent"` returns
"Not supported". The Windows ACPI WMI provider does not support extrinsic event
delivery for this WMI class. `Register-CimIndicationEvent` also produced no events.

**Conclusion**: WMI event registration for KBLL reading is **not supported** by
the Windows ACPI WMI provider on this system. The event mechanism works
internally (IoTService.exe has a `CWmiEventSink` class that receives events),
but external processes cannot register for these events.

### HID_EVENT20-23 — ✅ WORKING (Session 5 Breakthrough)

Four HID event classes exist in root\WMI:

- HID_EVENT20, HID_EVENT21, HID_EVENT22, HID_EVENT23

These carry keyboard/backlight events from the ACPI HID subsystem.
**Registration via `Register-WmiEvent` and `Register-CimIndicationEvent` both succeed**
and deliver real-time events when hotkeys are pressed.

**Rust implementation**: `src-tauri/src/hw/hotkeys/mod.rs` — `start_wmi_hid_listener()`
subscribes to all four classes on separate threads with auto-reconnect.

#### HID_EVENT20 Event Format

Each event delivers a 32-byte `EventDetail` buffer:

```
EventDetail[0] = 0x01 (report ID / header, always 1)
EventDetail[1] = event type code (distinguishes which key)
EventDetail[2] = value (new state / level)
EventDetail[3-31] = 0x00 (padding)
```

#### Complete Hotkey Event Map (HID_EVENT20)

| detail[1] | Key         | detail[2]           | Description                                      |
| --------- | ----------- | ------------------- | ------------------------------------------------ |
| 0x01      | Fn+F8       | —                   | Project / display mode (Win+P) — **not working** |
| 0x05      | Fn+F10      | 0x00/0x05/0x0A/0x80 | Keyboard backlight (Off/Low/Med/High)            |
| 0x07      | Fn+Esc      | 0x00/0x01           | Fn Lock toggle (FNLK) — event-driven only        |
| 0x1B      | Fn+F9       | 0x00                | Windows Settings (Win+I) — **not working**       |
| 0x21      | Fn+F4       | 0x00/0x01           | Mic mute (0=muted, 1=active)                     |
| 0x23      | Fn+F7       | —                   | AI key press (configurable)                      |
| 0x24      | Fn+F7       | —                   | AI key release                                   |
| 0x25      | Xiaomi logo | —                   | Xiaomi logo key press (configurable)             |
| 0x26      | Xiaomi logo | —                   | Xiaomi logo key release                          |
| 0x27      | —           | 0x01                | Unknown event                                    |
| 0x28      | —           | 0x01                | Unknown event                                    |
| 0x29      | —           | 0x01                | Unknown event                                    |
| 0x2A      | —           | ISSS value          | Screenshot? Reads ISSS from EC, clears it        |
| 0x2B      | —           | IOTS value          | Unknown — reads IOTS from EC, clears it          |

#### KBLL (Keyboard Backlight) via HID_EVENT20 — ✅ WORKING

When Fn+F10 is pressed, EV20 Case 0x05 fires and reads KBLL from EC RAM.
The event delivers the new backlight level in `detail[2]`:

| detail[2] | KBLL raw | Level  |
| --------- | -------- | ------ |
| 0x00      | 1        | Off    |
| 0x05      | 2        | Low    |
| 0x0A      | 4        | Medium |
| 0x80      | 8        | High   |

**Runtime verified** (2026-06-29): User pressed Fn+F10 repeatedly, captured
all 4 levels in real-time via `Register-WmiEvent -Query "SELECT * FROM HID_EVENT20"`.

**Limitation**: This is an **event-driven** read — KBLL is only delivered when
the user presses the backlight hotkey. There is no way to **query** the current
KBLL value on demand via WMI. A polling read would require direct ERAM access
(blocked by IoTDriver.sys).

**Write**: KBLL cannot be written via WMAA (no write command exists in the
ACPI WMAA write path). Writing requires direct ERAM access to offset 0xB2.

---

## 8. Registry Keys

| Path                                               | Keys                                                            |
| -------------------------------------------------- | --------------------------------------------------------------- |
| `HKLM\SOFTWARE\MI\DisplaySettings`                 | AiAdaptiveBrightness, AiBrightnessMin/Max/Sensitivity/Smoothing |
| `HKLM\SOFTWARE\MI\Touchpad`                        | HapticsEnabled, EdgeSlide, TrackpadRepress                      |
| `HKLM\SOFTWARE\MI\PerformanceMode`                 | LastLongBattery                                                 |
| `HKLM\SOFTWARE\MI\IoTDriver`                       | ChargingThreshold, FwVersion, Uid, Enable, SSID, WiFiStatus     |
| `HKLM\SOFTWARE\MI\MiDeviceService`                 | ProductModel = "TM2424"                                         |
| `HKLM\SYSTEM\CurrentControlSet\Services\IoTDriver` | ImagePath, Start=3, Type=1                                      |

---

## 9. ACPI Devices

| Device ID             | Description                       |
| --------------------- | --------------------------------- |
| ACPI\PNP0C14\MIFS     | WMAA (MICommonInterface)          |
| ACPI\PNP0C14\0        | HQWmiCommonInterface              |
| ACPI\IOTD0000         | Xiaomi IoT Module (IoTDriver)     |
| ACPI\PNP0C02\IOTRAPS  | Motherboard resources (IoTDriver) |
| ACPI\MSFT0001         | Standard PS/2 Keyboard            |
| ACPI\PNP0C0A\1        | Battery                           |
| ACPI\ThermalZone\TZ00 | ACPI Thermal Zone                 |

---

## 10. IoTService.exe Binary Analysis

### Source Files (from PDB paths)

- `D:\Work\IoTDriver\IoTService\IoT.cpp` — Main IoT device logic
- `D:\Work\IoTDriver\IoTService\IoTDriver.cpp` — Driver communication (EC RAM)
- `D:\Work\IoTDriver\IoTService\RamIO.cpp` — EC RAM read/write via IOCTL
- `D:\Work\IoTDriver\IoTService\Worker_IPC.cpp` — IPC client
- `D:\Work\IoTDriver\IoTService\Worker_IPCBroker.cpp` — IPC broker (pipe server)
- `D:\Work\IoTDriver\IoTService\Worker_WMI.cpp` — WMI operations
- `D:\Work\IoTDriver\IoTService\OnServiceEvent.cpp` — Event handler
- `D:\Work\IoTDriver\IoTService\Util_Driver.cpp` — Driver utilities
- `D:\Work\IoTDriver\IoTService\Util_Dump.cpp` — Crash dump utilities
- `D:\Work\IoTDriver\IoTService\RunAsHelper.cpp` — Elevation helper
- `D:\Work\IoTDriver\IoTService\worker.cpp` — Main worker thread

### String Analysis Results (Session 5)

Key strings found in IoTService.exe:

**EC RAM Access**:

- `RamIO.cpp` — EC RAM I/O source file
- `RamDevice::Read() invalid parameter.` / `RamDevice::Read() IoCtrl error: %lu.`
- `RamDevice::Write() invalid parameter.` / `RamDevice::Write() IoCtrl error: %lu.`
- `RamIsReady: RAM status is busy (0x%02X)` — EC RAM status check
- `DeviceIoControl failed: %lu.` — IOCTL call failure

**IPC Protocol**:

- `Received IPC message: SrcId=0x%04X, DstId=0x%04X, Type=%d`
- `Invalid IPC message signature:` / `Invalid IPC message size:`
- `IPC Broker started on pipe: \\.\pipe\LOCAL\IoTService_IPC_Broker`
- `IPC Client connected successfully.` / `IPC Client disconnected.`

**WMI Events**:

- `.?AVCWmiEventSink@@` — WMI event sink class (IoTService registers for WMI events!)
- `Worker_WMI::getDeviceStatus()` / `Worker_WMI::setDeviceStatus()`
- `Invalid JSON data for SetDeviceStatus` / `Invalid JSON for SendLaptopStatus`

**EC Command Protocol**:

- `ReadCmdAck: command ack is last cmd: %d.`
- `ReadCmdAck: command ack is next cmd: %02X %02X %02X %02X, retry: %d.`
- `ReadCmdAck: expected ack %02X %02X %02X %02X, but got %02X %02X %02X %02X`
- `ReadCmdAck: command %02X read ack timeout.`
- `ReadCmdRet: expected cmd ret %02X %02X %02X %02X, but got %02X %02X %02X %02X`
- `ReadCmdRet: command %02X read ret timeout.`

**IPC Broker Pipe Connection**:

- Successfully connected to `\\.\pipe\LOCAL\IoTService_IPC_Broker` ✅
- Pipe is MESSAGE mode, bidirectional
- No initial data sent by server (client must send first)
- Read blocks waiting for client message

**Service Log Analysis**:

- Log file: `C:\ProgramData\MI\IoTService\service.log` (2.5MB)
- Shows active EC RAM reading: commands 1-100, then timeout on cmd 10
- `IoTDriver::ReportLaptopStatus(16) fail, error code: -4` (read ack timeout)
- IPC Broker starts on service startup, IPC Client connects immediately

**XiaomiPCManager**:

- Found at `C:\Users\mafsc\AppData\Local\MI\XiaomiPCManager`
- Uses EBWebView (WebView2-based UI)
- Not currently running (no process found)
- This is the Mi Smart Hub equivalent that connects to IoTService IPC

No references to keyboard backlight (KBLL), fan RPM, or Smart Mode were found in
IoTService.exe strings. The service handles: device status, WiFi management,
firmware version, bind status, laptop status notifications, and charging threshold.
However, the `CWmiEventSink` class confirms IoTService receives WMI events
(including KBLL via EV20 case 0x05) but does not expose them via IPC.

---

## 11. Hardware Capabilities Summary

### ✅ Fully Working

| Capability                     | Access Method                     | Implementation   |
| ------------------------------ | --------------------------------- | ---------------- |
| Performance mode (read/write)  | WMI WMAA `0x0800`                 | `wmi_ec.rs`      |
| Battery health (SOH1)          | WMI WMAA `0x1000/0x01`            | `wmi_ec.rs`      |
| Adapter power (ADPW)           | WMI WMAA `0x1000/0x06`            | `wmi_ec.rs`      |
| Battery charge rate            | WMI BatteryStatus                 | `battery.rs`     |
| Battery voltage                | WMI BatteryStatus                 | `battery.rs`     |
| Battery serial number          | WMI BatteryStaticData             | `battery.rs`     |
| Battery chemistry              | WMI BatteryStaticData             | `battery.rs`     |
| Battery cycle count            | WMI BatteryCycleCount             | `battery.rs`     |
| Battery designed capacity      | WMI BatteryStaticData             | `battery.rs`     |
| Battery full charge capacity   | WMI BatteryFullChargedCapacity    | `battery.rs`     |
| CPU temperature                | WMI EsifDeviceInformation         | `fan.rs`         |
| GPU temperature                | WMI EsifDeviceInformation         | `fan.rs`         |
| CPU package power (TDP)        | WMI EsifDeviceInformation         | `fan.rs`         |
| Thermal zone temperature       | WMI MSAcpi_ThermalZoneTemperature | `thermal.rs`     |
| Critical trip point            | WMI MSAcpi_ThermalZoneTemperature | `thermal.rs`     |
| Fan control (auto/fixed)       | Intel IGCL                        | `fan.rs`         |
| Charging threshold             | IoTService IPC + Registry         | `charging.rs`    |
| Brightness                     | WMI + Windows API                 | `display.rs`     |
| Touchpad settings              | Registry                          | `touchpad.rs`    |
| Audio                          | Windows Core Audio                | `audio.rs`       |
| Screen cast                    | Windows Miracast                  | `screen_cast.rs` |
| Hotkeys (Fn+F4/F7/F8/F9/F10)   | HID_EVENT20 WMI + keyboard hook   | `hotkeys/`       |
| Keyboard backlight read (KBLL) | HID_EVENT20 WMI event             | `hotkeys/mod.rs` |
| MI usage type (MIUT)           | WMI WMAA `0x0A00/0x05`            | `wmi_ec.rs`      |
| WMID type (WMIT)               | WMI WMAA `0x0A00/0x07`            | `wmi_ec.rs`      |
| Lid open type (LOTS)           | WMI WMAA `0x0C00/0x02`            | `wmi_ec.rs`      |
| Removable type (RMTS)          | WMI WMAA `0x0C00/0x03`            | `wmi_ec.rs`      |
| PL1 flag                       | WMI WMAA `0x0C00/0x04`            | `wmi_ec.rs`      |
| EPOF flag                      | WMI WMAA `0x0C00/0x05`            | `wmi_ec.rs`      |
| SAGV mode                      | WMI WMAA `0x0C00/0x06`            | `wmi_ec.rs`      |
| Hotkey brightness (HBDA)       | WMI WMAA `0x1000/0x02`            | `wmi_ec.rs`      |
| Auto illumination (AILM)       | WMI WMAA `0x0A00/0x08`            | `wmi_ec.rs`      |
| Label mode (LBLM)              | WMI WMAA `0x0A00/0x09`            | `wmi_ec.rs`      |
| BIOS boot option               | HQWmiCommonInterface              | `hq_wmi.rs`      |
| BIOS load default              | HQWmiCommonInterface              | `hq_wmi.rs`      |
| S5 RTC wake                    | HQWmiCommonInterface              | `hq_wmi.rs`      |
| PXE boot                       | HQWmiCommonInterface              | `hq_wmi.rs`      |
| WiFi country code              | HQWmiCommonInterface              | `hq_wmi.rs`      |
| Shipping country code          | HQWmiCommonInterface              | `hq_wmi.rs`      |

### ⚠️ Partially Working / Needs Integration

| Capability             | Access Method            | Status       | Notes                                    |
| ---------------------- | ------------------------ | ------------ | ---------------------------------------- |
| Fan mode (QFAN)        | WMAA `0x0800` write      | ⚠️ Available | Not yet implemented as separate command  |
| Smart Mode             | WMAA `0x0800` FUN3=5-8   | ⚠️ Available | Part of performance mode write path      |
| Brightness levels      | WMAA `0x1000/0x02` write | ⚠️ Available | FUN4 values 0-8 map to brightness levels |
| Display/Eye protection | ICC profiles             | ⚠️ Available | Needs integration                        |
| WiFi management        | IoTService IPC           | ⚠️ Partial   | Fire-and-forget only                     |

### ❌ Not Accessible

| Capability                          | Reason                                                                                                                                                                                                                                                        | Potential Solution                                                                                          |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| **Keyboard backlight read (KBLL)**  | ✅ **SOLVED via HID_EVENT20** — event-driven read works when Fn+F10 is pressed. Cannot poll on demand.                                                                                                                                                        | HID_EVENT20 WMI subscription implemented in `hotkeys/mod.rs`. For on-demand read, needs direct ERAM access. |
| **Keyboard backlight write (KBLL)** | No WMAA write command exists. ECWT can write KBLL but is ACPI-internal only.                                                                                                                                                                                  | Needs direct ERAM access via custom kernel driver or EC I/O port driver (WinRing0/inpoutx64).               |
| **Fan RPM**                         | Via RPMD → H_EC.RPRC(). H_EC may not be present. Win32_Fan returns 0.                                                                                                                                                                                         | Try WMI WQEN for RPMD, or direct ERAM read. EV20 case 0x07 reads FNLK via FUNR(0x17) but only via event.    |
| **EC RAM direct IOCTL**             | IoTDriver.sys blocks ALL IOCTLs to IOTD0000 (not just EC RAM). Tested IOCTL_ACPI_EVAL_METHOD (0x224004) and IOCTL_ACPI_EVAL_METHOD_EX (0x224008) — ALL return ACCESS_DENIED. Proxy process approach also failed (driver checks full path, not just filename). | Write custom kernel driver, or use existing EC I/O port driver                                              |
| **Battery current (BTCT)**          | ERAM offset 0x8C. Needs direct ERAM access.                                                                                                                                                                                                                   | Same as KBLL — needs ERAM access                                                                            |
| **MCPI pipe queries**               | Broker only — no registered clients respond                                                                                                                                                                                                                   | IoTService receives WMI events via CWmiEventSink but doesn't forward them via IPC                           |
| **WMIEvent extrinsic events**       | `Register-WmiEvent` for WMIEvent returns "Not supported"                                                                                                                                                                                                      | Use HID_EVENT20-23 instead (these work). WMIEvent class is a different notification path.                   |

---

## 12. Architecture Decision

### Primary Hardware Access Path: WMI WMAA

The WMAA method provides direct EC access via ACPI WMI without going through
IoTDriver.sys, bypassing the process name check. Only requires admin privileges.

1. **Performance mode**: WMI WMAA `0x0800` (primary) + HQWmiCommonInterface (fallback)
2. **Battery/adapter sensors**: WMI WMAA `0x1000` (primary)
3. **Thermal monitoring**: EsifDeviceInformation + MSAcpi_ThermalZoneTemperature
4. **Fan control**: Intel IGCL (auto/fixed modes)
5. **Charging threshold**: IoTService IPC pipe 0x1003 + registry fallback
6. **Battery**: Win32_Battery + WMAA SOH1 (health %)
7. **Brightness**: WMI + Windows API
8. **Touchpad**: Registry
9. **BIOS config**: HQWmiCommonInterface (11 methods)

### Blocked Features

Features that require direct EC RAM access (blocked by IoTDriver.sys):

- **Keyboard backlight write (KBLL)** — read is solved via HID_EVENT20 events, but write still needs ERAM
- **Fan RPM** (via RPMD/H_EC.RPRC)
- **Battery current** (BTCT at ERAM+0x8C)
- **Battery capacity raw** (BTPR at ERAM+0x8E)
- **Battery voltage raw** (BTVT at ERAM+0x90)

### Potential Solutions for Blocked Features

1. **Custom kernel driver**: Write a minimal KMDF driver that reads ERAM via
   physical memory mapping (MmMapIoSpace). This is the most reliable solution
   but requires driver signing.

2. **EC I/O port access**: The EC is accessible via I/O ports 0x66 (command)
   and 0x62 (data). A driver like WinRing0 or inpoutx64 could provide this
   access. However, these drivers are often flagged by antivirus software.
   No existing I/O port driver was found on this system.

3. **ACPI WMI event registration (WMIEvent class)**: ❌ TESTED AND FAILED.
   `Register-WmiEvent` for WMIEvent returns "Not supported".
   `Register-CimIndicationEvent` also produced no events.
   The Windows ACPI WMI provider does not support extrinsic event delivery
   for the WMIEvent class.

4. **HID_EVENT20-23 WMI events**: ✅ **WORKING**.
   `Register-WmiEvent` for HID_EVENT20 succeeds and delivers real-time events.
   KBLL (keyboard backlight) is delivered via event when Fn+F10 is pressed.
   This is an event-driven read, not a poll-on-demand query.
   Implemented in Rust: `hotkeys/mod.rs` → `start_wmi_hid_listener()`.

5. **Proxy process**: ❌ TESTED AND FAILED.
   Copied `ecram_shim.exe` to the exact DriverStore path as `IoTService.exe`
   (took ownership via `takeown` + `icacls`). The driver still returns
   ACCESS_DENIED. The driver's `SeLocateProcessImageName` check is NOT just
   a filename comparison — it likely checks the full image path or uses a
   different mechanism (possibly PID-based or ObRegisterCallbacks).

6. **IoTService IPC Broker**: ✅ PIPE CONNECTS but messages are not processed.
   The IPC broker is a message router — it validates MCPI header and routes
   to registered clients. No client responds to hardware queries.
   IoTService.exe receives WMI events via CWmiEventSink but does not forward
   them via IPC. The XiaomiPCManager app (Mi Smart Hub) is the intended IPC
   client but was not running during testing.

7. **XiaomiPCManager reverse engineering**: The Mi Smart Hub app
   (`C:\Users\mafsc\AppData\Local\MI\XiaomiPCManager`) uses WebView2 and
   connects to IoTService IPC. Reverse engineering its IPC client could
   reveal the protocol for requesting KBLL readings from IoTService.

8. **IoTDriver.sys blocks ALL IOCTLs**: ❌ TESTED AND CONFIRMED.
   Tested IOCTL_ACPI_EVAL_METHOD (0x224004) and IOCTL_ACPI_EVAL_METHOD_EX
   (0x224008) on the IOTD0000 device — ALL return ACCESS_DENIED (error 5).
   The driver intercepts ALL device I/O, not just EC RAM read/write IOCTLs.
   This means ACPI eval methods (FUNR, WMAA(2), EV20, \_WED) cannot be called
   directly on this device.

---

## 13. Source Files Structure

```
src-tauri/src/hw/
├── wmi_ec.rs       # WMAA commands (17 methods via MICommonInterface)
├── hq_wmi.rs       # HQWmiCommonInterface (11 BIOS methods)
├── thermal.rs      # MSAcpi_ThermalZoneTemperature
├── battery.rs      # BatteryStatus, BatteryStaticData, BatteryFullChargedCapacity
├── fan.rs          # EsifDeviceInformation, Win32_Fan, Intel IGCL
├── ecram.rs        # EC RAM access via IoTDriver IOCTL (blocked)
├── iotservice.rs   # IoTService IPC pipe communication (MCPI)
├── charging.rs     # Charging threshold via IPC + registry
├── display.rs      # Brightness, display settings
├── touchpad.rs     # Touchpad settings via registry
├── audio.rs        # Audio via Windows Core Audio
├── screen_cast.rs  # Miracast screen casting
├── wmi_cache.rs    # WMI connection cache
├── errors.rs       # Hardware error types
└── hotkeys/
    └── mod.rs      # Keyboard hook + HID_EVENT20-23 WMI listener
```

---

## 14. Fixes Applied

### iotservice.rs (Session 2-3)

1. Added MCPI magic (0x4950434D) at offset 0 of IPC header
2. Extended header from 12 to 16 bytes with type_lo, routing, field, payload_len
3. Set pipe to MESSAGE mode using SetNamedPipeHandleState
4. Use CreateFileW instead of std::fs::OpenOptions
5. Added MCPI magic validation on response headers

### charging.rs (Session 2-3)

1. Updated IotIpcMsg struct to 16-byte MCPI format
2. Set pipe to MESSAGE mode
3. Use CreateFileW for pipe handle
4. Send payload separately after 16-byte header

### wmi_ec.rs (Session 3)

1. Full Rust implementation of WMI MiInterface (WMAA) access
2. VARIANT construction with SAFEARRAY of VT_UI1
3. 17 public API functions for all WMAA commands
4. Runtime verified — all commands work

### hq_wmi.rs (Session 4)

1. Full Rust implementation of HQWmiCommonInterface
2. 11 public API functions for all BIOS methods
3. Runtime verified — all methods respond with success

### thermal.rs (Session 4)

1. Reads temperature from ACPI thermal zones via MSAcpi_ThermalZoneTemperature
2. Converts tenths-of-Kelvin to Celsius
3. Runtime verified — TZ00: ~27.85°C

### battery.rs (Session 4)

1. Added serial_number and chemistry fields from BatteryStaticData
2. Runtime verified

### hotkeys/mod.rs (Session 5)

1. WMI HID_EVENT20-23 subscription via `start_wmi_hid_listener()`
2. Complete hotkey event map: Fn+F4 (mic), Fn+F7 (AI), Fn+F8 (project),
   Fn+F9 (settings), Fn+F10 (keyboard backlight), Xiaomi logo key
3. KBLL read via HID_EVENT20 Case 0x05 (event-driven, not pollable)
4. Fn+F8 → Win+P (project/display mode)
5. Fn+F9 → Win+I (Windows Settings)
6. Auto-reconnect with exponential backoff for dropped WMI connections

---

## 15. Complete Hotkey Map

### Fn Key Combinations

| Key         | Event Code | Action                | Status         | Implementation                                     |
| ----------- | ---------- | --------------------- | -------------- | -------------------------------------------------- |
| Fn+F1       | —          | Mute (audio)          | ✅ OS-level    | HID consumer control (0x00E2)                      |
| Fn+F2       | —          | Volume down           | ✅ OS-level    | HID consumer control                               |
| Fn+F3       | —          | Volume up             | ✅ OS-level    | HID consumer control                               |
| Fn+F4       | 0x21       | Mic mute toggle       | ✅ Working     | `hotkeys/mod.rs` → OSD + system mute               |
| Fn+F5       | —          | Brightness down       | ✅ OS-level    | HID consumer control                               |
| Fn+F6       | —          | Brightness up         | ✅ OS-level    | HID consumer control                               |
| Fn+F7       | 0x23/0x24  | AI key (configurable) | ✅ Working     | `hotkeys/mod.rs` → configurable action             |
| Fn+F8       | 0x01       | Project / cast screen | ✅ Fixed       | `hotkeys/mod.rs` → `ShellExecuteW("ms-project:")`  |
| Fn+F9       | 0x1B       | Windows Settings      | ✅ Fixed       | `hotkeys/mod.rs` → `ShellExecuteW("ms-settings:")` |
| Fn+F10      | 0x05       | Keyboard backlight    | ✅ Working     | `hotkeys/mod.rs` → OSD + KBLL event                |
| Fn+F11      | —          | Unknown               | ❌ Not mapped  | No HID_EVENT20 event observed                      |
| PrintScreen | —          | Screenshot            | ❌ Not working | No event generated; key may be Fn-locked           |
| Fn+Esc      | 0x07       | Fn Lock toggle        | ✅ EC firmware | Event-driven read via HID_EVENT20                  |

### Special Keys

| Key | Event Code | Action | Status |
| ------------------ | ---------- | ------------ | ---------- | ----------------------------------------------------------------------------------------------------------------------- |
| Xiaomi logo key | 0x25/0x26 | Configurable | ✅ Working |
| Copilot key (0xC3) | — | Configurable | ⚠️ Partial | `RegisterHotKey` with `MOD_NOREPEAT` + alt `Win+Shift+F23` path; may still be intercepted by Shell on some Win11 builds |

### HID_EVENT20 Event Codes (All Observed)

| detail[1] | detail[2]           | Key                 | Notes                              |
| --------- | ------------------- | ------------------- | ---------------------------------- |
| 0x01      | —                   | Fn+F8               | Project (Win+P) — **not working**  |
| 0x05      | 0x00/0x05/0x0A/0x80 | Fn+F10              | KBLL: Off/Low/Med/High             |
| 0x07      | 0x00/0x01           | Fn+Esc              | Fn Lock toggle (FNLK)              |
| 0x1B      | 0x00                | Fn+F9               | Settings (Win+I) — **not working** |
| 0x21      | 0x00/0x01           | Fn+F4               | Mic mute (0=muted, 1=active)       |
| 0x23      | —                   | Fn+F7 press         | AI key                             |
| 0x24      | —                   | Fn+F7 release       | AI key                             |
| 0x25      | —                   | Xiaomi logo press   | Configurable                       |
| 0x26      | —                   | Xiaomi logo release | Configurable                       |
| 0x27      | 0x01                | Unknown             | Not mapped                         |
| 0x28      | 0x01                | Unknown             | Not mapped                         |
| 0x29      | 0x01                | Unknown             | Not mapped                         |
| 0x2A      | ISSS                | Unknown             | Reads/clears EC ISSS register      |
| 0x2B      | IOTS                | Unknown             | Reads/clears EC IOTS register      |

---

## 16. What Remains Unmapped / Blocked

### ❌ Still Blocked (Requires Direct EC RAM Access)

| Feature                                 | EC Offset | Why Blocked                                                                         | Workaround                                              |
| --------------------------------------- | --------- | ----------------------------------------------------------------------------------- | ------------------------------------------------------- |
| **KBLL write** (keyboard backlight set) | 0xB2      | No WMAA write command. ECWT is ACPI-internal only. IoTDriver.sys blocks all IOCTLs. | None — needs custom kernel driver or EC I/O port driver |
| **Fan RPM read**                        | —         | Via RPMD → H_EC.RPRC(). H_EC may not exist (CondRefOf). Win32_Fan returns 0.        | None — needs ERAM access or H_EC device                 |
| **Battery current (BTCT)**              | 0x8C      | Needs direct ERAM read                                                              | None — needs ERAM access                                |
| **Battery raw capacity (BTPR)**         | 0x8E      | Needs direct ERAM read                                                              | BatteryStatus WMI gives approximate values              |
| **Battery raw voltage (BTVT)**          | 0x90      | Needs direct ERAM read                                                              | BatteryStatus WMI gives approximate values              |
| **Smart Mode read (SMMT/SMMD)**         | 0x4A/0x4B | Can write via WMAA but cannot read back                                             | Write works, read-back not possible                     |
| **QFAN read**                           | 0x68      | FUNR(0x16) only works from ACPI event context                                       | Write works via WMAA, read not possible                 |
| **FNLK read**                           | —         | FUNR(0x17) only works from ACPI event context                                       | Event-driven read via HID_EVENT20 Case 0x07             |

### ⚠️ Event-Driven Only (No On-Demand Query)

| Feature       | How It Works                                   | Limitation                                 |
| ------------- | ---------------------------------------------- | ------------------------------------------ |
| **KBLL read** | HID_EVENT20 delivers level when Fn+F10 pressed | Cannot poll current level on demand        |
| **FNLK read** | HID_EVENT20 Case 0x07 delivers Fn Lock state   | Only delivered when Fn+Esc pressed         |
| **QFAN read** | HID_EVENT20 Case 0x16 delivers fan mode        | Only delivered when fan mode key pressed   |
| **WMIT read** | HID_EVENT20 Case 0x22 delivers smart mode      | Only delivered when smart mode key pressed |

### 🔍 Unknown HID_EVENT20 Events (Not Yet Mapped)

| detail[1] | detail[2]  | Observed During | Possible Function                                   |
| --------- | ---------- | --------------- | --------------------------------------------------- |
| 0x27      | 0x01       | Testing         | Unknown — possibly refresh/screenshot               |
| 0x28      | 0x01       | Testing         | Unknown — possibly display toggle                   |
| 0x29      | 0x01       | Testing         | Unknown — possibly airplane mode                    |
| 0x2A      | ISSS value | DSL analysis    | Reads ISSS from EC, clears it — possibly screenshot |
| 0x2B      | IOTS value | DSL analysis    | Reads IOTS from EC, clears it — unknown             |

### ❌ Hotkeys Not Working (Event Fires but Action Fails)

| Key             | Event                   | Problem                                                                             | Possible Cause                                                                |
| --------------- | ----------------------- | ----------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| **Copilot key** | VK 0xC3 / Win+Shift+F23 | `RegisterHotKey` + LL hook fire but Windows Shell may intercept first on Win11 24H2 | Shell-level interception; `MOD_NOREPEAT` + alt F23 path added as mitigation   |
| **Fn+F11**      | —                       | No event observed                                                                   | Key may not generate HID_EVENT20; possibly Fn-locked or not wired in firmware |
| **PrintScreen** | —                       | No event observed                                                                   | Key may require Fn lock to activate; no HID_EVENT20 event generated           |

### ✅ Previously Broken, Now Fixed

| Key                  | Problem                                                                       | Fix                                                                           |
| -------------------- | ----------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| **Fn+F8** (Project)  | `send_win_key_combo(Win+P)` — synthetic Win+P ignored by Explorer due to UIPI | Replaced with `ShellExecuteW("ms-project:")` — opens Cast/Project UI directly |
| **Fn+F9** (Settings) | `send_win_key_combo(Win+I)` — synthetic Win+I ignored by Explorer due to UIPI | Replaced with `ShellExecuteW("ms-settings:")` — opens Settings app directly   |

### 📋 Summary: Access Coverage

| Category            | Total Features | Working | Event-Only | Blocked/Not Working          |
| ------------------- | -------------- | ------- | ---------- | ---------------------------- |
| WMAA read commands  | 14             | 14      | 0          | 0                            |
| WMAA write commands | 13             | 13      | 0          | 0                            |
| HQWmi methods       | 11             | 11      | 0          | 0                            |
| Hotkeys (Fn+F1–F12) | 12             | 7       | 1 (FNLK)   | 4 (F8, F9, F11, PrintScreen) |
| Copilot key         | 1              | 0       | 0          | 1                            |
| EC RAM fields       | ~30            | 0       | 4          | ~26                          |
| **Total**           | **~81**        | **45**  | **5**      | **31**                       |

**Bottom line**: Fn+F1–F6 are OS-level HID consumer controls (mute, volume, brightness).
Fn+F4 (mic), F7 (AI key), F10 (keyboard light) work. Fn+F8/F9 fire WMI events
but the synthetic Win+P/Win+I doesn't reach Explorer. Copilot key is intercepted
by Windows Shell before our hook. Fn+F11 and PrintScreen don't generate events.
All WMAA read/write commands work. EC RAM access remains blocked by IoTDriver.sys.
