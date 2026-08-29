# Sprint 38 — IoT Hardware Opportunities (Phases 1–4)

> **Date:** 2026-08-29
> **Sprint:** 38
> **Theme:** Unlock the underused Xiaomi IoT hardware — events/OSD, presence, cross-device, deep integration
> **Duration:** ~2–3 weeks
> **Dependencies:** Sprint 37 (P1 audio/wifi), Sprint 36 (stability)
> **Status:** ✅ Complete
> **Branch:** `feature/IoT-hardware`
> **Audit Reference:** `docs/IOT_HARDWARE_OPPORTUNITIES_REPORT.md`, `docs/CROSS_DEVICE_ALTERNATIVES_REPORT.md`, `docs/HARDWARE_GAP_ANALYSIS.md`

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **OBRIGATÓRIO: 100% dos tickets desta sprint devem ser concluídos. A sprint não será aceita como entregue se qualquer ticket permanecer incompleto.**
>
> **MANDATORY: 100% of the tickets in this sprint MUST be completed. The sprint will NOT be accepted as delivered if any ticket remains incomplete.**

## Health Check (must pass before every commit — `node scripts/health-check.mjs`)

```bash
node scripts/health-check.mjs          # version, fmt, clippy(-D warnings, --features face),
                                       # test-rust(--features face), tsc, lint, format, test-front
                                       # (vitest --coverage), build, i18n, toolchain
```

## Executive Summary

This sprint implements the full IoT-hardware opportunities plan derived from
`docs/IOT_HARDWARE_OPPORTUNITIES_REPORT.md`, across 4 phases:

- **P1 — Quick wins:** consume IoT power/EC events → native OSD notifications,
  scenario-based auto performance mode, an IoT pipe watchdog, and a battery WMI
  timeout fix (crash-related).
- **P2 — Presence & beacon:** BLE (btleplug) phone proximity detection →
  auto lock/unlock, with telemetry.
- **P3 — Cross-device:** LocalSend protocol (files), scrcpy orchestration
  (phone as camera), sherpa-onnx transcription (100% local).
- **P4 — Deep integration:** KDE Connect (discovery + plugins), NFC tap-to-pair
  groundwork, Syncthing REST orchestration.

**Hard rule:** every module is a NEW isolated file under `src-tauri/src/hw/`
(plus commands in a new `commands/` module or registered handlers) — existing
working features must not regress. Minimal, surgical edits to shared wiring
(`lib.rs` setup, `hw/mod.rs`, `commands` registration, i18n) only.

## Goals

| #   | Goal                                                      | KPI                                            |
| --- | --------------------------------------------------------- | ---------------------------------------------- |
| 1   | IoT power/EC events produce native OSD + optional actions | `iot_events.rs` + listener wired in lib.rs     |
| 2   | Auto perf-mode on AC/lid scenario (configurable)          | `scenario_rules.rs` + commands                 |
| 3   | IoT pipe self-heals without user action                   | `iot_watchdog.rs` background task              |
| 4   | Battery WMI never blocks > ~5 s (crash mitigation)        | battery.rs timeout + retry + cache             |
| 5   | BLE proximity detections and auto lock/unlock             | `ble_presence.rs` (btleplug)                   |
| 6   | P2P file transfer via LocalSend protocol on LAN           | `localsend.rs` client+server (no new big deps) |
| 7   | Phone-as-camera via scrcpy orchestration                  | `scrcpy_bridge.rs`                             |
| 8   | Local transcription via sherpa-onnx CLI                   | `transcription.rs`                             |
| 9   | KDE Connect discovery + ping/clipboard MVP                | `kde_connect.rs`                               |
| 10  | NFC tap-to-pair groundwork (NDEF + guidance)              | `nfc_pairing.rs`                               |
| 11  | Syncthing REST orchestration                              | `syncthing.rs` + commands                      |
| 12  | Frontend + i18n for all new toggles/status                | 4 locale files complete (i18n gate)            |

## Wave / Dependency Map

