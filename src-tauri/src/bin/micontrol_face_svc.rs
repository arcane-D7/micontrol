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

        // Drive the session with synthetic frames when the `face` feature
        // (camera + ORT) is unavailable — used for dev/diagnostics and to
        // keep the pipeline testable. With the feature on, real camera frames
        // + embeddings feed the session (see run_auth_camera in Phase B).
        #[cfg(feature = "face")]
        {
            let _ = run_auth_camera(&mut session, store_path)?;
        }
        #[cfg(not(feature = "face"))]
        {
            let _ = run_auth_synthetic(&mut session)?;
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
/// real landmarks, detect+embed, feed the session. Phase B implementation.
#[cfg(feature = "face")]
fn run_auth_camera(
    session: &mut micontrol_lib::hw::face::service::AuthSession,
    _store_path: &str,
) -> Result<(), String> {
    // Placeholder until the ORT + camera wiring lands (Phase B).
    let _ = session;
    Err("camera pipeline not yet wired (face feature build)".to_string())
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
    use windows_service::service::{ServiceAccess, ServiceInfo, ServiceStartType, ServiceType};
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
            let service = manager
                .create_service(&info, ServiceAccess::START)
                .expect("create MiControlFace service");
            service
                .start(&Vec::<std::ffi::OsString>::new())
                .expect("start service");
            println!("MiControlFace service installed and started.");
        }
        Some("start") => {
            let manager = ServiceManager::local_computer(
                None::<&std::ffi::OsStr>,
                ServiceManagerAccess::CONNECT,
            )
            .expect("open SCM");
            manager
                .open_service(&svc::SERVICE_NAME, ServiceAccess::START)
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
                .open_service(&svc::SERVICE_NAME, ServiceAccess::STOP)
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
                .open_service(&svc::SERVICE_NAME, ServiceAccess::DELETE)
                .expect("open service")
                .delete()
                .expect("delete service");
            println!("MiControlFace removed.");
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
