# Hardware Gap Analysis: Xiaomi PC Manager vs MiControl

> **Date:** 2026-07-24
> **Machine:** Xiaomi Book Pro 14 (TM2424, SN 77079/26RV00757)
> **XPM Version:** 5.8.0.57 (uninstalled, logs retained)
> **MiControl Version:** 0.1.13
> **Investigation Method:** XPM log analysis (ProgramData\MI), registry forensics, WMI probing, Consultor internet research, reverse-engineering analysis

---

## Executive Summary

This report documents all hardware features present in the official Xiaomi PC Manager (XPM) that MiControl does **not** currently implement. The analysis is based on forensic examination of XPM's log files, registry entries, uninstaller records, and binary manifests left behind after XPM was uninstalled, combined with internet research on Xiaomi's hardware interfaces.

**Key findings:**

- **67 internal API methods** were identified in XPM's SvrCModule log
- **15 feature gaps** were catalogued and analyzed
- **6 gaps are Easy/Medium difficulty** and can be implemented using existing MiControl infrastructure
- **5 gaps are Hard** and require proprietary Xiaomi SDKs or cloud APIs
- **4 gaps are Very Hard** and depend on Xiaomi's Lyra cross-device framework or signed binaries

---

## 1. XPM Architecture Overview

### Installation Structure (Reconstructed from Logs)

```
C:\Program Files\MI\XiaomiPCManager\5.8.0.57\
├── XiaomiPcManager.exe          # Main UI (WinUI 3 / .NET 6)
├── XiaomiPcManager.dll          # Core logic
├── XiaomiPcHost.exe             # Background host process
├── SvrCModule.dll               # Server-side control module (67 API methods)
├── SvrCModuleClrWrapper.dll     # .NET wrapper for SvrCModule
├── EcIoSdk.dll                  # EC I/O SDK (embedded controller access)
├── OSDUtility.exe               # On-screen display utility
├── OSDLauncher.exe              # OSD launcher
├── MiSmartShareDevice.exe       # Cross-device smart share
├── MiSmartShareDLL.dll          # Smart share core
├── MiSmartShareHandoff.exe      # App handoff
├── MiPlayCastService.exe        # Screen cast service
├── MiPlayCastSDK.dll            # Cast SDK
├── MiScreenShare.exe            # Screen sharing
├── MiDistributedCamera.dll      # Phone-as-webcam
├── MiDistributedCameraBroker.exe # Camera broker (64-bit)
├── MiDistributedCameraBroker32.exe # Camera broker (32-bit)
├── MiDistributedAudio.dll       # Distributed audio
├── MiDistributedFileServer.dll  # Distributed file server
├── MiDropTransfer.dll           # File transfer
├── MiDropShellExt.dll           # Shell extension (drag & drop)
├── MiDropDeskband.dll           # Deskband UI
├── MiTelephone.dll              # Call relay
├── MiMultiDeviceConnection.dll  # Multi-device manager
├── MiPCAudio.exe                # PC audio service
├── SubtitleTranscriptor.dll     # Real-time subtitles
├── LibSubtitleBusiness.dll      # Subtitle business logic
├── LibAivsAdapter.dll           # AI Voice Service adapter
├── LibAudioRecorder.dll         # Audio recording
├── CleanerEngine.dll            # System cleanup engine
├── CleanerProxy.dll             # Cleanup proxy
├── MiScanner.dll                # Security scanner
├── MiSceneClient.dll            # Scenario recognition
├── PcControlCenter.dll          # PC control center
├── PcCSharpIPC.dll              # C# IPC
├── PcyybAssistant.exe           # PC assistant (Pcyyb = PC应用宝)
├── VirtualCameraSDK.dll         # Virtual camera SDK
├── VirtualDisplay.dll           # Virtual display
├── Color_Verification.dll       # Color verification
├── MiTouchGesture.dll           # Touch gesture support
├── MiTwsRecords.dll             # TWS earbud records
├── libMiPods.dll                # Mi Pods (earbuds) library
├── RealtekBTXiaomi.dll          # Realtek Bluetooth (Xiaomi custom)
├── RealtekWiFi.dll              # Realtek WiFi (Xiaomi custom)
├── QcBtWrapper.dll              # Qualcomm Bluetooth wrapper
├── SambaServer.dll / .exe       # SMB file sharing server
├── AISearchSDK.dll              # AI search SDK
├── aisearchservice.dll          # AI search service
├── OneTrackDll.dll              # Telemetry tracking
├── OneTrackService.dll          # Telemetry service
├── MIPrivacyAgreement.dll       # Privacy agreement
├── PrivacyAuthorization.dll     # Privacy authorization
├── micont_service.exe           # Lyra micont service
├── miexpress.dll                # Lyra express
├── micont_rtm.dll               # Lyra RTM
├── dist_service.exe             # Distributed service
├── DistributedService.exe       # Distributed service (alt)
├── handoff_svc.exe              # Handoff service
├── MAFSvr.exe                   # MAF server
├── AndrowsInstaller.exe         # Android app installer
├── XaAppStore.exe               # Xiaomi App Store
├── InstallIcc.exe               # ICC color profile installer
├── DeleteDriver.exe             # Driver deletion tool
├── devcon.exe                   # Device console utility
├── pnputil.exe                  # PnP utility
├── FixServiceTool.exe           # Service fix tool
├── uninstall.exe                # Uninstaller
├── Driver_Uninstall.cmd         # Driver uninstall script
├── drivers\EcIo\XiaomiEcIo.sys  # EC I/O kernel driver
└── icc\                         # ICC color calibration files
    ├── Protect00_SDR_EDO4503.m3d
    ├── Protect01_SDR_EDO4503.m3d
    ├── Protect99_SDR_EDO4503.m3d
    ├── D65_P3_Cali.m3d
    ├── SRGB_Cali.m3d
    └── P3_Cali.m3d
```

### XPM Service Architecture