```
Wave P1-A (seq): MIOT-01 iot_events.rs ──> MIOT-02 (scenario_rules + battery) ──> MIOT-03 (watchdog) ──> MIOT-FRONT
   (shares lib.rs startup wiring → sequential)
Wave P2-A (seq): MIOT-04 btleplug dep + ble_presence.rs ──> MIOT-05 UI + i18n
Wave P3-A (seq): MIOT-06 localsend ──> MIOT-07 scrcpy ──> MIOT-08 sherpa ──> MIOT-09 frontend trio
Wave P4-A (seq): MIOT-10 kde_connect ──> MIOT-11 nfc_pairing ──> MIOT-12 syncthing ──> MIOT-13 frontend
```

Each module is isolated: new files only; shared edits limited to `hw/mod.rs`,
`lib.rs` (startup + invoke handler), i18n JSON, and one new frontend section.

## Technical Specs

### MIOT-01: IoT Events → OSD + actions listener

**Files:** `src-tauri/src/hw/iot_events.rs` (NEW), `src-tauri/src/hw/mod.rs`, `src-tauri/src/lib.rs`
**Problem:** The chip emits power/EC events (`IotEvent`) but nothing consumes them to
notify the user or drive actions.
**Solution:**

- `iot_events.rs`: a background `tokio::spawn` loop that:
  1. Surveys `power_listener` WM_POWERBROADCAST (via a shared registry callback) AND
     polls AC/battery state deltas (throttled) to synthesize `PowerEvent`s.
  2. Maps events → `osd::show_*` calls (AC plug → OSD + Windows toast), EC events → OSD.
  3. Exposes `register_power_callback(f: Box<dyn Fn(PowerEvent) + Send + Sync>)` and
     `start_iot_event_listener()`.
- Register listener + callback in `lib.rs` setup; callback checks scenario rules
  (MIOT-02) and OSD config (registry `SOFTWARE\MiControl\IotEvents`).
  **Acceptance:** cargo check; unit test for event mapping; OSD function reused.

### MIOT-02: Scenario auto perf-mode + battery WMI timeout

**Files:** `src-tauri/src/hw/scenario_rules.rs` (NEW), `src-tauri/src/hw/battery.rs` (edit)
**Problem:** No automatic perf-mode switching by AC/lid scenario; battery WMI may block ~15 s.
**Solution:**

- `scenario_rules.rs`: registry-backed rules (`Enabled`, `OnAcMode`, `OnBatteryMode`,
  `LidClosedAction`) applied when a power event arrives; calls
  `performance::set_performance_mode`.
- `battery.rs`: wrap `wmi_cache::with_wmi` blocking calls with a 5 s timeout
  (spawn + `recv_timeout`); on timeout return cached battery info and log; keep
  `AC_PROBE_MIN_INTERVAL` cache.
  **Acceptance:** cargo check; battery returns ≤ ~5 s under timeout; rules module tested.

### MIOT-03: IoT pipe watchdog

**Files:** `src-tauri/src/hw/iot_watchdog.rs` (NEW), `src-tauri/src/lib.rs`
**Problem:** If `IoTService` / `ecram_service` pipe disappears (crash, update), features
silently break and never recover.
**Solution:** background task with bounded cooldown: if `is_pipe_available()` false →
call `ecram_service_mgmt::ensure_service_running()`; log transitions; heartbeat metric.
**Acceptance:** cargo check; watch loop unit-simulated.

### MIOT-04: BLE phone presence (btleplug)

**Files:** `src-tauri/Cargo.toml` (add `btleplug`), `src-tauri/src/hw/ble_presence.rs` (NEW)
**Problem:** No way to know when the user is near/far to auto lock/unlock.
**Solution:** `ble_presence.rs`: scan for the paired phone by MAC/name via btleplug
WinRT backend; maintain presence state + RSSI; expose `start_presence_monitor()` and
callbacks to lock (`LockWorkStation`) / unlock hint on threshold crossing. Config in
registry (MAC allowlist, radius dBm, enable). Telemetry (last RSSI/latency) returned.
**Acceptance:** cargo check (Windows); unit tests on threshold logic.

### MIOT-05: P2 frontend + i18n

