# EC Command Protocol — Reverse Engineering Report

> **Date:** 30/07/2026
> **Analyst:** reverse-engineer agent (static analysis, Ghidra decompilation)
> **Target:** IoTService.exe (original Xiaomi binary)
> **SHA256:** 29E89BA0DAA83D29F2A9083B8704FFF25D668B736664B5D6B27312971B0D4D2E
> **Consultor guidance:** MCPI pipe is a router, not a processor. Data comes from EC commands.

## Executive Summary

The EC command protocol uses a **4-phase state machine** over ECRAM at physical addresses `0xFE0B0F00` (status/data) and `0xFE0B0F01` (command). Each feature sends a 7-byte command template containing the cmd_id and device UID, then polls for ACK and RET responses. **16 distinct EC cmd_ids** were identified, mapping to all IoT features.

## 1. EC Command State Machine

### 1.1 RamIsReady (`FUN_140008900` @ VA `0x140008900`)

- **Status address:** `0xFE0B0F00`
- **Read size:** 1 byte
- **Ready condition:** `byte == 0x00`
- **Busy condition:** `byte != 0x00` (returns error 6)
- **No polling loop** — caller must retry

### 1.2 WriteCommand (`FUN_140007e20` @ VA `0x140007e20`)

- **Command address:** `0xFE0B0F01`
- **Command size:** 7 bytes
- **Trigger address:** `0xFE0B0F00`
- **Trigger byte:** `0x55`

**7-byte command template:**

```
[cmd_id, 0x01, 0x01, UID_byte0, UID_byte1, UID_byte2, UID_byte3]
```

UID is the device UID (lower 4 bytes, little-endian), stored in global `DAT_1400acc38`.

After writing the 7 bytes, writes `0x55` to `0xFE0B0F00` to trigger the command.

### 1.3 ReadCmdAck (`FUN_140007fb0` @ VA `0x140007fb0`)

- **Status address:** `0xFE0B0F00`
- **Read size:** 4 bytes
- **Poll interval:** 5ms (`Sleep(5)`)
- **Max retries:** 100

**Expected ACK (4 bytes):** `[0x55, cmd_id, 0x01, 0x02]`

**Patterns:**

- `[0x55, cmd_id, 0x01, 0x01]` = "last cmd" — EC still processing previous command. Sleep 5ms, retry.
- `[0x55, cmd_id, 0x01, 0x02]` = ACK received correctly. Return success.
- `[0x55, cmd_id, 0x01, 0x03]` = "next cmd" — EC already moved to RET phase. Retry.
- Mismatch: ResetEC, return error code based on first byte:
  - `0x11` → error 7
  - `0x22` → error 8 (timeout)
  - `0x33` → error 9
  - `0x44` → error 10
  - default → error 4 (ACK mismatch)

### 1.4 ReadCmdRet (`FUN_140008380` @ VA `0x140008380`)

- **Status address:** `0xFE0B0F00`
- **Read size:** 8 bytes
- **Poll interval:** 45ms (`Sleep(0x2D)`)
- **Max retries:** 60

**Expected RET (8 bytes):** `[0x55, cmd_id, 0x01, 0x03, <4 bytes data>]`

**Patterns:**

- `[0x55, cmd_id, 0x01, 0x02]` = "last ack" — EC still processing ACK. Retry.
- `[0x55, cmd_id, 0x01, 0x03, ...]` = RET received. Bytes 4-7 are the return data.
- Mismatch: ResetEC, return error code (same as ACK).

### 1.5 ResetEC (`FUN_140007d50` @ VA `0x140007d50`)

Writes a single zero byte (`0x00`) to `0xFE0B0F00` to clear the EC status.

### 1.6 ReadSensorData (`FUN_1400086d0` @ VA `0x1400086d0`)

- **Sensor address:** `0xFE0B0F08`
- **Read size:** 120 bytes (0x78)

### 1.7 WriteSensorData (`FUN_1400087f0` @ VA `0x1400087f0`)

- **Sensor address:** `0xFE0B0F08`
- **Max size:** 120 bytes (0x78)

## 2. Per-Feature cmd_id Map