```
C:\ProgramData\MI\
├── AIService\          # AI service (AIService.exe, AIBroker)
├── AIoT\               # AI + IoT cross-device (Lyra, MiDrop, MiTrust)
├── Dist\               # Distributed service
├── DistData\           # Distributed data
├── DistFS\             # Distributed file system
├── DistFsService\      # DistFS service
├── DistFsSvc\          # DistFS service (alt)
├── DMFT\               # Device Management Framework Transform
├── Handoff\            # App handoff
├── IoTService\         # IoT service (EC commands, device control)
├── Lyra\               # Lyra IPC framework logs
├── MiAIBrightness\     # AI adaptive brightness
├── MiDeviceService\    # Device service
├── MiDropService\      # File transfer service
├── MiHygieneBroker\    # System hygiene (cleanup) broker
├── Miplay\             # Mi Play (screen cast/mirror)
├── MIPrivacyAgreement\ # Privacy agreement
├── MiScenarioRecognition\ # Scenario recognition
├── MiService\          # Core MI service
├── MiSmartShare\       # Smart share
├── OSDLauncher\        # OSD launcher
├── SvrCModule\         # Server control module (67 API methods)
├── Xiaomi Support Assistant\ # Support assistant
└── XiaomiAISearch\     # AI search
```

### XPM IPC Architecture

XPM uses a **Lyra IPC framework** for inter-process communication:

- **Pipe:** `\\.\Pipe\pipe123.sock`
- **SDK Version:** lyra 5.1.167.10 / lyra_rpc_1.1.25.1
- **Services:** MediaResourceService, com.xiaomi.mirror:cast
- **Transports:** BLE (64), BT (1), IP Bonjour (2), IP P2P (16), IP SoftAP (32), NFC (4)

---

## 2. XPM SvrCModule API Methods (67 Total)

These are the internal JSON-RPC style methods XPM's SvrCModule.dll exposed to the UI:

### Battery & Charging (10 methods)

| Method                                       | Description                                                     | MiControl Has?   |
| -------------------------------------------- | --------------------------------------------------------------- | ---------------- |
| `get_battery_health_status`                  | Battery health classification (Good/Fair/Poor)                  | ❌ No            |
| `get_battery_original_info`                  | Factory battery data (manufacturer, chemistry, design capacity) | ❌ No            |
| `get_charging_mode`                          | Current charging mode                                           | ❌ No            |
| `get_charging_protect`                       | Battery Care toggle state (EC 0xA4)                             | ❌ No            |
| `get_charging_threshold`                     | Charge limit threshold value                                    | ✅ Yes (presets) |
| `is_support_hyper_charging`                  | Hyper charging capability probe                                 | ❌ No            |
| `is_support_longbatterylife_and_intelligent` | Long battery life + intelligent mode support                    | ❌ No            |
| `resume_charging_protect`                    | Resume charging protection after suspend                        | ❌ No            |
| `register_battery_notify`                    | Battery notification callback                                   | ❌ No            |
| `register_battery_percentage`                | Battery percentage callback                                     | ❌ No            |

### Performance & Power (8 methods)

| Method                                | Description                      | MiControl Has? |
| ------------------------------------- | -------------------------------- | -------------- |
| `get_workLoad_mode`                   | Current performance mode         | ✅ Yes         |
| `get_workLoad_mode_decepticon_enable` | Decepticon mode enable state     | ✅ Yes         |
| `get_turbo_engine_enable`             | OS Turbo engine enable state     | ❌ No          |
| `get_first_set_powersave`             | First power-save setting         | ❌ No          |
| `get_interconnect_power_saving`       | Cross-device power saving        | ❌ No          |
| `register_workLoad_mode_change`       | Performance mode change callback | ❌ No          |
| `register_turbo_engine_changed`       | Turbo engine change callback     | ❌ No          |
| `register_close_powersave`            | Close power-save callback        | ❌ No          |

### Display & Eye Protection (5 methods)

| Method                           | Description                         | MiControl Has? |
| -------------------------------- | ----------------------------------- | -------------- |
| `get_aibrightness_state`         | AI brightness state                 | ✅ Yes         |
| `set_aibrightness`               | Set AI adaptive brightness          | ✅ Yes         |
| `is_support_ai_brightness`       | AI brightness capability probe      | ✅ Yes         |
| `register_eye_protection_change` | Eye protection mode change callback | ❌ No          |
| `register_monitor_change`        | Monitor change callback             | ❌ No          |

### Touchpad (6 methods)

| Method                              | Description                | MiControl Has? |
| ----------------------------------- | -------------------------- | -------------- |
| `get_haptic_feedback`               | Haptic feedback state      | ✅ Yes         |
| `get_touchpad_edge_sliding`         | Edge slide gesture state   | ✅ Yes         |
| `get_touchpad_gestures_screenshot`  | Gesture screenshot state   | ✅ Yes         |
| `get_touchpad_pressing_sensitivity` | Pressing sensitivity level | ✅ Yes         |
| `get_touchpad_vibration_intensity`  | Vibration intensity level  | ✅ Yes         |
| `set_touchpad_pressing_sensitivity` | Set pressing sensitivity   | ✅ Yes         |
| `set_touchpad_vibration_intensity`  | Set vibration intensity    | ✅ Yes         |

### Meeting Assistant / AI (5 methods)

| Method                             | Description                                                 | MiControl Has? |
| ---------------------------------- | ----------------------------------------------------------- | -------------- |
| `get_meeting_assistant_settings`   | Meeting assistant config (mic_nc, spk_nc, center, subtitle) | ❌ No          |
| `get_enroll_train_data`            | Voice enrollment training data                              | ❌ No          |
| `set_ai_noise_canceling_mode`      | AI noise cancellation mode                                  | ❌ No          |
| `set_meeting_bandwidth_protection` | Meeting bandwidth protection                                | ❌ No          |
| `get_meeting_bandwidth_protection` | Meeting bandwidth protection state                          | ❌ No          |

### System Maintenance (8 methods)

| Method                                          | Description                    | MiControl Has? |
| ----------------------------------------------- | ------------------------------ | -------------- |
| `get_abnormal_restart_environment_recovery`     | Crash recovery state           | ❌ No          |
| `get_application_anomaly_monitoring_and_repair` | App anomaly monitoring         | ❌ No          |
| `get_insufficient_disk_space_reminder`          | Low disk space reminder        | ❌ No          |
| `get_remote_control_state`                      | Remote control state           | ❌ No          |
| `get_replacement_assistant_state`               | Replacement assistant state    | ❌ No          |
| `download_replacement_assistant`                | Download replacement assistant | ❌ No          |
| `install_replacement_assistant`                 | Install replacement assistant  | ❌ No          |
| `open_replacement_assistant`                    | Open replacement assistant     | ❌ No          |

### Driver Management (5 methods)