**Files:** `src/pages/tabs/crossdevice.tsx` (edit), `src/i18n/{pt,en,es,fr}.json`
**Solution:** presence section (enable, phone picker, threshold, status) calling new
commands `get_presence_status`, `set_presence_config`.
**Acceptance:** tsc, eslint, prettier, vitest, i18n gates pass.

### MIOT-06: LocalSend protocol (files, LAN)

**Files:** `src-tauri/src/hw/localsend.rs` (NEW), `src-tauri/src/hw/mod.rs`
**Solution:** Rust implementation of LocalSend v2.2: UDP multicast discovery
(224.0.0.167:53317) + HTTP (plain) register/prepare/upload endpoints on 53317 via
tokio; supports sending files to discovered peers and receiving to Downloads.
**Acceptance:** cargo check; discovery unit tests with mock peers; commands exposed.

### MIOT-07: scrcpy orchestration

**Files:** `src-tauri/src/hw/scrcpy_bridge.rs` (NEW)
**Solution:** detect `scrcpy` binary (PATH + winget hint); spawn `scrcpy --camera-facing=front`
etc. for phone-as-camera; manage process (start/stop/status) via std::process + pid file.
**Acceptance:** cargo check; status command returns not-installed gracefully.

### MIOT-08: Local transcription via sherpa-onnx CLI

**Files:** `src-tauri/src/hw/transcription.rs` (NEW)
**Solution:** orchestrate `sherpa-onnx` CLI (detect/download binary + int8 model to
app-data), run on a wav file, return text; Tauri command `transcribe_audio`.
No heavy Rust ASR dependency → zero build risk.
**Acceptance:** cargo check; command returns friendly error when binary absent; tests on arg builder.

### MIOT-09: P3 frontend + i18n (files, camera, transcription)

**Files:** `src/pages/tabs/crossdevice.tsx` (edit), `src/components/*` (NEW small),
`src/i18n/{pt,en,es,fr}.json`
**Solution:** send-file UI, scrcpy camera toggle, transcription file picker.
**Acceptance:** all frontend gates pass.

### MIOT-10: KDE Connect discovery + plugins MVP

**Files:** `src-tauri/src/hw/kde_connect.rs` (NEW)
**Solution:** UDP multicast KDE Connect identity packets (port 1716); TCP JSON
handshake + Ping plugin (non-TLS MVP, isolated behind config); clipboard plugin
stub. Document TLS limitation in module docs.
**Acceptance:** cargo check; discovery parsing tests.

### MIOT-11: NFC tap-to-pair groundwork

**Files:** `src-tauri/src/hw/nfc_pairing.rs` (NEW)
**Solution:** NDEF record builder (pairing handshake payload) + guidance UI to pair
Xiaomi 14T over NFC; deep-link to Phone Link when applicable.
**Acceptance:** cargo check; NDEF encode/decode round-trip test.

### MIOT-12: Syncthing REST orchestration

**Files:** `src-tauri/src/hw/syncthing.rs` (NEW)
**Solution:** reqwest client for Syncthing REST (system/status, config/folders,
events); commands `get_syncthing_status`, `set_syncthing_folder`.
**Acceptance:** cargo check; URL building tests.

### MIOT-13: P4 frontend + i18n

**Files:** `src/pages/tabs/crossdevice.tsx` (edit), `src/i18n/{pt,en,es,fr}.json`
**Solution:** NFC pairing info, Syncthing status/folder panel, KDE Connect toggle.
**Acceptance:** all frontend gates pass.

## Story Points

| Ticket  | Points | Owner   |
| ------- | ------ | ------- |
| MIOT-01 | 3      | copilot |
| MIOT-02 | 3      | copilot |
| MIOT-03 | 2      | copilot |
| MIOT-04 | 5      | copilot |
| MIOT-05 | 2      | copilot |
| MIOT-06 | 5      | copilot |
| MIOT-07 | 2      | copilot |
| MIOT-08 | 3      | copilot |
| MIOT-09 | 2      | copilot |
| MIOT-10 | 3      | copilot |
| MIOT-11 | 2      | copilot |
| MIOT-12 | 2      | copilot |
| MIOT-13 | 2      | copilot |

