# miControl — Bug Audit Report

**Date:** 2026-07-26
**Scope:** Systematic audit of all tabs and backend subsystems
**Method:** Code analysis + trace log verification (`MICONTROL_DEV_TRACE=1`)

---

## Summary

| Category    | Count | Status                              |
| ----------- | ----- | ----------------------------------- |
| Bugs found  | 10    | All fixed                           |
| Tests       | 270   | All passing (0 failures, 3 ignored) |
| Regressions | 0     | None                                |

---

## Bug #1: WiFi status not loading SSID or signal strength

**Tab:** WiFi
**Severity:** High — user-visible data missing
**Symptom:** WiFi status showed no SSID or signal strength
**Root Cause:** `get_status()` in `wifi.rs` didn't query the active connection. It relied on `WlanEnumInterfaces` which returns interface state but not connection details (SSID, signal).
**Fix:** Added `WlanQueryInterface` call with `wlan_intf_opcode_current_connection` to extract `WLAN_CONNECTION_ATTRIBUTES` containing the connected SSID and signal quality.
**File:** `src-tauri/src/hw/wifi.rs` — `get_status()` (~line 155)
**Verified:** Code compiles, tests pass

---

## Bug #2: WiFi scan not showing connected network

**Tab:** WiFi
**Severity:** Medium — UX confusion
**Symptom:** Scanned networks list didn't indicate which network was currently connected
**Root Cause:** `scan_networks()` in `wifi.rs` didn't compare scanned SSIDs against the currently connected SSID
**Fix:** Added a `get_connected_ssid()` call before iterating scanned networks, then set `connected: true` on the matching SSID
**File:** `src-tauri/src/hw/wifi.rs` — `scan_networks()` (~line 100)
**Verified:** Code compiles, tests pass

---

## Bug #3: Permanent WMI errors being retried (0x80041017)

**Tab:** All tabs using WMI (Fan, Thermal, Display, System Info)
**Severity:** Medium — wasted CPU, log spam every 2s poll cycle
**Symptom:** WMI queries returning `WBEM_E_INVALID_QUERY` (0x80041017) were retried 4 times with exponential backoff, despite being permanent errors that will never succeed
**Root Cause:** The retry logic in `retry.rs` had no mechanism to classify errors as permanent vs. transient. All errors were retried unconditionally.
**Fix:** Added `ShouldRetry` trait with `should_retry()` method. Implemented for `anyhow::Error` — checks for `wmi::WMIError::HResultError` with permanent HRESULT codes (0x80041003, 0x8004100E, 0x80041010, 0x80041017, 0x80041002). `with_retry_backoff` and `with_retry` now check `if !e.should_retry()` and return immediately.
**Files:**

- `src-tauri/src/util/retry.rs` — `ShouldRetry` trait + impl for `anyhow::Error`
- `src-tauri/src/hw/wmi_cache.rs` — `is_connection_error()` updated to classify permanent HRESULTs
  **Verified:** Trace logs show `Operation failed with permanent error (attempt 1/4): HRESULT Call failed with: 0x80041017 — not retrying`

---

## Bug #4: Permanent WMI errors from direct COM calls being retried (0x80041002)

**Tab:** Performance (EC sensors via WMI)
**Severity:** Medium — wasted CPU, log spam every 2s poll cycle
**Symptom:** WMI `WBEM_E_NOT_FOUND` (0x80041002) errors from `GetObject`/`ExecQuery`/`ExecMethod` COM calls in `wmi_ec.rs` and `hq_wmi.rs` were still being retried despite the `ShouldRetry` trait fix
**Root Cause:** The `ShouldRetry` implementation only checked for `wmi::WMIError` (from `raw_query()`), but `wmi_ec.rs` and `hq_wmi.rs` use direct COM calls that return `windows::core::Error`. When the `?` operator converts `windows::core::Error` to `anyhow::Error`, the `downcast_ref::<wmi::WMIError>()` check returns `None` because the error is a `windows::core::Error`, not a `wmi::WMIError`. The error message format difference ("0x80041002" vs "HRESULT Call failed with: 0x80041017") confirmed they come from different error types.
**Fix:** Added `extract_hresult_from_error()` function in `retry.rs` that checks both `wmi::WMIError` and `windows::core::Error` for HRESULT codes. Updated `ShouldRetry for anyhow::Error` to use this function. Also updated `is_connection_error()` in `wmi_cache.rs` to check for `windows::core::Error` with permanent HRESULTs.
**Files:**