| Method                           | Description                                                     | MiControl Has?     |
| -------------------------------- | --------------------------------------------------------------- | ------------------ |
| `scan_drivers`                   | Scan for driver updates                                         | ✅ Partial         |
| `get_drivers_detail`             | Detailed driver info (name, version, size, status, hardware_id) | ❌ No (basic only) |
| `set_driver_visited`             | Mark driver as visited                                          | ❌ No              |
| `set_unhandled_driver_count`     | Set unhandled driver count                                      | ❌ No              |
| `register_driver_red_dot_status` | Driver red-dot notification callback                            | ❌ No              |

### Function Key & Input (1 method)

| Method             | Description             | MiControl Has? |
| ------------------ | ----------------------- | -------------- |
| `get_function_key` | Fn key behavior setting | ❌ No          |

### Network & Status (5 methods)

| Method                  | Description              | MiControl Has? |
| ----------------------- | ------------------------ | -------------- |
| `get_network_status`    | Network status           | ❌ No          |
| `get_mi_service_status` | MI service status        | ❌ No          |
| `get_app_store_state`   | App store state          | ❌ No          |
| `get_app_version`       | App version              | ❌ No          |
| `get_push_config`       | Push notification config | ❌ No          |

### Registration Callbacks (14 methods)

| Method                                     | Description                       | MiControl Has? |
| ------------------------------------------ | --------------------------------- | -------------- |
| `register_ac_power_status`                 | AC power change callback          | ❌ No          |
| `register_intent`                          | Intent callback                   | ❌ No          |
| `register_login_changed`                   | Login state change callback       | ❌ No          |
| `register_main_window_close`               | Main window close callback        | ❌ No          |
| `register_mi_device_service_status_change` | Device service status change      | ❌ No          |
| `register_mi_service_status_change`        | MI service status change          | ❌ No          |
| `register_network_change`                  | Network change callback           | ❌ No          |
| `register_stage_change`                    | Stage change callback             | ❌ No          |
| `register_intelligent_acceleration_status` | Intelligent acceleration callback | ❌ No          |
| `request_status_of_app`                    | Request app status                | ❌ No          |
| `set_js_log`                               | Set JS log                        | ❌ No          |
| `get_feedback_type_list`                   | Get feedback type list            | ❌ No          |
| `get_last_scan_time`                       | Get last scan time                | ❌ No          |

---

## 3. Feature Gap Analysis

### Gap 1: Battery Care Toggle (EC Register 0xA4)

**Priority:** 🔴 **HIGH** — Easy to implement, high user value

**What it is:** A master on/off switch that _enables_ the charging-threshold logic. When Battery Care = `0x00`, the EC ignores the threshold register and charges to 100%. When = `0x01`, the threshold register (0xA7) is respected.

**How XPM implements it:** EC register `0xA4`, written via port I/O through EcIoSdk.dll → IoTDriver.sys (or direct EC port I/O at ports 0x62/0x66).

**MiControl status:** MiControl has charging threshold (40/50/60/70/80/100%) but NOT the Battery Care toggle. Without enabling 0xA4, the threshold may not take effect on some EC firmware versions.

**Technical details:**

- EC Register: `0xA4` (1 byte)
- Values: `0x00` = off (charge to 100%), `0x01` = on (respect threshold)
- Access: Via IoTService IPC EC command protocol (cmd_id for EC read/write)
- XPM method: `get_charging_protect`, `resume_charging_protect`
- XPM calls `SyncChargingProtect` on startup and after resume from sleep

**Implementation:**

```
1. Add EC write to register 0xA4 (0x01/0x00) alongside existing threshold write
2. Read back 0xA4 to confirm state
3. Persist last-known value in config for re-assertion after S3/S4 resume
4. Add Tauri command: set_battery_care(enabled: bool)
5. Add UI toggle in Battery tab
```

**Difficulty:** Easy
**Risk:** Low — standard EC register write, same path as existing threshold

---

### Gap 2: Battery Health Status & Original Info

**Priority:** 🟡 **MEDIUM** — Easy to implement, moderate user value

**What it is:**

- `get_battery_health_status` — Derived health classification (Good/Fair/Poor) from wear level
- `get_battery_original_info` — Factory/static battery data (manufacturer, chemistry, design capacity, manufacture date, serial number)

**How XPM implements it:** Via Windows WMI `root\WMI` namespace classes (no Xiaomi driver needed).

**Verified WMI data on this machine (TM2424):**

```
BatteryStaticData:
  DesignedCapacity: 68224 mWh (≈68Wh)
  DeviceName: BX70
  ManufactureName: COSMX
  SerialNumber: GYBX706418002793GMD1R100
  Chemistry: 1313818956 (maps to Li-ion)
  Technology: 1 (rechargeable)
  UniqueID: GYBX706418002793GMD1R100COSMXBX70

BatteryFullChargedCapacity:
  FullChargedCapacity: 70425 mWh (current max)

BatteryCycleCount:
  CycleCount: 0 (not supported by this EC)

BatteryStatus:
  RemainingCapacity: 69655 mWh
  Voltage: 17786 mV
  PowerOnline: True
  Charging: False
```

**Health calculation:** `FullChargedCapacity / DesignedCapacity × 100 = 70425 / 68224 × 100 = 103.2%` (battery is actually above design capacity, likely due to conservative design rating)

**MiControl status:** MiControl has battery level, charging, health, cycle count, capacity, voltage, charge rate. But it does NOT have:

- Manufacturer name (COSMX)
- Device name (BX70)
- Serial number
- Chemistry
- Design capacity vs full charged capacity comparison
- Derived health classification

**Implementation:**

```
1. Query root\WMI BatteryStaticData for original info
2. Query root\WMI BatteryFullChargedCapacity for current max
3. Compute wear %: (DesignedCapacity - FullChargedCapacity) / DesignedCapacity × 100
4. Map to health label: ≥80% Good, 60-80% Fair, <60% Poor
5. Add Tauri command: get_battery_original_info()
6. Display in Battery tab alongside existing data
```

**Difficulty:** Easy
**Risk:** None — pure WMI read, no driver needed

---

### Gap 3: Hyper Charging Support Detection

**Priority:** 🟢 **LOW** — Medium difficulty, low user value (informational only)

**What it is:** Xiaomi's term for >65W fast charging (100W/120W/140W GaN). The Book Pro 14 TM2424 ships with a 100W GaN adapter. `is_support_hyper_charging` is a capability probe, not a toggle.

**How XPM implements it:** Queries EC/charge-controller for negotiated input power and charger capability. Detection is by EC charger-status register and/or per-model capability flag.