| cmd_id | Feature                      | Function VA   | Confidence |
| ------ | ---------------------------- | ------------- | ---------- |
| 0x01   | GetBindStatus                | `0x140008bd0` | CONFIRMED  |
| 0x02   | SetBindStatus                | inline        | CONFIRMED  |
| 0x03   | ResetDevice                  | inline        | CONFIRMED  |
| 0x04   | WriteWiFiItem                | inline        | CONFIRMED  |
| 0x05   | EmptyWiFiItems               | inline        | CONFIRMED  |
| 0x06   | DeleteWiFiItem               | inline        | CONFIRMED  |
| 0x07   | ReadWiFiStatus               | `0x140008dc0` | CONFIRMED  |
| 0x08   | ReadWiFiCount                | inline        | CONFIRMED  |
| 0x09   | GetWiFiByIndex               | `0x140008fa0` | CONFIRMED  |
| 0x0A   | GetFwVersion                 | `0x140008a40` | CONFIRMED  |
| 0x0B   | GetModel                     | `0x140009190` | CONFIRMED  |
| 0x0C   | ConnectWiFi                  | `0x140009320` | CONFIRMED  |
| 0x0D   | GetDeviceID                  | `0x1400093f0` | CONFIRMED  |
| 0x0E   | SendLaptopStatus (SUSPEND)   | `0x1400095b0` | CONFIRMED  |
| 0x0F   | SendLaptopStatus (SHUTDOWN)  | `0x1400095b0` | CONFIRMED  |
| 0x10   | SendLaptopStatus (WIN_READY) | `0x1400095b0` | CONFIRMED  |

## 3. Per-Feature Response Layouts

### GetBindStatus (0x01)

- RET byte 0: `0x01` = bound, `0x02` = not bound
- If bound: ReadSensorData → offset 0 = UID size, offset 1-7 = UID bytes (byte-swapped to big-endian u64)

### GetFwVersion (0x0A)

- RET byte 0: `0x01` = success
- If success: ReadSensorData → offset 0 = string length, offset 1+ = ASCII firmware version string

### GetModel (0x0B)

- RET byte 0: `0x01` = success
- If success: ReadSensorData → offset 0 = string length, offset 1+ = ASCII model string

### GetDeviceID (0x0D)

- RET byte 0: `0x01` = success
- If success: ReadSensorData → offset 0 = DID size, offset 1-7 = DID bytes (byte-swapped to big-endian u64)

### ReadWiFiStatus (0x07)

- RET byte 0: WiFi status code (0x01-0x07, 0x0F = valid)
- If valid: ReadSensorData → offset 4 = SSID length, offset 5+ = SSID string

### ReadWiFiCount (0x08)

- RET byte 0: `0x01` = success
- If success: ReadSensorData → offset 0 = WiFi count (byte)

### GetWiFiByIndex (0x09)

- Pre-write: `[0x01, index]` to sensor data (index must be < 20)
- RET byte 0: `0x01` = success
- If success: ReadSensorData → full WiFi item structure (101 bytes)

### WriteWiFiItem (0x04)

- Pre-write: 101-byte WiFi item payload to sensor data
- RET byte 0: `0x01` or `0x02` = success

### DeleteWiFiItem (0x06)

- Pre-write: 37-byte payload `[0x25, 0x00, 0x00, 0x00, SSID_len, SSID...]` to sensor data
- RET byte 0: `0x01` or `0x02` = success

### EmptyWiFiItems (0x05)

- No pre-write payload
- RET byte 0: `0x01` = success

### ConnectWiFi (0x0C)

- No pre-write payload
- RET byte 0: `0x01` or `0x02` = success

### SendLaptopStatus

- LaptopStatus 4 (WinReady) → cmd_id 0x10
- LaptopStatus 6 (Suspending) → cmd_id 0x0E
- LaptopStatus 8 (Shutting) → cmd_id 0x0F
- RET byte 0: `0x01` = success

### GetDeviceStatus

- Uses WMI, NOT EC commands
- Queries `Worker_WMI::getDeviceStatus()` via `ACPI\PNP0C14\MIFS_0`

### SetChargingLimit

- NOT FOUND in EC command path
- Hypothesis: may use WMI `MiInterface` method or registry

## 4. ECRAM Address Map

| Address      | Size      | Purpose                                                                            |
| ------------ | --------- | ---------------------------------------------------------------------------------- |
| `0xFE0B0F00` | 1 byte    | Status/data register (read for status, write 0x55 to trigger, write 0x00 to reset) |
| `0xFE0B0F01` | 7 bytes   | Command register (write command template)                                          |
| `0xFE0B0F08` | 120 bytes | Sensor data buffer (read/write payload data)                                       |

## 5. Error Codes

| Code | Meaning                 |
| ---- | ----------------------- |
| 0    | Success                 |
| 1    | Invalid parameter       |
| 2    | IOCTL error             |
| 3    | Failed to write command |
| 4    | ACK mismatch            |
| 5    | RET mismatch            |
| 6    | Device busy             |
| 7    | EC error 0x11           |
| 8    | Timeout                 |
| 9    | EC error 0x33           |
| 10   | EC error 0x44           |
