# Face Unlock Module for miControl — Implementation Plan

> **Date:** 2026-08-02
> **Status:** Approved — implementation in progress
> **Reference:** Analysis of `everglow01/Windows-Face-Hello` (v1.0.6, reference), `caochitam/windows-face-unlock`, `zs1083339604/FaceWinUnlock-Tauri`

## 1. Goal

Make the **XiaoMi WebCam (RGB, no IR)** do face unlock on the Windows lock screen,
implemented **natively in Rust inside miControl** — no external binaries, no Python.
A Windows Credential Provider (Rust DLL) shows a "Face Unlock" tile on LogonUI;
a LocalSystem service (Rust) captures the camera, runs liveness + face recognition
via ONNX models, and the CP submits the Windows credential via LSA.

**Explicitly NOT requested / out of scope:** making Windows Hello itself accept the
webcam (impossible — no IR sensor). This is a _Windows Hello-style_ credential provider,
the same approach all 3 researched projects use.

## 2. Architecture

```
┌─ Lock screen (LogonUI.exe, SYSTEM session) ────────────────────────────┐
│  micontrol_facecp.dll   (Rust cdylib, windows crate COM)               │
│   ICredentialProvider / ICredentialProviderCredential                  │
│    • 1 tile: avatar + label + [→] submit + status text                 │
│    • SetSelected → auth_start via pipe → poll auth_poll (~400ms)       │
│    • GetSerialization → LSA read password → KERB_INTERACTIVE_UNLOCK    │
│    • 3 attempts then fall back to password/PIN (system tile remains)   │
└───────────────┬────────────────────────────────────────────────────────┘
                │ named pipe \\.\pipe\micontrol_face   (JSON, msg-mode)
                │ DACL: SYSTEM + Administrators; FILE_FLAG_FIRST_PIPE_INSTANCE
                │ CP verifies server SID == LocalSystem before sending
                ▼
┌─ micontrol_face_svc.exe  (Rust, LocalSystem Windows service, boot auto)┐
│  • Camera: OpenCV-DSHOW equivalent via `opencv` crate (index 0, retry) │
│  • ORT (onnxruntime) CPU inference:                                    │
│      det_10g.onnx (SCRFD 320×320) → faces + 5 kps                       │
│      w600k_r50.onnx (ArcFace 512-d, FP16) → embedding                  │
│      face_landmarker.task (MediaPipe 468-pt) → EAR blink + pose        │
│      antispoof.onnx (MiniFASNet 80×80) + RetinaFace crop → anti-spoof  │
│  • Auth state machine: liveness → antispoof → recognize → margin check │
│  • Store: DPAPI machine-scope encrypted gallery (features only)        │
│  • Lockout: 5 biometric fails / 30 s; EcoQoS escape + ABOVE_NORMAL     │
└─────────────────────────────────────────────────────────────────────────┘
   ▲ pipe (same, client role for enrollment & admin)        │
┌─ miControl Tauri app (user session, elevated for admin) ─┘────────────┐
│  • New "Face Unlock" tab: enroll (camera preview, quality guide),     │
│    manage templates, settings (threshold, liveness, lockout),         │
│    diagnostics (service health, camera probe, model status)           │
│  • Writes LSA Secret L$FaceHello_<user> (admin), DPAPI gallery        │
└────────────────────────────────────────────────────────────────────────┘
```

### Security model (from facehello, the hardened reference)

- **Password never crosses the pipe.** The service returns only `{ok, user, similarity}`.
  The CP reads the password from the LSA Secret itself (SYSTEM context).