**MiControl status:** MiControl has AC power status and charge rate via WMI but does NOT detect or display hyper charging support.

**Implementation:**

```
1. Hardcode TM2424 → 100W support (per-model constant)
2. Read charge rate from BatteryStatus.ChargeRate while plugged in
3. If sustained input > 65W (≈65000mW), display "Hyper charging active"
4. Add to Battery tab as informational badge
```

**Difficulty:** Medium (per-model table needed)
**Risk:** None — informational only

---

### Gap 4: Long Battery Life & Intelligent Mode

**Priority:** 🟢 **LOW** — Medium difficulty, overlaps with existing modes

**What it is:**

- **Long Battery Life (长续航):** Aggressive power conservation preset (caps CPU PL1/PL2, lowers brightness/refresh, may set lower charge ceiling)
- **Intelligent (智能):** Adaptive mode that auto-switches between performance and conservation based on load/AC-vs-battery

**How XPM implements it:** Additional EC/firmware preset IDs on supported SKUs (2024+ Xiaomi Book Pro). `is_support_longbatterylife_and_intelligent` is a capability probe.

**MiControl status:** MiControl has 11 performance modes including LongBattery(11) and SmartAdaptive(9), which cover these use cases. The XPM "Long Battery Life" and "Intelligent" modes overlap heavily with MiControl's existing Eco/Smart/LongBattery modes.

**Implementation:**

```
Option A: Treat as aliases for existing modes
  - Long Battery Life → LongBattery(11) + threshold 60% + brightness cap
  - Intelligent → SmartAdaptive(9)

Option B: If distinct EC preset IDs exist, add as new modes
  - Requires EC register verification on TM2424
```

**Difficulty:** Medium
**Risk:** Low — may be redundant with existing modes

---

### Gap 5: Eye Protection / Dynamic Eye Care

**Priority:** 🟡 **MEDIUM** — Medium difficulty, moderate user value

**What it is:** Xiaomi's low-blue-light / adaptive-color-temperature feature. "Dynamic" adjusts color temperature over time-of-day and ambient conditions using per-panel `.m3d` calibration files (Xiaomi-proprietary 3D LUT) plus downloadable ICC color profiles.

**How XPM implements it:**

- `.m3d` files: Proprietary per-panel 3D LUT calibration blobs (Protect00/01/99_SDR_EDO4503.m3d)
- ICC profiles: Downloaded from `https://icc-client.pc.mi.com/cli/query?sn=<serial>`
- Files: D65_P3_Cali.m3d, SRGB_Cali.m3d, P3_Cali.m3d
- Applied via `InstallIcc.exe` and Windows Color Management APIs
- XPM logs: `OnEyeProtectionModeNow : 0` (currently off)
- Registry: `HKLM\SOFTWARE\MI\DisplaySettings` with AiAdaptiveBrightness, AiBrightnessMin/Max/Sensitivity/Smoothing

**MiControl status:** MiControl has brightness, HDR, refresh rate, AI adaptive brightness, ambient light sensor. Does NOT have eye protection / blue light filter.

**Implementation (functional equivalent):**

```
Option A: Use SetDeviceGammaRamp for blue-light reduction
  - Adjust gamma ramp to warm colors (reduce blue channel)
  - Schedule based on time-of-day
  - No ICC profiles needed

Option B: Apply warm ICC profile via Windows Color Management
  - InstallColorProfile() + WcsSetDefaultColorProfile()
  - Create custom warm ICC profile

Option C: Toggle Windows Night Light
  - Registry: HKCU\Software\Microsoft\Windows\CurrentVersion\CloudStore
  - Or use SetDeviceGammaRamp as above

Note: Skip .m3d files (proprietary format, no public spec)
Note: Skip icc-client.pc.mi.com API (Xiaomi cloud, undocumented)
```

**Difficulty:** Medium (functional equivalent) / Very Hard (exact XPM replication)
**Risk:** Low — gamma ramp is reversible

---

### Gap 6: AI Noise Cancellation (Meeting Assistant)

**Priority:** 🟡 **MEDIUM** — Hard difficulty, high user value

**What it is:** Real-time mic noise suppression (`mic_nc`), speaker/far-end suppression (`spk_nc`), voice-focus/beamforming (`center`), and live subtitle transcription.

**How XPM implements it:**

- `LibAivsAdapter.dll` — Xiaomi AI Voice Service adapter (proprietary, signed)
- `SubtitleTranscriptor.dll` — On-device speech-to-text (proprietary)
- `LibAudioRecorder.dll` — Audio capture
- Meeting assistant settings: `mic_nc: 0, spk_nc: 0, center: 0, subtitle: 0, tray_visible: 1`
- Voice enrollment: `get_enroll_train_data` for personalized voice models

**MiControl status:** MiControl has audio device list, volume, mute, default endpoint. Does NOT have AI noise cancellation or subtitle transcription.

**Implementation (platform alternatives):**

```
Mic Noise Canceling:
  - Use Windows Studio Effects / Voice Clarity (if NPU available)
  - Or IAudioEffectsManager API for AEC + NS
  - Or third-party: RNNoise model via virtual mic

Speaker Noise Canceling:
  - Process render loopback through NS model

Subtitles:
  - Use Windows Live Captions API
  - Or Windows.Media.SpeechRecognition
  - Or Whisper model for on-device STT

Note: Do NOT load LibAivsAdapter.dll (proprietary, license risk)
```

**Difficulty:** Hard
**Risk:** Medium — audio processing can affect system audio stability

---

### Gap 7: NFC Tap-to-Pair

**Priority:** 🟢 **LOW** — Hard difficulty, low user value (requires Xiaomi phone)

**What it is:** Phone taps laptop NFC area → tag payload triggers Mi Share / screen mirror / file transfer. XPM writes pairing tag into NFC SRAM at address `0x10a800`.

**How XPM implements it:**

- `nfc_data_reader_writer.cpp` — NFC SRAM read/write via NCI driver
- NXP NFC controller (PN7160/PN7220-class)
- Tag data format: NDEF external type `com.xiaomi.mi_connect_service:externaltype`
- Payload: Protobuf `AttrAdvData` (deviceType=3 for PC, appIds=16378, MAC addresses, Lyra ability)
- XPM log: `ReadSRamBlock 10a800, result 01` / `ED_COFIG_REG, result 01`
- Tag data: `037dd42a50636f6d2e7869616f6d692e6d695f636f6e6e6563745f736572766963653a65787465726e616c747970650a4e0801100d2201032a094d492d4e4643544147380f4a34271700000003000e5441475f444953434f564552454465064d4952524f52011138303a31333a31363a33373a45313a34307901016a02fa7f`

