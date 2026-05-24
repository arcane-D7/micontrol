# IoT Shim Access Report

## Scope

This report covers the Xiaomi IoT module surfaces that are accessible from `micontrol` through the deployed DriverStore shim.

Direct shim access means:

- the process runs from the IoTDriver DriverStore directory
- the IoTDriver path-prefix check passes
- the shim can open the IoTDriver device interface and issue its IOCTLs

It also distinguishes adjacent IoT surfaces that exist in the platform but are not currently routed through the shim.

## Directly Accessible Via Shim Today

### 1. IoTDriver device interface

- Device interface GUID: `{AB7924A1-3162-4010-B33B-837E87E25FBC}`
- Discovery path: SetupAPI enumeration in [src-tauri/src/bin/ecram_shim.rs](../src-tauri/src/bin/ecram_shim.rs) and [src-tauri/src/hw/ecram.rs](../src-tauri/src/hw/ecram.rs)

### 2. IOCTL `0x22E000` — ECRAM read

- Implemented in the shim CLI and backend wrappers
- Current entry points:
  - `ecram_shim.exe <addr_hex> <count>`
  - `ecram_shim.exe read <addr_hex> <count>`
  - `read_ecram_via_shim(addr, len)` in [src-tauri/src/hw/ecram.rs](../src-tauri/src/hw/ecram.rs)

Accessible physical regions through this primitive:

| Region | Address | Size | Status |
|---|---:|---:|---|
| ERAM | `0xFE0B0300` | `0x100` | accessible and partially decoded |
| SMA2 | `0xFE0B0A00` | `0x100` | accessible, not decoded |
| IoT status | `0xFE0B0F00` | `0x08` | accessible, not decoded |
| IoT sensors | `0xFE0B0F08` | `0x78` | accessible, partially characterized |

Named-region shim entry point:

- `ecram_shim.exe read-region ERAM`
- `ecram_shim.exe read-region SMA2`
- `ecram_shim.exe read-region IOT_STATUS`
- `ecram_shim.exe read-region IOT_SENSORS`

Backend helper:

- `read_named_region_via_shim(region)` in [src-tauri/src/hw/ecram.rs](../src-tauri/src/hw/ecram.rs)

### 3. IOCTL `0x22E004` — ECRAM write

- Implemented in the shim CLI
- Backend helper implemented but not yet exposed in a Tauri command/UI

Entry points:

- `ecram_shim.exe write <addr_hex> <hex_data>`
- `write_ecram_via_shim(addr, data)` in [src-tauri/src/hw/ecram.rs](../src-tauri/src/hw/ecram.rs)

Notes:

- the driver-side write validation policy is still not fully mapped
- this is the highest-risk surface because arbitrary EC writes may have side effects

### 4. Shim deployment / privilege path

- `deploy_ecram_shim()` copies `ecram_shim.exe` into the DriverStore directory using `SeRestorePrivilege`
- implementation lives in [src-tauri/src/hw/ecram.rs](../src-tauri/src/hw/ecram.rs)
- copy method uses `FILE_FLAG_BACKUP_SEMANTICS` to bypass the TrustedInstaller DACL

This makes the following end-to-end shim flow accessible today:

1. locate IoTDriver DriverStore directory from `HKLM\SYSTEM\CurrentControlSet\Services\IoTDriver\ImagePath`
2. deploy shim into that directory
3. spawn shim from DriverStore
4. call IoTDriver read/write IOCTLs
5. return JSON to the main app

## Decoded Data Currently Reachable Through The Shim

### ERAM fields already decoded

These fields are available via `read_eram_map()` in [src-tauri/src/hw/ecram.rs](../src-tauri/src/hw/ecram.rs):