## Execution Summary (2026-08-29)

All 13 tickets completed on `feature/IoT-hardware` (base `9f61cc1`). Every
commit gated by the 11-step health check (`EXIT=0`) plus per-ticket Rust gates
(fmt → check → clippy `-D warnings` → targeted unit tests). No version bump,
no push to remote.

| Commit    | Ticket  | Summary                                                   |
| --------- | ------- | --------------------------------------------------------- |
| `8f7df42` | docs    | IoT hardware opportunities report + sprint plan           |
| `ba1318d` | MIOT-01 | IoT events → native OSD + actions listener                |
| `ee1b7cf` | MIOT-02 | Scenario auto perf-mode + battery WMI timeout             |
| `ca24b21` | MIOT-03 | IoT pipe watchdog with service recovery                   |
| `5ca8686` | MIOT-04 | BLE phone presence monitor (btleplug)                     |
| `9a11ae0` | MIOT-05 | Presence UI + i18n                                        |
| `326e604` | MIOT-06 | LocalSend P2P file transfer protocol¹                     |
| `61914e3` | MIOT-07 | scrcpy phone-camera orchestration                         |
| `9c8b68b` | MIOT-08 | On-device transcription via sherpa-onnx CLI               |
| `9168764` | MIOT-09 | Cross-device P3 UI (files/camera/transcription) + i18n    |
| `2da87da` | MIOT-09 | Fix: transcription/scrcpy status fields → serde camelCase |
| `1700c1f` | MIOT-10 | KDE Connect discovery + ping MVP (non-TLS, documented)    |
| `59c0c99` | MIOT-11 | NFC tap-to-pair groundwork (NDEF handshake + guidance)    |
| `30c359b` | MIOT-12 | Syncthing REST client (status/folders/toggle)             |
| `009472e` | MIOT-13 | Cross-device P4 UI (KDE/NFC/Syncthing) + i18n             |

¹ `localsend.rs`: 9 unit tests (info parse, filename sanitize, discovery…) —
total suite across new modules: 9 (MIOT-06) + 3 (MIOT-07) + 4 (MIOT-08) +
6 (MIOT-10) + 6 (MIOT-11) + 6 (MIOT-12) = 34 new tests, all passing.

Every module is isolated in `src-tauri/src/hw/<module>.rs`; shared edits limited
to `hw/mod.rs`, `commands/crossdevice.rs`, `lib.rs` invoke handler, i18n files,
and `src/pages/tabs/crossdevice.tsx`. Schemas under `src-tauri/gen/schemas/`
regenerated by cargo builds were reverted before each commit (never committed).

## Commits (one per module/wave, conventional)

1. `feat(miot-01): iot events to native OSD + listener`
2. `feat(miot-02): scenario auto perf-mode + battery wmi timeout`
3. `feat(miot-03): iot pipe watchdog with service recovery`
4. `feat(miot-04): ble phone presence monitor (btleplug)`
5. `feat(miot-05): presence UI + i18n`
6. `feat(miot-06): localsend p2p file transfer protocol`
7. `feat(miot-07): scrcpy phone-camera orchestration`
8. `feat(miot-08): local audio transcription via sherpa-onnx`
9. `feat(miot-09): cross-device UI (files/camera/transcription) + i18n`
10. `feat(miot-10): kde connect discovery + ping plugin`
11. `feat(miot-11): nfc tap-to-pair groundwork`
12. `feat(miot-12): syncthing rest orchestration`
13. `feat(miot-13): deep-integration UI + i18n`

## What Was Deferred

| Item                              | Reason                                             | Next Action  |
| --------------------------------- | -------------------------------------------------- | ------------ |
| KDE Connect TLS encryption        | MVP over plain TCP; TLS needs cert mgmt            | P5           |
| BLE advertising (peripheral mode) | Windows WinRT limitation; chip advertises natively | P5           |
| Real sentry HTTP collector config | Requires user-chosen endpoint                      | after sprint |