**MiControl status:** Not implemented.

**Implementation:**

```
1. Use Windows.Networking.Proximity API for NFC access
2. Write NDEF tag with com.xiaomi.mi_connect_service format
3. Phone-side mi_connect_service handles the rest
4. Reference: github.com/XFY9326/MiLinkNFC

Note: Actual cross-device behavior still needs Xiaomi's phone-side services
Note: Low priority — requires Xiaomi phone with Mi Share
```

**Difficulty:** Hard
**Risk:** Low — NFC write is safe, but feature depends on phone-side software

---

### Gap 8: Distributed Camera (Phone as Webcam)

**Priority:** 🟢 **LOW** — Hard difficulty, alternatives exist

**What it is:** Use Xiaomi phone camera as PC webcam via `MiDistributedCameraBroker.exe`.

**How XPM implements it:**

- Part of Xiaomi HyperOS Cross-Device Camera 2.0
- Transported over Lyra interconnect (Wi-Fi Direct/P2P + BLE discovery)
- Proprietary protocol, not UVC-over-network
- XPM log: `distributed_camera.cpp` with file/command/P2P channel listeners
- `MiDistributedCameraBroker.exe` (64-bit) and `MiDistributedCameraBroker32.exe` (32-bit)
- `VirtualCameraSDK.dll` for virtual camera device

**MiControl status:** Not implemented.

**Implementation (alternatives):**

```
Option A: Android USB UVC (zero custom code)
  - Android 14+ phones can act as USB UVC webcam natively
  - Just plug in via USB and select as camera in apps

Option B: RTSP/WebRTC → virtual camera
  - Build RTSP receiver → DirectShow virtual camera
  - Use obs-virtualcam-style approach

Note: Do NOT try to replicate MiDistributedCameraBroker.exe (proprietary Lyra)
```

**Difficulty:** Hard (Xiaomi-native) / Easy (USB UVC alternative)
**Risk:** None — use standard alternatives

---

### Gap 9: OS Turbo

**Priority:** 🟡 **MEDIUM** — Medium difficulty, moderate user value

**What it is:** System-level optimization routine — memory/foreground-app prioritization, background throttling, startup trimming. Distinct from CPU Turbo performance mode (EC 0x68).

**How XPM implements it:**

- `os_turbo_module.cpp` — OS Turbo module
- XPM log: `[OST]Check first agress privacy or not` → `[OST]Send enable OST`
- Registry-based privacy agreement check before enabling
- OS-level scheduler/resource tweaks, not hardware power change

**MiControl status:** Not implemented. MiControl has performance modes (EC 0x68) but not OS-level optimization.

**Implementation:**

```
1. Use SetProcessInformation / PROCESS_POWER_THROTTLING for EcoQoS
2. Use PowerSetActiveScheme for power plan switching
3. Background app throttling via Task Manager API
4. Startup app management via registry/Task Scheduler
5. Compose as a software profile alongside existing performance modes
```

**Difficulty:** Medium
**Risk:** Low — software-only optimizations

---

### Gap 10: Cross-Device Interconnect (小米互联)

**Priority:** 🔴 **NOT RECOMMENDED** — Very Hard, proprietary framework

**What it is:** Magic Desktop, Mi Drop, App handoff, clipboard sync, notification sync, call relay, KM sharing, network sharing, TWS earbud switching, camera-as-webcam, AI file search — all over Lyra IPC framework.

**How XPM implements it:**

- **Lyra IPC framework** (pipe: `\\.\Pipe\pipe123.sock`)
- SDK: lyra 5.1.167.10 / lyra_rpc_1.1.25.1
- Transports: BLE (64), BT (1), IP Bonjour (2), IP P2P (16), IP SoftAP (32), NFC (4)
- Services: MediaResourceService, com.xiaomi.mirror:cast
- Authenticated by same Xiaomi account
- Protobuf-serialized messages over encrypted channels
- Components: MiMultiDeviceConnection, MiTelephone, MiDistributedAudio, MiDistributedFileServer, MiDropTransfer, MiSmartShare, MiTrustService, HandoffManager

**MiControl status:** Not implemented. MiControl has Miracast via WinRT but not Xiaomi's cross-device features.

**Implementation:**

```
NOT RECOMMENDED — Out of scope
- Lyra is proprietary, no public SDK
- Requires Xiaomi account authentication
- Requires signed Xiaomi binaries
- Use platform-neutral alternatives instead:
  - Clipboard sync: KDE Connect protocol (open, documented)
  - File transfer: SMB or custom LAN transfer
  - Notification sync: Windows Phone Link
  - Call relay: Windows Phone Link
```

**Difficulty:** Very Hard
**Risk:** High — proprietary protocol, cannot be replicated without Xiaomi binaries

---

### Gap 11: System Cleanup & Security Scan

**Priority:** 🟢 **LOW** — Medium difficulty, low value for MiControl's scope

**What it is:** Junk-file cleanup, cache/log/temp removal, startup optimization (CleanerEngine.dll), and malware/security scan (MiScanner.dll).

**How XPM implements it:**

- `CleanerEngine.dll` / `CleanerProxy.dll` — Cleanup engine
- `MiScanner.dll` — Security scanner
- `MiHygieneBroker.exe` — Hygiene broker service
- XPM log: `HygienePipeClient StartSession using_pc_host: 1; enable_junk_cleaner: 0; enable_system_boost: 0`

**MiControl status:** Not implemented.

**Implementation:**

```
Cleanup:
  - Known temp/cache paths (%TEMP%, browser caches, Windows Update cache)
  - Storage Sense API
  - Disk cleanup via CleanMgr API or direct file deletion

Security Scan:
  - Do NOT build AV engine
  - Shell out to Windows Defender via MpCmdRun.exe
  - Or use AMSI API for script scanning
```