| Offset | Field | Meaning |
|---:|---|---|
| `0x03` | `cpu_temp_c` | CPU temperature |
| `0x04..0x05` | `fan_rpm` | fan RPM |
| `0x06..0x07` | `fan2_rpm` | secondary fan RPM |
| `0x0A` | `cpu_power_w` | CPU power |
| `0x40` | `perf_profile` | performance profile byte |
| `0x42` | `tdp_w` | TDP-related byte |
| `0x80` | `ac_flags` | AC flags byte |
| `0x80 bit0` | `ac_connected` | AC present |
| `0x81` | `ac_adapter_w` / `ADPW` | AC adapter wattage |
| `0x8C..0x8D` | `battery_current_ma` / `BTCT` | battery current |
| `0x8E..0x8F` | `battery_capacity_mah` / `BTPR` | remaining battery capacity |
| `0x90..0x91` | `battery_voltage_mv` / `BTVT` | battery voltage |
| `0x96` | `charge_threshold_pct` | charging threshold byte |
| `0x97` | `battery_temp_c` | battery temperature |

### Existing app usage backed by shim fallback

- `try_get_ac_power_mw()` now falls back to shim if direct IoTDriver access is blocked
- `read_eram_map()` falls back to shim if direct IoTDriver access fails

## Adjacent IoT Platform Surfaces Not Currently Routed Through The Shim

These belong to the same Xiaomi IoT stack, but are not currently exercised through the DriverStore shim itself.

### 1. WMI class `MICommonInterface`

- Namespace: `root\WMI`
- Method: `MiInterface(InData: UInt8[], OutData: UInt8[], ReturnValue: UInt16)`
- Present in probe scripts such as `probe_mi_final.py`

Current state:

- discovered and documented
- not yet wired into `micontrol`
- not part of the current shim path

### 2. IoT WMI event classes

- `HID_EVENT20`
- `HID_EVENT21`
- `HID_EVENT22`
- `HID_EVENT23`

Current state:

- already subscribed directly in [src-tauri/src/hw/hotkeys.rs](../src-tauri/src/hw/hotkeys.rs)
- these are used for Xiaomi AI/Xiaomi key hotkey behavior
- they are part of the IoT ecosystem, but not accessed through `ecram_shim.exe`

### 3. IoTService IPC broker

- Named pipe reference appears in planning/docs (`LOCAL\IoTService_IPC_Broker`)
- protocol details remain incomplete

Current state:

- not implemented in `micontrol`
- not required for current shim read/write path

## Current Accessible Function Inventory

This is the exhaustive list of shim-backed operations implemented in the codebase now.

### CLI operations in `ecram_shim.exe`

1. `read <addr> <len>`
2. legacy read form: `<addr> <len>`
3. `write <addr> <hex_data>`
4. `read-region ERAM`
5. `read-region SMA2`
6. `read-region IOT_STATUS`
7. `read-region IOT_SENSORS`

### Rust backend functions in `micontrol`

1. `deploy_ecram_shim()`
2. `find_iotdriver_store_dir()`
3. `read_ecram_via_shim(addr, len)`
4. `read_named_region_via_shim(region)`
5. `write_ecram_via_shim(addr, data)`
6. `read_eram_map()`
7. `try_get_ac_power_mw()` with shim fallback

### Physical memory surfaces reachable by those functions

1. ERAM `0xFE0B0300..0xFE0B03FF`
2. SMA2 `0xFE0B0A00..0xFE0B0AFF`
3. IoT status `0xFE0B0F00..0xFE0B0F07`
4. IoT sensors `0xFE0B0F08..0xFE0B0F7F`

## Unknowns / Gaps

1. The meaning of most ERAM bytes is still unknown.
2. The meaning of the whole SMA2 region is still unknown.
3. The IoT status block bit layout is still unknown.
4. The driver-side safety rules for write IOCTL `0x22E004` are still not fully characterized.
5. `MICommonInterface.MiInterface()` subcommands are still not mapped.
6. The complete IoTService named-pipe protocol is still not mapped.

## Recommended Next Steps

1. Add a read-only Tauri debug command for `read_named_region_via_shim()` so ERAM/SMA2/status can be inspected live from the app.
2. Snapshot ERAM, SMA2 and IoT status across charger, fan, performance and battery state changes to expand the field map.
3. Gate `write_ecram_via_shim()` behind a debug-only command and test writes only on safe scratch bytes first.
4. Probe `MICommonInterface.MiInterface()` in parallel to determine whether some IoT functions are cleaner through WMI than through raw EC access.
5. Correlate DSDT field names with ERAM/SMA2 offsets and update the report as the map expands.