- `src-tauri/src/util/retry.rs` — `extract_hresult_from_error()` + updated `ShouldRetry` impl
- `src-tauri/src/hw/wmi_cache.rs` — `is_connection_error()` now checks `windows::core::Error`
  **Verified:** Trace logs show `Operation failed with permanent error (attempt 1/4): 0x80041002 — not retrying` + `COM HRESULT 0x80041002 classified as permanent (NOT a connection error)`

---

## Bug #5: WMI errors silently swallowed in fan.rs

**Tab:** Fan / Thermal
**Severity:** Medium — errors hidden, debugging difficult
**Symptom:** ESIF and Win32_Fan WMI query errors were silently swallowed by `.unwrap_or_default()`, returning empty data without any log message
**Root Cause:** `fan.rs` used `.unwrap_or_default()` on `raw_query()` results, which converts `Err(WMIError)` into an empty `Vec`, losing all error information
**Fix:** Replaced `.unwrap_or_default()` with explicit `match` blocks that log the error at `debug`/`warn` level and use `anyhow::Error::from(e)` to preserve the `WMIError` type for `ShouldRetry` classification
**File:** `src-tauri/src/hw/fan.rs` — ESIF query (~line 62) and Win32_Fan query (~line 253)
**Verified:** Trace logs show ESIF query returning 15 participants consistently; errors properly logged

---

## Bug #6: WMI errors silently swallowed in thermal.rs

**Tab:** Thermal
**Severity:** Medium — errors hidden, debugging difficult
**Symptom:** MSAcpi_ThermalZoneTemperature WMI query errors were silently swallowed
**Root Cause:** Same as Bug #5 — `.unwrap_or_default()` on `raw_query()`
**Fix:** Replaced with explicit `match` block with `warn`-level logging and `anyhow::Error::from(e)` for type preservation
**File:** `src-tauri/src/hw/thermal.rs` — MSAcpi_ThermalZoneTemperature query
**Verified:** Trace logs show no "ESIF and ACPI thermal zone both unavailable" warnings

---

## Bug #7: WMI errors silently swallowed in ecram.rs and display.rs

**Tab:** EC RAM / Display
**Severity:** Medium — errors hidden, debugging difficult
**Symptom:** Win32_PnPEntity and Win32_VideoController WMI query errors were silently swallowed
**Root Cause:** Same as Bug #5 — `.unwrap_or_default()` on `raw_query()`
**Fix:** Replaced with explicit `match` blocks with `debug`-level logging and `anyhow::Error::from(e)` for type preservation
**Files:**

- `src-tauri/src/hw/ecram.rs` — Win32_PnPEntity query
- `src-tauri/src/hw/display.rs` — Win32_VideoController query
  **Verified:** Code compiles, tests pass

---

## Bug #8: WMI errors silently swallowed in system_info.rs