- **LSA Secret** `L$FaceHello_<user>` (UTF-16LE). Write = admin console; read = SYSTEM CP.
- **Gallery** = feature vectors (512-d), DPAPI `CRYPTPROTECT_LOCAL_MACHINE` + entropy.
- **Pipe DACL** SYSTEM + Administrators; `FILE_FLAG_FIRST_PIPE_INSTANCE` anti-squatting.
- **Server identity check** by CP: `GetNamedPipeServerProcessId` → token → `WinLocalSystemSid`.
- **Lockout**: service-side 5 fails/30s (biometric only) + CP 3 attempts → password.
- **Anti-spoof**: passive MiniFASNet multi-frame + active liveness (blink/turn challenge).

## 3. New crates needed (in `src-tauri/Cargo.toml`)

```toml
# ── face unlock ──
ort = { version = "2", features = ["download-binaries"] }        # onnxruntime CPU
opencv = { version = "0.95", features = ["dshow"] }              # camera + preprocessing
mediapipe-rs = { version = "0.12", features = ["tasks-vision"] } # face_landmarker.task
windows-service = "0.7"                                          # LocalSystem service
```

> Note: `opencv` crate requires C++ toolchain + `opencv_world` DLL at runtime. Alternative:
> use `ort` for everything (SCRFD + ArcFace + MiniFASNet) and a raw camera path. Fallback
> considered in §7.

## 4. Binary layout (new `[[bin]]` entries)

| Binary                   | Purpose                                               | Runs as                        |
| ------------------------ | ----------------------------------------------------- | ------------------------------ |
| `micontrol_face_svc.exe` | LocalSystem auth service (camera + ORT + pipe server) | SYSTEM service `MiControlFace` |
| `micontrol_facecp.dll`   | Credential Provider (cdylib, COM)                     | LogonUI                        |

Built by `tauri.conf.json` resources: `bin/micontrol_facecp.dll`, `bin/micontrol_face_svc.exe`
(+ models in `resources/face_models/`).

## 5. Implementation phases

### Phase A — Recognition core (pure Rust, testable, no OS deps)

- `src/hw/face/mod.rs` — module skeleton, config, errors
- `src/hw/face/models.rs` — ONNX model loading (ort), SCRFD pre/post, ArcFace embed
- `src/hw/face/liveness.rs` — MediaPipe landmark → EAR blink + solvePnP head pose
- `src/hw/face/antispoof.rs` — MiniFASNet + RetinaFace crop
- `src/hw/face/matcher.rs` — cosine + margin (port of `best_match_with_margin`)
- `src/hw/face/store.rs` — DPAPI machine-scope gallery (features only, versioned)
- `src/hw/face/credvault.rs` — LSA Secret read/write (admin write, SYSTEM read)
- Unit tests with synthetic embeddings (no camera/models needed)

### Phase B — Auth service

- `src/bin/micontrol_face_svc.rs` — windows-service loop, camera open/retry,
  `_boost_cpu_scheduling` equivalent, pipe server (DACL SYSTEM+Admins, FIRST_PIPE_INSTANCE),
  JSON protocol `ping` / `auth_start` / `auth_poll`, lockout state machine, warmup
- `src/hw/face/service.rs` — the auth pipeline (liveness → antispoof → recognize → margin)

### Phase C — Credential Provider (Rust DLL)

- `src/bin/micontrol_facecp/` (cdylib):
  - `lib.rs` — DllMain, DllGetClassObject, DllCanUnloadNow, CLSID registry
  - `provider.rs` — ICredentialProvider (SetUsageScenario, GetCredentialCount/At, Advise/UnAdvise)
  - `credential.rs` — ICredentialProviderCredential (SetSelected, GetSerialization,
    Kerb pack via LsaConnectUntrusted + LsaLookupAuthenticationPackage)
  - `pipe_client.rs` — named pipe client + server-SID verification
  - `credvault_reader.rs` — LSA Secret read (SYSTEM)
- Registers: `HKCR\CLSID\{GUID}` + `HKLM\...\Credential Providers\{GUID}`

### Phase D — Enrollment + frontend

- Tauri commands: `face_enroll`, `face_list_templates`, `face_delete_template`,
  `face_set_password`, `face_get_settings`, `face_set_settings`,
  `face_service_install/start/stop/status`, `face_diagnostics`
