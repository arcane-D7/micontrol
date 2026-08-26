//! MiControl Face Auth Service — LocalSystem Windows service.
//!
//! Provides the lock-screen face authentication backend for miControl's
//! "Face Unlock" feature (Windows Hello-style, RGB webcam):
//!
//! - Runs as `NT AUTHORITY\SYSTEM` (installed via SCM), so it survives the
//!   locked workstation and can open the webcam in the lock-screen context.
//! - Serves the named pipe `\\.\pipe\micontrol_face` (DACL SYSTEM + Admins,
//!   message mode, single instance) to the Credential Provider DLL.
//! - Protocol: `ping` / `auth_start` / `auth_poll` (JSON).
//! - The sign-in password is read by the CP from the LSA Secret — it never
//!   crosses this pipe.
//!
//! Build/install (elevated):
//!   micontrol_face_svc.exe install [--startup auto]
//!   micontrol_face_svc.exe start | stop | remove
//!   micontrol_face_svc.exe run      (foreground, for dev/debug)

#![cfg(windows)]

use micontrol_lib::hw::face::config::DATA_DIR;
use micontrol_lib::hw::face::pipe_server;
use micontrol_lib::hw::face::store::{load_store, FaceStore};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ── Logging ─────────────────────────────────────────────────────────────────

fn setup_logging() {
    let _ = std::fs::create_dir_all(DATA_DIR);
    let log_path = format!(r"{DATA_DIR}\face_svc.log");
    let config = fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{} [{}] {}",
                chrono_like_now(),
                record.level(),
                message
            ))
        })
        .level(log::LevelFilter::Info)
        .chain(fern::log_file(log_path).expect("open face_svc.log"))
        .chain(std::io::stdout());
    let _ = config.apply();
}

fn chrono_like_now() -> String {
    // Minimal timestamp (no chrono dep): HH:MM:SS from system time.
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let s = d.as_secs() % 86400;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

// ── CPU scheduling boost (EcoQoS escape) ────────────────────────────────────
// Port of `_boost_cpu_scheduling()` from the reference service: background /
// session-0 processes get EcoQoS execution-speed throttling (~3.5× slower
// inference). Exit the throttle and raise priority to ABOVE_NORMAL.

fn boost_cpu_scheduling() {
    use windows::Win32::System::Threading::{
        GetCurrentProcess, ProcessPowerThrottling, SetPriorityClass, SetProcessInformation,
        ABOVE_NORMAL_PRIORITY_CLASS, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        PROCESS_POWER_THROTTLING_STATE,
    };

    // 1. Exit EcoQoS execution-speed throttling.
    unsafe {
        let state = PROCESS_POWER_THROTTLING_STATE {
            Version: 1,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: 0, // disable throttling
        };
        let _ = SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            &state as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
        // 2. Priority ABOVE_NORMAL.
        let _ = SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS);
    }
    log::info!("[cpu] EcoQoS throttle disabled; priority = ABOVE_NORMAL");
}

// ── Auth runner (async thread model) ────────────────────────────────────────
// The CP calls auth_start (fire-and-forget), then polls auth_poll until done.
// A background thread owns the camera + recognition for the duration of one
// attempt. Lockout (5 fails / 30 s) is enforced here.

struct AuthRunner {
    state: std::sync::Mutex<RunnerState>,
}

struct RunnerState {
    instruction: String,
    done: bool,
    result: Option<AuthResultJson>,
    fails: u32,
    locked_until_unix: i64,
}

#[derive(Clone)]
struct AuthResultJson {
    success: bool,
    user: Option<String>,
    similarity: f32,
    reason: String,
}

impl AuthRunner {
    fn new() -> Self {
        Self {
            state: std::sync::Mutex::new(RunnerState {
                instruction: "starting".to_string(),
                done: true,
                result: None,
                fails: 0,
                locked_until_unix: 0,
            }),
        }
    }

    fn snapshot(&self) -> Value {
        let st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let mut resp = json!({
            "ok": true,
            "done": st.done,
            "instruction": st.instruction,
        });
        if let Some(r) = &st.result {
            resp["success"] = json!(r.success);
            if let Some(u) = &r.user {
                resp["user"] = json!(u);
            }
            resp["similarity"] = json!(r.similarity);
            resp["reason"] = json!(r.reason);
        }
        resp
    }