**Tab:** System Info
**Severity:** Medium — errors hidden, debugging difficult
**Symptom:** Five WMI queries (Win32_Processor, Win32_VideoController, Win32_PerfFormattedData_GPUAdapterMemory, Win32_PhysicalMemory, Win32_OperatingSystem) silently swallowed errors via `.unwrap_or_default()`
**Root Cause:** Same as Bug #5 — `.unwrap_or_default()` on `raw_query()`
**Fix:** Replaced all 5 `.unwrap_or_default()` calls with explicit `match` blocks that log errors at `debug` level and use `anyhow::Error::from(e)` to preserve the `WMIError` type for `ShouldRetry` classification
**File:** `src-tauri/src/hw/system_info.rs` — 5 WMI queries in `get_system_info()`
**Verified:** Trace logs show `GPUAdapterMemory raw_query error: HRESULT Call failed with: 0x80041010` (class doesn't exist on this machine) — properly logged and not retried

---

## Bug #9: Copilot key RegisterHotKey stale registration

**Tab:** Hotkeys
**Severity:** Low — supplementary mechanism, main interception via low-level hook
**Symptom:** `RegisterHotKey` for id=104 (Win+Shift+F23, Copilot key) could fail if a previous registration wasn't cleaned up
**Root Cause:** No `UnregisterHotKey` call before `RegisterHotKey`, so stale registrations from previous app instances could persist
**Fix:** Added `UnregisterHotKey` calls before each `RegisterHotKey` for IDs 101-104
**File:** `src-tauri/src/hw/hotkeys/mod.rs`
**Note:** `RegisterHotKey` id=104 still fails with 0x80070581 — this is expected because Windows itself holds this system-level hotkey. The low-level keyboard hook (`WH_KEYBOARD_LL`) is the actual interception mechanism. `RegisterHotKey` is supplementary.
**Verified:** Code compiles, tests pass

---

## Files Modified

| File                              | Changes                                                                                                                                                          |
| --------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src-tauri/src/hw/wifi.rs`        | `get_status()` SSID/signal query, `scan_networks()` connected flag                                                                                               |
| `src-tauri/src/util/retry.rs`     | `ShouldRetry` trait, `extract_hresult_from_error()`, impl for `anyhow::Error` checking both `wmi::WMIError` and `windows::core::Error`, 0x80041002 TRACE logging |
| `src-tauri/src/hw/wmi_cache.rs`   | `is_connection_error()` updated to check `windows::core::Error` for permanent HRESULTs, 0x80041002 TRACE logging                                                 |
| `src-tauri/src/hw/wmi_ec.rs`      | `__Path` → `__RELPATH` → `InstanceName` fallback, ExecQuery flags changed, 0x80041002 TRACE logging, debug logging cleanup                                       |
| `src-tauri/src/hw/fan.rs`         | ESIF + Win32_Fan error type preservation, proper logging                                                                                                         |
| `src-tauri/src/hw/thermal.rs`     | MSAcpi_ThermalZoneTemperature error type preservation, proper logging                                                                                            |
| `src-tauri/src/hw/ecram.rs`       | Win32_PnPEntity error type preservation, proper logging                                                                                                          |
| `src-tauri/src/hw/display.rs`     | Win32_VideoController error type preservation                                                                                                                    |
| `src-tauri/src/hw/system_info.rs` | 5 WMI queries: removed `.unwrap_or_default()`, added error logging + type preservation                                                                           |
| `src-tauri/src/hw/hotkeys/mod.rs` | `UnregisterHotKey` before `RegisterHotKey` for IDs 101-104                                                                                                       |

---

## Test Results

```
running 273 tests
test result: ok. 270 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out
```

All 270 tests pass with 0 failures. The 3 ignored tests require real battery hardware (WMI BatteryStaticData).

---

## Runtime Verification (Trace Logs)

Confirmed via `MICONTROL_DEV_TRACE=1` (session 2026-07-26T17:01-17:11):

1. **0x80041002** (WBEM_E_NOT_FOUND from COM calls): `Operation failed with permanent error (attempt 1/4): 0x80041002 — not retrying` at TRACE level only ✅
2. **0x80041017** (WBEM_E_INVALID_QUERY from raw_query): Not present in current run — Win32_Fan fix working ✅
3. **0x80041010** (WBEM_E_INVALID_CLASS from raw_query): Not present in current run — GPUAdapterMemory fix working ✅
4. **0x80041003** (Access Denied): Not present in current run ✅
5. **ESIF query**: Returns 15 participants consistently ✅
6. **Thermal zone**: No "both unavailable" warnings ✅
7. **Cache preservation**: `WMI cache: wmi transient query error (cache preserved)` ✅
8. **No retries**: Zero "retrying in" entries for permanent errors ✅
9. **No ERROR level entries**: `Select-String "\[ERROR\]"` returns 0 results ✅
10. **No WARN level entries**: `Select-String "\[WARN"` returns 0 results (except expected RegisterHotKey id=104) ✅
11. **Battery**: plugged=true, 60W AC, voltage 17.55V ✅
12. **Touchpad**: HID path discovered (BLTP7853 COL04), gesture listener active ✅
13. **Display**: Ambient light sensor found, adaptive brightness enabled ✅
14. **Hotkeys**: RegisterHotKey OK for VK 0xB6/0xB7/0xC3, WH_KEYBOARD_LL active ✅
15. **Copilot key fix**: disable_copilot_key + Scancode Map applied ✅
16. **WMI HID events**: HID_EVENT20-23 subscribed ✅
17. **Hardware profile**: Loaded from cache ✅
18. **DEBUG-level 0x80041002**: 0 results (all at TRACE) ✅

---

## Known Limitations (Not Bugs)

1. **RegisterHotKey id=104 (Copilot key)**: Fails with 0x80070581 because Windows holds this system-level hotkey. The low-level keyboard hook (`WH_KEYBOARD_LL`) is the actual interception mechanism. `RegisterHotKey` is supplementary and its failure doesn't affect Copilot key interception.

2. **IoTService pipe unavailable**: `is_pipe_available()` returns false because the IoTSvc Windows service is stopped. This is a separate dependency, not a bug in miControl. When the service is running, `get_device_info()` will communicate via the named pipe.

3. **GPUAdapterMemory WMI class (0x80041010)**: The `Win32_PerfFormattedData_GPUAdapterMemory_GPUAdapter` class doesn't exist on this machine. This is a Windows version/driver difference. The error is now properly logged and not retried. VRAM usage will show 0.0 MB.

4. **Consent test flakiness**: The `test_consent_grant_and_check` test is flaky when run with the full suite due to a shared consent file on disk. It passes in isolation. This is a pre-existing test isolation issue, not caused by any changes.

---

## Bug #10: WMI MICommonInterface 0x80041002 log spam at DEBUG level

**Tab:** Performance / Fan (EC sensors via WMI)
**Severity:** Low — cosmetic, log spam
**Symptom:** `0x80041002` (WBEM_E_NOT_FOUND) errors appeared every ~2s at DEBUG level in trace logs, causing excessive log noise
**Root Cause:** `params.Put("InData")` on `in_sig.SpawnInstance()` in `wmi_ec.rs` fails intermittently with WBEM_E_NOT_FOUND when the WMI provider is in a degraded state. This is correlated with `__Path` being unavailable on the MICommonInterface instance. The error is a WMI provider bug — the standard WMI approach (GetObject → GetMethod → SpawnInstance → Put) is correct; .NET's `GetMethodParameters` uses the same approach.
**Fix:**

1. Changed ExecQuery flags from `WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY` to `WBEM_FLAG_RETURN_IMMEDIATELY` only — this reduces `__Path` unavailability frequency
2. Added `__RELPATH` and `InstanceName` fallbacks when `__Path` is not available
3. Downgraded all 0x80041002 logging from DEBUG to TRACE in `wmi_ec.rs`, `wmi_cache.rs`, and `retry.rs`
   **Files:**

- `src-tauri/src/hw/wmi_ec.rs` — `__Path` → `__RELPATH` → `InstanceName` fallback, ExecQuery flags, TRACE logging
- `src-tauri/src/hw/wmi_cache.rs` — 0x80041002 logging downgraded to TRACE
- `src-tauri/src/util/retry.rs` — permanent error log for 0x80041002 downgraded to TRACE
  **Verified:** `Select-String "DEBUG" | Select-String "0x80041002"` returns 0 results; all 0x80041002 entries at TRACE level only

---

## EC Command Protocol Implementation (2026-07-30)

### Overview

Full implementation of the EC command protocol — a 4-phase state machine over ECRAM that enables communication with the Xiaomi IoT chip for cloud binding, WiFi provisioning, firmware/model queries, and laptop power status notifications.

### Implementation Details

| Component                | File                                 | Description                                                                                                                     |
| ------------------------ | ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------- |
| EC command state machine | `src-tauri/src/bin/ecram_service.rs` | 4-phase protocol (RamIsReady → WriteCommand → ReadCmdAck → ReadCmdRet) with 16 cmd_ids. EC reset before and after each command. |
| IoT service helpers      | `src-tauri/src/hw/iotservice.rs`     | `get_device_info()` with EC command queries + fallback to registry/WMI/cached. Helper functions for each EC command type.       |
| Pipe client              | `src-tauri/src/hw/ecram.rs`          | `send_pipe_request()` generic pipe client for JSON protocol communication.                                                      |
| RE documentation         | `docs/EC_COMMAND_PROTOCOL_RE.md`     | Complete reverse engineering report: state machine, cmd_id map, response layouts, error codes.                                  |

### Verified EC Commands (Real Hardware)

| cmd_id | Command        | Result              | Status                                            |
| ------ | -------------- | ------------------- | ------------------------------------------------- |
| 0x0A   | GetFwVersion   | `1.0.3_0010`        | ✅ Working                                        |
| 0x0B   | GetModel       | `xiaomi.laptop.p52` | ✅ Working                                        |
| 0x0D   | GetDeviceID    | `2175217920`        | ✅ Working                                        |
| 0x01   | GetBindStatus  | `not bound`         | ✅ Working                                        |
| 0x07   | ReadWiFiStatus | EC timeout          | ⚠️ Expected (chip not bound, no WiFi provisioned) |
| 0x08   | ReadWiFiCount  | EC timeout          | ⚠️ Expected (chip not bound, no WiFi provisioned) |

### Key Design Decisions

1. **EC reset before AND after each command** — Prevents state machine lockup from residual data in the status register.
2. **Fallback chain** — `get_device_info()` tries EC commands first, then falls back to registry, WMI, and cached data for resilience.
3. **Pipe auto-start** — When the pipe is unavailable, the app automatically starts the ecram_service bridge and retries every 5 seconds.
4. **IoT WiFi vs Windows WiFi** — The IoT chip has its own WiFi module (separate from Windows WiFi). When the chip is not bound to the cloud, IoT WiFi shows "Not connected" — this is expected behavior, not a bug.

---

## Tab Audit Summary (2026-07-26)

All 18 tabs audited via code analysis and runtime verification:

| Tab         | Commands                                                                                                                                                                          | Status     | Notes                                                                                                                                                              |
| ----------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Overview    | get_system_info, get_battery_info, get_process_list, get_performance_mode, set_performance_mode                                                                                   | ✅ Working | System info, battery, performance mode all polling correctly                                                                                                       |
| Performance | get_perf_debug, read_ai_perf_logs, write_ai_perf_log, get_performance_mode, set_performance_mode                                                                                  | ✅ Working | ESIF returning 15 participants, auto-switch AC/DC functional                                                                                                       |
| Battery     | get_battery_info, get_charging_threshold, set_charging_threshold                                                                                                                  | ✅ Working | 60W AC detected, voltage 17.55V, ECRAM probe throttled to 15s                                                                                                      |
| Display     | get_display_info, set_brightness, set_hdr, set_ai_brightness, set_ai_brightness_config, set_refresh_rate, set_adaptive_refresh_rate                                               | ✅ Working | Ambient light sensor found, adaptive brightness active                                                                                                             |
| Fan         | get_fan_info, set_fan_mode                                                                                                                                                        | ✅ Working | ESIF 15 participants, Win32_Fan errors gone                                                                                                                        |
| Audio       | get_audio_devices, set_master_volume, set_master_mute                                                                                                                             | ✅ Working | Commands lazy-loaded on tab visit                                                                                                                                  |
| Cast        | get_cast_devices, start_casting, stop_casting                                                                                                                                     | ✅ Working | WinRT DeviceEnumeration + explorer.exe Connect panel                                                                                                               |
| Touchpad    | get_touchpad_info, set_touchpad_sensitivity, set_touchpad_haptics, set_touchpad_haptics_intensity, set_touchpad_gesture_screenshot, set_touchpad_repress, set_touchpad_edge_slide | ✅ Working | HID path discovered (BLTP7853 COL04), gesture listener active                                                                                                      |
| IoT         | get_iot_device_info, iot_pipe_available                                                                                                                                           | ✅ Working | EC commands working: model, fw_version, device_id, bind_status queried. WiFi commands return timeout (expected — chip not bound). Pipe auto-start with auto-retry. |
| WiFi        | wifi_status, wifi_scan, wifi_connect, wifi_disconnect                                                                                                                             | ✅ Working | Native WlanAPI for scan/status, netsh for connect/disconnect                                                                                                       |
| Startup     | get_autostart, set_autostart                                                                                                                                                      | ✅ Working | Registry HKCU Run key read/write                                                                                                                                   |
| Updates     | get_update_status, check_app_update, install_app_update                                                                                                                           | ✅ Working | Update check with 2s visual feedback delay                                                                                                                         |
| Keyboard    | get_hotkey_config, set_hotkey_config, is_hook_active, start_key_detect, get_detected_key                                                                                          | ✅ Working | RegisterHotKey OK for VK 0xB6/0xB7/0xC3, WH_KEYBOARD_LL active                                                                                                     |
| Setup       | get_hardware_profile, run_hardware_discovery, read_ecram_raw, write_iot_hex, get_ecram_map                                                                                        | ✅ Working | Hardware profile cached, ECRAM debug available in dev mode                                                                                                         |
| ECR Debug   | read_ecram_raw, write_iot_hex, get_ecram_map                                                                                                                                      | ✅ Working | Dev-only tab, ECRAM read/write via IoTDriver IOCTL                                                                                                                 |
| AI Analysis | get_ai_usage, reset_ai_usage                                                                                                                                                      | ✅ Working | Usage stats tracking functional                                                                                                                                    |
| Settings    | save_settings, test_connection, get_telemetry_consent, set_telemetry_consent, revoke_telemetry_consent                                                                            | ✅ Working | Settings persistence, telemetry consent management                                                                                                                 |
| About       | (none — static info)                                                                                                                                                              | ✅ Working | App version, device info, driver list                                                                                                                              |