- `src/pages/tabs/face.tsx` — camera preview (via Tauri webview? or native helper),
  quality-guided enrollment, template list, settings, service control, diagnostics
- Installer-hooks: register `MiControlFace` service + `micontrol_facecp.dll` CP,
  ship models

### Phase E — Installer integration

- `installer-hooks.nsi`: copy CP DLL to System32 (or register via regsvr32),
  `micontrol_face_svc.exe install`, models to `resources/face_models/`
- Uninstall: remove CP registration, stop/delete service, delete gallery (keep option)

## 6. Models (downloaded separately, ~100 MB total)

| File                                          | Source                       | Size    | Purpose                  |
| --------------------------------------------- | ---------------------------- | ------- | ------------------------ |
| `det_10g.onnx`                                | InsightFace buffalo_l        | ~34 MB  | SCRFD detection 320×320  |
| `w600k_r50.onnx`                              | InsightFace buffalo_l (FP16) | ~87 MB  | ArcFace 512-d            |
| `face_landmarker.task`                        | MediaPipe                    | ~3.7 MB | 468 landmarks (liveness) |
| `antispoof.onnx`                              | Silent-Face MiniFASNet V2    | ~1.7 MB | passive anti-spoof       |
| `antispoof_detector.caffemodel` + `.prototxt` | RetinaFace                   | ~1.8 MB | crop for antispoof       |

## 7. Known risks & mitigations

| Risk                                     | Mitigation                                                                                                                                                                                                                                                                                             |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Session-0 camera access** (hardest)    | facehello proved DSHOW works from SYSTEM with retry/backoff. We add `scripts/cam_session0_probe` equivalent + diagnostics before enabling. If the XiaoMi WebCam fails in session 0, fall back to **user-session service** (windows-face-unlock pattern: Task Scheduler @logon, service survives lock). |
| `opencv` crate needs C++ toolchain + DLL | Prefer pure-`ort` path; use `opencv` only for camera capture (or raw `MSMF` via `windows` crate `Win32::Media::MediaFoundation`).                                                                                                                                                                      |
| MediaPipe Tasks in Rust                  | `mediapipe-rs` supports `FaceLandmarker` task files. Fallback: EAR/pose from 5 kps (SCRFD) — weaker but no extra model.                                                                                                                                                                                |
| EcoQoS throttling in session 0           | `SetProcessInformation(ProcessPowerThrottling)` + `ABOVE_NORMAL` (facehello does this, measured 3.5× speedup).                                                                                                                                                                                         |
| Camera privacy settings                  | SYSTEM is not subject to per-user camera privacy toggle (facehello confirmed, zero registry hacks).                                                                                                                                                                                                    |
| Account lockout (wrong password)         | UI warns: password ≠ PIN. Only write LSA password after explicit user confirmation; keep 3-attempt CP limit.                                                                                                                                                                                           |
| Webcam occupied                          | Single camera lock; release on WTS_SESSION_LOCK (facewinunlock's fix).                                                                                                                                                                                                                                 |

## 8. Testing strategy

- Unit: matcher (cosine/margin), store (DPAPI roundtrip), liveness math (EAR), pipe protocol (mock)
- Integration: service ↔ CP over real pipe (mock auth = fake face), camera probe on hardware
- E2E: lock screen tile on real hardware (manual, with restore point first)
- CI: `cargo test` for core; models not in CI (skip inference tests)

## 9. Deliverables

1. Working Rust face-recognition core with unit tests
2. LocalSystem auth service (`micontrol_face_svc.exe`) — camera + pipe + ORT
3. Credential Provider DLL (`micontrol_facecp.dll`) — lock-screen tile
4. miControl "Face Unlock" tab (enroll, manage, settings, diagnostics)
5. Installer integration (service + CP registration + models)