**Difficulty:** Medium (cleanup) / Very Hard (AV — don't attempt)
**Risk:** Low (cleanup) / High (AV — don't attempt)

---

### Gap 12: Driver Management Details

**Priority:** 🟡 **MEDIUM** — Medium difficulty, moderate user value

**What it is:** `scan_drivers` / `get_drivers_detail` returning detailed driver info including driver_name, driver_size, driver_status, driver_type, hardware_id, current_version, latest_version, release_date, auto_exception_check, auto_restart, auto_update.

**How XPM implements it:**

- Local enumeration via Windows PnP/SetupAPI (SetupDiGetClassDevs, PnPUtil)
- Xiaomi cloud driver-catalog API for "latest_version" and "release_date"
- XPM log shows driver data with fields: `driver_name`, `driver_size`, `driver_status`, `driver_type` (HR, IGC, ISH), `hardware_id`, `current_version`, `latest_version`, `release_date`

**MiControl status:** MiControl has basic driver management (install, scan, XPM detection) but NOT detailed driver info or version comparison.

**Implementation:**

```
1. Use Win32_PnPSignedDriver WMI class for local driver enumeration
2. Use SetupAPI (SetupDiGetClassDevs) for detailed driver info
3. Use PnPUtil for driver store management
4. For update checking: point to Xiaomi's official driver page or Windows Update
5. Do NOT reverse-engineer Xiaomi's cloud catalog API
```

**Difficulty:** Medium
**Risk:** Low — standard Windows APIs

---

### Gap 13: Function Key Customization

**Priority:** 🟡 **MEDIUM** — Medium difficulty, moderate user value

**What it is:** `get_function_key` reads/sets Fn-key behavior — Fn-lock (F1-F12 vs multimedia), and dedicated hotkey behavior.

**How XPM implements it:**

- WMI event GUIDs for Fn key events (confirmed by Linux `xiaomi-wmi.c` driver)
- Fn-lock state is an EC/BIOS setting
- XPM queries this via SvrCModule

**MiControl status:** MiControl has hotkey handling (AI key, Xiaomi key, Copilot key, Fn+F4/F7/F8/F9/F10) but NOT Fn-lock toggle or function key customization.

**Implementation:**

```
1. Read Fn-lock state from EC register (needs verification on TM2424)
2. Toggle Fn-lock via EC write
3. For key remapping: intercept WMI event GUIDs and launch custom handlers
4. Reference: Linux xiaomi-wmi.c for WMI GUIDs
```

**Difficulty:** Medium
**Risk:** Low — EC register needs verification

---

### Gap 14: Replacement Assistant

**Priority:** 🟢 **LOW** — Not a hardware feature

**What it is:** A guided after-sales helper tool (`download_/install_/open_replacement_assistant`). This is Xiaomi's device-migration / data-transfer assistant ("换机助手"), not a hardware feature.

**MiControl status:** Not applicable.

**Implementation:** N/A — treat as a link/launcher to Xiaomi's official tool if needed.

---

### Gap 15: Abnormal Restart Environment Recovery

**Priority:** 🟢 **LOW** — Easy, improves robustness

**What it is:** `get_abnormal_restart_environment_recovery` + `get_application_anomaly_monitoring_and_repair` — watchdog features that detect crash/unexpected shutdown and restore XPM's runtime state.

**MiControl status:** Not implemented.

**Implementation:**

```
1. Use RegisterApplicationRestart (Windows Restart Manager)
2. Use WER (WerRegister* / LocalDumps) for crash dumps
3. Boot-persistence check: registry "last clean exit" flag
4. Detect abnormal termination and re-init state on next launch
```

**Difficulty:** Easy-Medium
**Risk:** None — standard Windows crash handling

---

## 4. XPM Display Color Calibration Pipeline

### ICC/M3D Color Profile System

XPM implements a sophisticated display color calibration pipeline:

**Components:**

- `Color_Verification.dll` — Color verification
- `InstallIcc.exe` — ICC profile installer
- `set_screen.cpp` — Screen settings module
- `dynamic_eye_operator.cpp` — Dynamic eye protection operator
- `relative_software_module.cpp` — Relative software module

**Calibration Files (.m3d):**

- `Protect00_SDR_EDO4503.m3d` — Eye protection level 0 (off)
- `Protect01_SDR_EDO4503.m3d` — Eye protection level 1 (low)
- `Protect99_SDR_EDO4503.m3d` — Eye protection level 99 (max)
- `D65_P3_Cali.m3d` — D65 white point, P3 color gamut calibration
- `SRGB_Cali.m3d` — sRGB calibration
- `P3_Cali.m3d` — P3 gamut calibration

**Cloud API:**

- URL: `https://icc-client.pc.mi.com/cli/query?sn=<serial_number>`
- Downloads panel-specific calibration files
- XPM log: `Get icc/m3d update url is https://icc-client.pc.mi.com/cli/query?sn=77079/26RV00757`
- XPM log: `Download Success file = C:\ProgramData\Timi Personal Computing\MiService\icc\D65_P3_Cali.m3d`

**Registry Settings:**

```
HKLM\SOFTWARE\MI\DisplaySettings:
  AiAdaptiveBrightness: 0x1 (enabled)
  AiBrightnessMin: 0x5 (5%)
  AiBrightnessMax: 0x64 (100%)
  AiBrightnessSensitivity: 0xaa (170)
  AiBrightnessSmoothing: 0xa (10)
```

**MiControl status:** MiControl has AI adaptive brightness but NOT the ICC color calibration pipeline.

---

## 5. XPM AI Service Architecture

### AIService.exe

- Runs as a Windows service (`AIService.exe /Service`)
- Path: `C:\Program Files\MI\AIService\2.0.1.572\AIService.exe`
- AppId: 41
- Monitors AIBroker process and restarts if needed
- Handles WTS_SESSION_LOGON events (login/logout)
- Handles SERVICE_CONTROL_POWEREVENT (power state changes)

### AIBroker

- AI Engine pipe server
- ShortCutKeyManager for AI Search Bar (Ctrl+Ctrl double-tap)
- Registers raw input devices for global hotkey capture
- Monitors power broadcast messages

### AI Search

- SQLite database: `ai_database1.db` (117MB)
- Search engine logs (multiple 10MB rotating logs)
- `AISearchSDK.dll` / `aisearchservice.dll`
- `XiaomiAISearch.exe` — AI search UI

### Meeting Assistant

- Settings: `mic_nc` (mic noise canceling), `spk_nc` (speaker noise canceling), `center` (voice focus), `subtitle` (live captions), `tray_visible`
- Voice enrollment: `get_enroll_train_data` for personalized voice models
- `LibAivsAdapter.dll` — AI Voice Service adapter
- `SubtitleTranscriptor.dll` — Real-time subtitle transcription

---

## 6. XPM Cross-Device Architecture (Lyra)

### Lyra IPC Framework

**Version:** lyra 5.1.167.10 / lyra_rpc_1.1.25.1.2026012913

**Transport:** Named pipe `\\.\Pipe\pipe123.sock`

**Components:**

- `micont_service.exe` — Micont service (Lyra node)
- `miexpress.dll` — Express transport
- `micont_rtm.dll` — Real-time messaging
- `lyra_rpc.dll` — RPC protocol
- `lyra_dist_data_srv.dll` — Distributed data server
- `lyra_dist_file_srv.dll` — Distributed file server
- `lyra_dist_file_sys.dll` — Distributed file system
- `lyra_dist_fs_service.dll` — DistFS service
- `lyra_dist_srv_sdk.dll` — Distributed server SDK

**Services:**

- `MediaResourceService` — Media resource sharing
- `com.xiaomi.mirror:cast` — Screen mirror/cast
- `MiDistCameraClient` — Distributed camera
- `MiDistributedAudio` — Distributed audio
- `MiDistributedFileServer` — Distributed file server
- `MiTelephone` — Call relay
- `MiMultiDeviceConnection` — Multi-device manager
- `MiDropService` — File transfer
- `MiSmartShare` — Smart share with NFC
- `TWS` — True Wireless Stereo earbud management
- `IDMWrapper` — Inter-Device Management (screen share)
- `HandoffManager` — App handoff
- `MiTrustService` — Device trust/pairing

**Discovery Transports (AppDiscTypeEnum):**

| Value | Transport             |
| ----- | --------------------- |
| 1     | Bluetooth (BT)        |
| 2     | IP Bonjour (mDNS)     |
| 4     | NFC                   |
| 16    | IP P2P (Wi-Fi Direct) |
| 32    | IP SoftAP             |
| 64    | BLE                   |

---

## 7. XPM Registry Configuration

### Complete Registry Map

```
HKLM\SOFTWARE\MI\
├── AIService\Action\SystemSetting
│   ├── action: C:\Program Files\MI\AIService\2.0.1.572\system_settings_ddf.json
│   ├── name: SystemSetting
│   └── path: C:\Program Files\MI\AIService\2.0.1.572
│
├── Config\AIService
│   ├── Name: AIService
│   ├── AppId: 41
│   ├── LogPath: C:\ProgramData\MI\AIService\log;%localappdata%\Packages\8497DDF3.639A2791C9AB_kf545nqv09rxe\LocalState
│   └── DumpPath: (empty)
│
├── DisplaySettings
│   ├── AiAdaptiveBrightness: 0x1
│   ├── AiBrightnessMin: 0x5
│   ├── AiBrightnessMax: 0x64
│   ├── AiBrightnessSensitivity: 0xaa
│   └── AiBrightnessSmoothing: 0xa
│
├── DistFSService
│   ├── RtmEnableState: 0x0
│   └── RtmDisablingError: 0x0
│
├── IoTDriver
│   └── ChargingThreshold: 0x64 (100%)
│
├── MiDeviceService
│   └── ProductModel: TM2424
│
├── MiScenarioRecognition
│   ├── CloudControl: 1
│   └── UserControl: 1
│
├── PerformanceMode
│   ├── (Default): (empty)
│   └── LastLongBattery: 0x9
│
├── Touchpad
│   ├── HapticsEnabled: 0x1
│   ├── EdgeSlide: 0x1
│   └── TrackpadRepress: 0x1
│
└── Update\XiaomiPCManager
    ├── Name: XiaomiPCManager
    ├── InstallResult: success
    ├── AfterVersion: 5.8.0.57
    └── BeforeVersion: 5.8.0.48
```

---

## 8. Priority Implementation Recommendations

### Tier 1: High Value, Easy (Do First)

| #   | Feature                          | Difficulty | EC/WMI       | Value                               |
| --- | -------------------------------- | ---------- | ------------ | ----------------------------------- |
| 1   | Battery Care toggle (EC 0xA4)    | Easy       | EC 0xA4      | High — completes charging threshold |
| 2   | Battery original info & health   | Easy       | WMI root\WMI | Medium — useful battery diagnostics |
| 3   | Crash recovery (Restart Manager) | Easy       | Win32 API    | Low — improves app robustness       |

### Tier 2: Medium Value, Medium Difficulty

| #   | Feature                            | Difficulty | Approach           | Value                       |
| --- | ---------------------------------- | ---------- | ------------------ | --------------------------- |
| 4   | Eye protection (blue light filter) | Medium     | SetDeviceGammaRamp | Medium — user comfort       |
| 5   | OS Turbo (system optimization)     | Medium     | Process throttling | Medium — performance        |
| 6   | Hyper charging detection           | Medium     | WMI charge rate    | Low — informational         |
| 7   | Function key customization         | Medium     | EC register + WMI  | Medium — input flexibility  |
| 8   | Driver management details          | Medium     | SetupAPI + WMI     | Medium — system maintenance |
| 9   | Long battery life mode             | Medium     | Composite profile  | Low — overlaps existing     |

### Tier 3: High Value, Hard (Consider Later)

| #   | Feature                | Difficulty | Approach               | Value                  |
| --- | ---------------------- | ---------- | ---------------------- | ---------------------- |
| 10  | AI noise cancellation  | Hard       | Windows Studio Effects | High — meeting quality |
| 11  | Subtitle transcription | Hard       | Windows Live Captions  | Medium — accessibility |

### Tier 4: Not Recommended (Proprietary/Out of Scope)

| #   | Feature                          | Difficulty | Reason                                       |
| --- | -------------------------------- | ---------- | -------------------------------------------- |
| 12  | Cross-device interconnect (Lyra) | Very Hard  | Proprietary framework, no public SDK         |
| 13  | NFC tap-to-pair                  | Hard       | Requires Xiaomi phone + proprietary services |
| 14  | Distributed camera               | Hard       | Use Android USB UVC instead                  |
| 15  | System cleanup                   | Medium     | Out of scope for hardware control app        |
| 16  | Security scan                    | Very Hard  | Use Windows Defender instead                 |
| 17  | ICC color calibration            | Very Hard  | Proprietary .m3d format + cloud API          |
| 18  | Replacement assistant            | N/A        | Not a hardware feature                       |

---

## 9. Technical Implementation Details

### EC Register Access Path

MiControl already has EC access via IoTService IPC (16 EC cmd_ids). The key registers for gap features:

| Register | Purpose                | Access                  | XPM Method             |
| -------- | ---------------------- | ----------------------- | ---------------------- |
| 0x68     | Performance mode       | ✅ Already in MiControl | get_workLoad_mode      |
| 0xA4     | Battery Care toggle    | ❌ Missing              | get_charging_protect   |
| 0xA7     | Charge threshold value | ✅ Already in MiControl | get_charging_threshold |

**Note:** EC register 0xA4 is the master enable for the threshold logic at 0xA7. Without 0xA4 = 0x01, the threshold at 0xA7 may be ignored by the EC on some firmware versions.

### WMI Classes Available on TM2424

| WMI Class                   | Namespace | Purpose                           | MiControl Uses? |
| --------------------------- | --------- | --------------------------------- | --------------- |
| BatteryStaticData           | root\WMI  | Original battery info             | ❌ No           |
| BatteryFullChargedCapacity  | root\WMI  | Current max capacity              | ❌ No           |
| BatteryCycleCount           | root\WMI  | Cycle count (returns 0 on TM2424) | ✅ Yes          |
| BatteryStatus               | root\WMI  | Live battery status               | ✅ Yes          |
| MICommonInterface           | root\WMI  | WMAA ACPI method                  | ✅ Yes          |
| HQWmiCommonInterface        | root\WMI  | BIOS control                      | ✅ Yes          |
| EsifDeviceInformation       | root\WMI  | ESIF device info                  | ❌ No           |
| HID_EVENT20-23              | root\WMI  | HID events                        | ❌ No           |
| WmiMonitorBrightness        | root\WMI  | Monitor brightness                | ✅ Yes          |
| WmiMonitorBrightnessMethods | root\WMI  | Brightness methods                | ✅ Yes          |

### XPM Process List (from Uninstaller Log)

All processes XPM managed during uninstall:

| Process                   | Purpose                  |
| ------------------------- | ------------------------ |
| XiaomiPcManager.exe       | Main UI                  |
| OSDUtility.exe            | On-screen display        |
| SubtitleTranscription.exe | Real-time subtitles      |
| MiSmartShareDevice.exe    | Cross-device smart share |
| MiPlayCastService.exe     | Screen cast service      |
| MiScreenShareGuide.exe    | Screen share guide       |
| MiPCAudio.exe             | PC audio service         |
| PcyybAssistant.exe        | PC assistant             |
| AndrowsInstaller.exe      | Android app installer    |
| micont_service.exe        | Lyra micont service      |
| DistributedService.exe    | Distributed service      |
| dist_service.exe          | Distribution service     |
| handoff_svc.exe           | Handoff service          |
| XiaomiAISearch.exe        | AI search                |
| XiaomiPcHost.exe          | Background host          |
| MAFSvr.exe                | MAF server               |
| MiHygieneBroker.exe       | System hygiene broker    |

---

## 10. Risk Assessment

### EC Register Write Risks

- **Writing wrong values to EC registers can hang the embedded controller** (requires full power-drain to recover)
- Always read-modify-write and validate
- Verify exact register map on TM2424 with RWeverything before shipping
- Xiaomi moves registers across EC firmware revisions

### IoTDriver.sys Coexistence

- If XPM is reinstalled, two writers to the same EC registers will fight (values overwritten by XPM's poller)
- Detect XPM's process and defer, or document mutual exclusivity
- XPM's `SyncChargingProtect` runs on startup and after resume

### Proprietary Binary Risks

- Loading `LibAivsAdapter.dll`, `CleanerEngine.dll`, Lyra DLLs has license/integrity risk
- These binaries phone-home to Xiaomi account services
- Avoid; use OS equivalents instead

### WMI BatteryCycleCount

- Frequently reports 0 on Xiaomi ECs (confirmed on TM2424)
- Don't hard-depend on it for health calculation
- Use wear-based health (FullChargedCapacity / DesignedCapacity) instead

---

## 11. Summary Statistics

| Metric                           | Count                |
| -------------------------------- | -------------------- |
| Total XPM SvrCModule API methods | 67                   |
| Methods MiControl already has    | 15                   |
| Methods MiControl is missing     | 52                   |
| Feature gaps identified          | 15                   |
| Easy to implement                | 3                    |
| Medium difficulty                | 6                    |
| Hard difficulty                  | 4                    |
| Very Hard / Not recommended      | 5+                   |
| XPM binaries (non-system)        | ~200+                |
| XPM services/components          | 25+                  |
| XPM cross-device services        | 13                   |
| WMI classes available            | 15+                  |
| EC registers mapped              | 3 (0x68, 0xA4, 0xA7) |

---

## References

1. XPM SvrCModule log: `C:\ProgramData\MI\SvrCModule\log\svrc_log.txt` (704KB, 67 API methods)
2. XPM AIoT logs: `C:\ProgramData\MI\AIoT\Log\` (Lyra, MiDrop, MiTelephone, MiDistCamera, etc.)
3. XPM uninstaller log: `C:\ProgramData\MI\AIoT\Log\unstaller.log.txt` (866KB, complete binary list)
4. XPM AIService log: `C:\ProgramData\MI\AIService\log\` (AI broker, AI service, application side action)
5. XPM smart share log: `C:\ProgramData\MI\AIoT\Log\smart_share_log.txt` (1.4MB, NFC, handoff, firewall)
6. XPM MiService log: `C:\ProgramData\MI\MiService\miservice.log` (49KB, service events)
7. XPM OSD log: `C:\ProgramData\MI\OSDLauncher\OSDLauncher.log` (1.5KB, OSD launches)
8. XPM registry: `HKLM\SOFTWARE\MI\` (complete configuration)
9. Linux Xiaomi WMI driver: `drivers/platform/x86/xiaomi-wmi.c` (Fn key event GUIDs)
10. Xiaomi-CoreCharge project: `github.com/alex-bogatiuk/Xiaomi-CoreCharge` (EC register confirmation)
11. Xiaomi HyperOS NFC protocol: `juejin.cn/post/7301951271232864265` (NFC tag format)
12. MiLinkNFC project: `github.com/XFY9326/MiLinkNFC` (NFC tool reference)
13. Android USB UVC: `source.android.com/docs/core/camera/webcam` (phone-as-webcam alternative)
14. Windows Battery WMI: `learn.microsoft.com/en-us/windows/win32/api/batclass/ns-batclass-battery_wmi_static_data`

---

_This report was generated by forensic analysis of XPM v5.8.0.57 residual logs and registry data on a Xiaomi Book Pro 14 (TM2424), combined with internet research on Xiaomi hardware interfaces. All EC register values should be verified on-device before implementation._