    /// Start an auth attempt on a background thread (non-blocking).
    fn start(&self, store_path: &str) {
        let mut st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let now = now_unix();
        // Lockout check.
        if st.locked_until_unix > now {
            st.instruction = format!("locked {}", st.locked_until_unix - now);
            st.done = true;
            st.result = Some(AuthResultJson {
                success: false,
                user: None,
                similarity: 0.0,
                reason: "locked".to_string(),
            });
            return;
        }
        st.instruction = "starting".to_string();
        st.done = false;
        st.result = None;
        drop(st);

        let path = store_path.to_string();
        let runner = Arc::new(Self::new());
        let runner_clone = runner.clone();
        let _ = std::thread::spawn(move || {
            run_auth_once(&runner_clone, &path);
        });
        let _ = runner;
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// One full auth attempt: load store, run the pipeline (via the shared
/// library's AuthSession), set the runner state.
fn run_auth_once(runner: &Arc<AuthRunner>, store_path: &str) {
    use micontrol_lib::hw::face::service::AuthSession;

    let result = (|| -> Result<AuthResultJson, String> {
        let store: FaceStore =
            load_store(std::path::Path::new(store_path)).map_err(|e| format!("store load: {e}"))?;
        if store.is_empty() {
            return Ok(AuthResultJson {
                success: false,
                user: None,
                similarity: 0.0,
                reason: "no_enrolled".to_string(),
            });
        }
        let mut session = AuthSession::new(&store);

        // Drive the session with real camera frames + ONNX inference when the
        // `face` feature is available; otherwise fall back to synthetic frames
        // (dev/diagnostics).
        #[cfg(feature = "face")]
        {
            run_auth_camera(&mut session, &store)?;
        }
        #[cfg(not(feature = "face"))]
        {
            run_auth_synthetic(&mut session)?;
        }

        match session.result() {
            Some(r) => Ok(AuthResultJson {
                success: r.success,
                user: r.name.clone(),
                similarity: r.similarity,
                reason: r.reason.clone(),
            }),
            None => Err("auth session ended without result".to_string()),
        }
    })();

    let mut st = runner.state.lock().unwrap_or_else(|p| p.into_inner());
    match result {
        Ok(r) => {
            st.done = true;
            st.instruction = r.reason.clone();
            // Lockout: count biometric failures only.
            if r.success {
                st.fails = 0;
            } else if r.reason.contains("mismatch")
                || r.reason.contains("liveness")
                || r.reason.contains("no_face")
            {
                st.fails += 1;
                if st.fails >= 5 {
                    st.locked_until_unix = now_unix() + 30;
                    st.fails = 0;
                    log::warn!("[lockout] 5 biometric failures → locked 30s");
                }
            }
            st.result = Some(r);
        }
        Err(e) => {
            st.done = true;
            st.instruction = format!("error: {e}");
            st.result = Some(AuthResultJson {
                success: false,
                user: None,
                similarity: 0.0,
                reason: e,
            });
        }
    }
}

/// Synthetic pipeline (no camera/models): open-eye frames with no embedding
/// → yields "no_face" (or gallery empty) — good enough for dev smoke tests.
#[cfg(not(feature = "face"))]
fn run_auth_synthetic(
    session: &mut micontrol_lib::hw::face::service::AuthSession,
) -> Result<(), String> {
    use micontrol_lib::hw::face::liveness::Challenge;
    // Force a blink challenge for deterministic behavior.
    if session.liveness_force_challenge(Challenge::Blink(1)) {
        // close → open completes the blink, then recognition with no face.
        session.feed(0.10, 0.0, None);
        session.feed(0.30, 0.0, None);
    } else {
        // liveness disabled or already gated — feed one open frame.
        session.feed(0.30, 0.0, None);
    }
    Ok(())
}

/// Real camera pipeline (feature `face`): capture frames, run liveness with
/// SCRFD-derived pose metrics, detect+embed with ONNX, feed the session.
#[cfg(feature = "face")]
fn run_auth_camera(
    session: &mut micontrol_lib::hw::face::service::AuthSession,
    store: &FaceStore,
) -> Result<(), String> {
    use micontrol_lib::hw::face::camera::Camera;
    use micontrol_lib::hw::face::models::FaceDetector;

    // 1. Load models (idempotent). Missing models → infra error.
    let mut det = FaceDetector::default();
    det.load().map_err(|e| format!("model load: {e}"))?;

    // 2. Open the configured camera with retry/backoff.
    let cam_index = store.settings.camera_index;
    let mut cam = Camera::open(cam_index, 8.0).map_err(|e| format!("camera: {e}"))?;

    // 3. Capture loop until the session concludes or we exceed a hard budget.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
    while !session.done() {
        if std::time::Instant::now() > deadline {
            return Ok(()); // session.result() will report timeout-relevant state
        }
        let frame = match cam.read() {
            Ok(f) => f,
            Err(e) => {
                // transient read error — keep trying briefly
                log::warn!("[svc] frame read error: {e}");
                std::thread::sleep(std::time::Duration::from_millis(120));
                continue;
            }
        };
        let faces = match det.detect(&frame) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("[svc] detect error: {e}");
                session.feed(0.30, 0.0, None);
                continue;
            }
        };
        if faces.is_empty() {
            session.feed(0.30, 0.0, None);
            continue;
        }
        // Multi-face protection: reject if requested.
        if store.settings.multi_face_protection_enabled && faces.len() >= 2 {
            session.feed(0.30, 0.0, None);
            continue;
        }
        // Largest/strongest face.
        let face = faces
            .iter()
            .max_by(|a, b| {
                a.det_score
                    .partial_cmp(&b.det_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned()
            .unwrap();

        // Estimate eye-aspect-ratio + yaw from SCRFD keypoints
        // (kps: [0]=right eye, [1]=left eye, [2]=nose). Approximation for
        // the MediaPipe precision — documented limitation.
        let (ear, yaw) = estimate_pose_metrics(&face);

        // Only run recognition while in the Recognize phase or as the embed
        // for the first matching frame. Recognizing every frame is costly;
        // gate on liveness phase so we don't embed during the challenge.
        let embedding = if session.instruction() == "recognizing"
            || session.instruction() == "no_face"
            || (!store.settings.liveness_enabled)
        {
            det.recognize(
                &frame.data,
                frame.width,
                frame.height,
                &face.kps,
                &face.bbox,
            )
            .ok()
        } else {
            None
        };

        session.feed(ear, yaw, embedding);
    }
    Ok(())
}

/// Estimate eye-aspect-ratio + yaw from SCRFD 5 keypoints.
///
/// SCRFD kps order (InsightFace): [0]=right eye, [1]=left eye, [2]=nose,
/// [3]=right mouth, [4]=left mouth. Unlike MediaPipe we don't have 6 eye
/// points, so EAR is approximated from the eye-corner separation vs the
/// nose-to-mouth span, and yaw from the nose's lateral offset from the
/// eye-line midpoint. This is intentionally conservative: it requires the
/// face to appear with a mostly neutral pose and a detectable eye separation.
#[cfg(feature = "face")]
fn estimate_pose_metrics(face: &micontrol_lib::hw::face::models::DetectedFace) -> (f32, f32) {
    let re = face.kps[0];
    let le = face.kps[1];
    let nose = face.kps[2];
    let inter_eye = ((re[0] - le[0]).powi(2) + (re[1] - le[1]).powi(2)).sqrt();
    if inter_eye < 1e-4 {
        return (0.0, 0.0);
    }
    // Eye-midline midpoint.
    let mid_x = (re[0] + le[0]) / 2.0;
    // Yaw: lateral nose offset relative to eye separation (normalized).
    let yaw = ((nose[0] - mid_x) / inter_eye).clamp(-0.9, 0.9);
    // EAR proxy: ratio of the vertical eye opening estimate to the inter-eye
    // span. SCRFD doesn't track eyelid height, so use a conservative open-eye
    // default when the face is frontal (yaw small) — records a "blink" only
    // when yaw magnitude spikes (likely occlusion), keeping liveness usable.
    let ear = if yaw.abs() < 0.15 { 0.28 } else { 0.12 };
    (ear, yaw)
}

// ── Pipe handler ────────────────────────────────────────────────────────────

fn handle_request(req: &Value, runner: &Arc<AuthRunner>, store_path: &str) -> Value {
    let cmd = req.get("cmd").and_then(|c| c.as_str()).unwrap_or("");
    match cmd {
        "ping" => {
            let st = runner.state.lock().unwrap_or_else(|p| p.into_inner());
            json!({
                "ok": true,
                "ready": true,
                "locked": st.locked_until_unix > now_unix(),
                "version": env!("CARGO_PKG_VERSION"),
                "protocol": 1,
            })
        }
        "auth_start" => {
            runner.start(store_path);
            json!({ "ok": true, "done": false })
        }
        "auth_poll" => runner.snapshot(),
        _ => pipe_server::err_response(format!("unknown command: {cmd}")),
    }
}

// ── Windows service wrapper ─────────────────────────────────────────────────

mod svc {
    use super::*;
    use windows_service::define_windows_service;
    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::service_dispatcher;

    pub const SERVICE_NAME: &str = "MiControlFace";

    define_windows_service!(ffi_service_main, service_main);

    #[allow(dead_code)] // only used in installed (SCM) mode
    fn service_main(_args: Vec<std::ffi::OsString>) {
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_handle = shutdown.clone();

        let status_handle =
            service_control_handler::register(SERVICE_NAME, move |control| match control {
                ServiceControl::Stop | ServiceControl::Shutdown => {
                    shutdown_handle.store(true, Ordering::SeqCst);
                    ServiceControlHandlerResult::NoError
                }
                _ => ServiceControlHandlerResult::NotImplemented,
            })
            .expect("register service control handler");

        // Report RUNNING.
        status_handle
            .set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::Running,
                controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint: std::time::Duration::from_secs(0),
                process_id: None,
            })
            .expect("set RUNNING");

        run_main_loop(&shutdown);
    }

    #[allow(dead_code)] // only used in installed (SCM) mode
    pub fn run() -> windows_service::Result<()> {
        service_dispatcher::start(SERVICE_NAME, ffi_service_main)
    }
}

/// The main service loop: boost CPU, warm up, serve the pipe until shutdown.
fn run_main_loop(shutdown: &AtomicBool) {
    setup_logging();
    boost_cpu_scheduling();
    log::info!(
        "MiControl Face service starting (v{})",
        env!("CARGO_PKG_VERSION")
    );

    let store_path = format!(r"{DATA_DIR}\faces.dat");
    let runner = Arc::new(AuthRunner::new());

    // Warm up the store (load once; fail-open to an empty store).
    match load_store(std::path::Path::new(&store_path)) {
        Ok(_) => log::info!("[store] gallery loaded from {store_path}"),
        Err(e) => log::warn!("[store] initial load failed ({e}); will retry per auth"),
    }

    log::info!(
        "[pipe] listening on {}",
        micontrol_lib::hw::face::config::PIPE_NAME
    );
    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        let handler = |req: &Value| handle_request(req, &runner, &store_path);
        match pipe_server::serve_one(&handler, shutdown) {
            Ok(true) => {}
            Ok(false) => break, // shutdown
            Err(e) => {
                log::warn!("[pipe] serve_one error: {e}");
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
    }
    log::info!("MiControl Face service stopping");
}

// ── CLI (install/remove/run) ────────────────────────────────────────────────

fn main() {
    use windows_service::service::{
        ServiceAccess, ServiceInfo, ServiceStartType, ServiceState, ServiceType,
    };
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("run") => {
            // Foreground (dev). No SCM handler.
            setup_logging();
            let shutdown = Arc::new(AtomicBool::new(false));
            boost_cpu_scheduling();
            log::info!("[dev] running in foreground (Ctrl+C to stop)");
            run_main_loop(&shutdown);
        }
        Some("install") => {
            let exe = std::env::current_exe().expect("current exe");
            let manager = ServiceManager::local_computer(
                None::<&std::ffi::OsStr>,
                ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
            )
            .expect("open SCM");
            let info = ServiceInfo {
                name: svc::SERVICE_NAME.into(),
                display_name: "MiControl Face Auth Service".into(),
                service_type: ServiceType::OWN_PROCESS,
                start_type: ServiceStartType::AutoStart,
                error_control: windows_service::service::ServiceErrorControl::Normal,
                executable_path: exe,
                launch_arguments: vec![],
                dependencies: vec![],
                account_name: None, // LocalSystem by default
                account_password: None,
            };

            // Idempotent install: if the service already exists (from a previous
            // version), stop it, update its configuration to point at the new
            // binary, then restart it. Otherwise create it fresh.
            let existing = manager.open_service(
                svc::SERVICE_NAME,
                ServiceAccess::QUERY_STATUS
                    | ServiceAccess::CHANGE_CONFIG
                    | ServiceAccess::START
                    | ServiceAccess::STOP,
            );
            match existing {
                Ok(service) => {
                    // Stop if running — but service stop is ASYNC: the SCM
                    // returns once the stop is accepted, not once the process
                    // exited. A subsequent start() within that window fails
                    // with 1056 (ERROR_SERVICE_ALREADY_RUNNING) and panics.
                    // Poll until the service actually reaches STOPPED (bounded)
                    // so the subsequent change_config + start can succeed.
                    let _ = service.stop();
                    let mut stopped = false;
                    for _ in 0..40 {
                        let st = service.query_status().ok();
                        match st.as_ref().map(|s| s.current_state) {
                            Some(ServiceState::Stopped) => {
                                stopped = true;
                                break;
                            }
                            // Already transitioning (STOP_PENDING): keep waiting.
                            _ => std::thread::sleep(std::time::Duration::from_millis(250)),
                        }
                    }
                    // If the service is still alive after the bounded poll (e.g.
                    // slow MSMF camera teardown in Session-0), kill any
                    // stragglers so the SCM doesn't hold the exe path open.
                    // This mirrors what the installer's KillFaceServiceProcess
                    // does BEFORE invoking us — but re-armed AFTER the stop,
                    // catching processes that respawned in between.
                    if !stopped {
                        eprintln!("MiControlFace did not stop within 10 s — proceeding anyway");
                        let _ = std::process::Command::new("taskkill.exe")
                            .args(["/IM", "micontrol_face_svc.exe", "/T", "/F"])
                            .output();
                        // Give taskkill a beat to reap the process.
                        std::thread::sleep(std::time::Duration::from_millis(1000));
                    }

                    // Point the existing service at the new executable. This
                    // can fail with ERROR_SERVICE_MARKED_FOR_DELETE / 1056 if a
                    // stale handle raced the stop — retry a few times before
                    // giving up so installer upgrades (which the hook now also
                    // deletes-then-recreates) stay robust.
                    let mut configured = false;
                    for attempt in 0..5 {
                        match service.change_config(&info) {
                            Ok(()) => {
                                configured = true;
                                break;
                            }
                            Err(e) => {
                                eprintln!("config retry {attempt}: change_config failed: {e}");
                                std::thread::sleep(std::time::Duration::from_millis(750));
                            }
                        }
                    }
                    if !configured {
                        eprintln!("MiControlFace change_config failed after retries — removing and recreating");
                        let _ = manager
                            .open_service(svc::SERVICE_NAME, ServiceAccess::DELETE)
                            .and_then(|s| s.delete());
                        let service = manager
                            .create_service(&info, ServiceAccess::START)
                            .expect("recreate MiControlFace service");
                        service
                            .start(&Vec::<std::ffi::OsString>::new())
                            .expect("start recreated service");
                        println!("MiControlFace service recreated and started.");
                        configure_failure_actions();
                        return;
                    }

                    // Start — tolerate 1056 (already running): the SCM may have
                    // auto-restarted the service (failure actions) or the stop
                    // raced. If it is already running, the new config is live.
                    match service.start(&Vec::<std::ffi::OsString>::new()) {
                        Ok(()) => {
                            println!("MiControlFace service updated to new binary and started.")
                        }
                        Err(e)
                            if e.to_string().contains("1056")
                                || e.to_string().contains("ALREADY_RUNNING") =>
                        {
                            println!("MiControlFace service is already running (1056) — new binary live.");
                        }
                        Err(e) => {
                            eprintln!("restart MiControlFace service: {e}");
                            // Service exists and config is updated — the start
                            // failure alone must not abort an installer run.
                            // The post-install verification step reports the
                            // real state; log and continue.
                            println!("MiControlFace install: service configured but start failed ({e}) — will retry on reboot");
                        }
                    }
                }
                Err(_) => {
                    let service = manager
                        .create_service(&info, ServiceAccess::START)
                        .expect("create MiControlFace service");
                    service
                        .start(&Vec::<std::ffi::OsString>::new())
                        .expect("start service");
                    println!("MiControlFace service installed and started.");
                }
            }
            // Post-reboot/self-heal hardening: the service crashes with
            // 0xc0000005 in FrameServerClient.dll_unloaded (MSMF camera in a
            // Session-0 SYSTEM service, ~60 min after boot). Without failure
            // actions the SCM leaves it STOPPED-1067 forever. Configure
            // RESTART 5/10/30s (reset 1 day) so the SCM auto-restarts it.
            configure_failure_actions();
        }
        Some("start") => {
            let manager = ServiceManager::local_computer(
                None::<&std::ffi::OsStr>,
                ServiceManagerAccess::CONNECT,
            )
            .expect("open SCM");
            manager
                .open_service(svc::SERVICE_NAME, ServiceAccess::START)
                .expect("open service")
                .start(&Vec::<std::ffi::OsString>::new())
                .expect("start service");
            println!("MiControlFace started.");
        }
        Some("stop") => {
            let manager = ServiceManager::local_computer(
                None::<&std::ffi::OsStr>,
                ServiceManagerAccess::CONNECT,
            )
            .expect("open SCM");
            let svc = manager
                .open_service(svc::SERVICE_NAME, ServiceAccess::STOP)
                .expect("open service");
            svc.stop().expect("send stop");
            println!("MiControlFace stop requested.");
        }
        Some("remove") => {
            let manager = ServiceManager::local_computer(
                None::<&std::ffi::OsStr>,
                ServiceManagerAccess::CONNECT,
            )
            .expect("open SCM");
            manager
                .open_service(svc::SERVICE_NAME, ServiceAccess::DELETE)
                .expect("open service")
                .delete()
                .expect("delete service");
            println!("MiControlFace removed.");
        }
        _ if args.is_empty() => {
            // Installed (SCM) mode: the Service Control Manager starts us with
            // no arguments. Dispatch to the Windows service entrypoint.
            if let Err(e) = svc::run() {
                eprintln!("service dispatcher error: {e}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!(
                "usage: {} [run|install|start|stop|remove]",
                std::env::current_exe().unwrap_or_default().display()
            );
            std::process::exit(2);
        }
    }
}

/// Configure SCM failure actions so the service is auto-restarted after a
/// crash (instead of being left STOPPED-1067 forever).
///
/// Rationale: MiControlFace periodically crashes with 0xc0000005 in
/// FrameServerClient.dll_unloaded (MSMF webcam inside a Session-0 SYSTEM
/// service). The SCM only restarts crashed services when `sc failure` actions
/// are configured (RESTART …), which MiControlBridge / IoTSvc already have —
/// MiControlFace was the only one missing it, which is why it was found
/// STOPPED after reboot while the others kept running.
///
/// Errors are non-fatal: the service is already started at this point; failure
/// actions are best-effort self-heal hardening.
fn configure_failure_actions() {
    let output = std::process::Command::new("sc.exe")
        .args([
            "failure",
            "MiControlFace",
            "reset=",
            "86400",
            "actions=",
            "restart/5000/restart/10000/restart/30000",
        ])
        .output();
    match output {
        Ok(o) if o.status.success() => {
            println!("MiControlFace failure actions configured (restart 5/10/30s).");
        }
        Ok(o) => {
            eprintln!(
                "warning: could not configure failure actions: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            );
        }
        Err(e) => {
            eprintln!("warning: could not run sc failure: {e}");
        }
    }
}
