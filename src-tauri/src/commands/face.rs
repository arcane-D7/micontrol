//! Tauri commands for the Face Unlock module.
//!
//! Exposes enrollment, template management, settings, service control and
//! diagnostics for the Windows Hello-style face unlock feature. When the
//! `face` feature is enabled the camera/ORT pipeline is available; without it
//! the commands still work for service control and settings (the recognition
//! pipeline degrades gracefully).

use crate::hw::face::config::{DATA_DIR, PIPE_NAME};
use crate::hw::face::errors::FaceErrorResponse;
use crate::hw::face::store::{load_store, save_store};
use crate::hw::face::FaceSettings;
use serde::Serialize;
use tauri::Emitter;

/// Path to the on-disk face gallery.
fn store_path() -> std::path::PathBuf {
    std::path::PathBuf::from(DATA_DIR).join("faces.dat")
}

/// Ensure the data directory exists before writing the gallery.
/// Without this, `face_set_settings` / `face_enroll` / `face_delete_template`
/// fail on fresh installs with `write tmp: The system cannot find the path
/// specified (os error 3)` because `C:\ProgramData\MiControl\face` is only
/// created by the face service when it starts.
fn ensure_data_dir() -> Result<(), FaceErrorResponse> {
    std::fs::create_dir_all(DATA_DIR).map_err(|e| FaceErrorResponse {
        code: "store".into(),
        message: format!("create data dir: {e}"),
    })
}

#[derive(Debug, Serialize)]
pub struct FaceStatus {
    pub service_installed: bool,
    pub service_running: bool,
    pub pipe_available: bool,
    pub enrolled_profiles: usize,
    pub camera_available: bool,
}

fn service_installed() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::System::Services::OpenServiceW;
        let manager = unsafe {
            windows::Win32::System::Services::OpenSCManagerW(
                None,
                None,
                windows::Win32::System::Services::SC_MANAGER_CONNECT,
            )
        };
        if manager.is_err() {
            return false;
        }
        let manager = manager.unwrap();
        let name: Vec<u16> = "MiControlFace\0".encode_utf16().collect();
        let svc = unsafe {
            OpenServiceW(
                manager,
                windows::core::PCWSTR(name.as_ptr()),
                windows::Win32::System::Services::SERVICE_QUERY_STATUS,
            )
        };
        let found = svc.is_ok();
        unsafe {
            windows::Win32::System::Services::CloseServiceHandle(svc.unwrap_or_default()).ok();
            windows::Win32::System::Services::CloseServiceHandle(manager).ok();
        }
        found
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn service_running() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::System::Services::{
            OpenSCManagerW, OpenServiceW, QueryServiceStatus, SC_MANAGER_CONNECT,
            SERVICE_QUERY_STATUS, SERVICE_RUNNING,
        };
        let manager = unsafe { OpenSCManagerW(None, None, SC_MANAGER_CONNECT) };
        if manager.is_err() {
            return false;
        }
        let manager = manager.unwrap();
        let name: Vec<u16> = "MiControlFace\0".encode_utf16().collect();
        let svc = unsafe {
            OpenServiceW(
                manager,
                windows::core::PCWSTR(name.as_ptr()),
                SERVICE_QUERY_STATUS,
            )
        };
        if svc.is_err() {
            unsafe {
                windows::Win32::System::Services::CloseServiceHandle(manager).ok();
            }
            return false;
        }
        let mut status = windows::Win32::System::Services::SERVICE_STATUS::default();
        let ok = unsafe { QueryServiceStatus(svc.unwrap(), &mut status) };
        unsafe {
            windows::Win32::System::Services::CloseServiceHandle(manager).ok();
        }
        ok.is_ok() && status.dwCurrentState == SERVICE_RUNNING
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn pipe_available() -> bool {
    // The auth pipe's DACL grants SYSTEM + Administrators only (the Credential
    // Provider at LogonUI runs as SYSTEM and unlocks via it). A normal,
    // non-elevated app user therefore cannot OPEN the pipe — but the pipe
    // still exists and works. We must not report "unavailable" just because
    // opening failed with access-denied:
    //   - File not found / path invalid → pipe really is gone → false
    //   - Access denied → pipe exists (design intent, SYSTEM-only) → true
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(PIPE_NAME)
    {
        Ok(_) => true,
        Err(e) => {
            let io = e.kind();
            io == std::io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(5)
            /* ERROR_ACCESS_DENIED */
        }
    }
}

/// Current Face Unlock status for the UI.
#[tauri::command]
pub async fn face_status() -> Result<FaceStatus, FaceErrorResponse> {
    let store = load_store(&store_path()).unwrap_or_default();
    let camera_available = camera_available();
    Ok(FaceStatus {
        service_installed: service_installed(),
        service_running: service_running(),
        pipe_available: pipe_available(),
        enrolled_profiles: store.profiles.len(),
        camera_available,
    })
}

#[cfg(feature = "face")]
fn camera_available() -> bool {
    // Probe opening camera index 0 briefly.
    crate::hw::face::camera::Camera::open(0, 3.0).is_ok()
}

#[cfg(not(feature = "face"))]
fn camera_available() -> bool {
    false
}

/// Install/start the MiControlFace auth service (elevated).
#[tauri::command]
pub async fn face_service_install() -> Result<serde_json::Value, FaceErrorResponse> {
    #[cfg(windows)]
    {
        // S42-020: route through the elevated helper (autonomous bridge
        // service → scheduled task → UAC). Installing an SCM service requires
        // admin; the raw Command::new() path below used to fail with
        // "Access is denied." (OpenSCManager ERROR_ACCESS_DENIED) whenever the
        // app UI ran unelevated — leaving the Face Unlock UI dead.
        let result =
            crate::elev_bridge::run_elevated("face_service_install", serde_json::json!({}))
                .await
                .map_err(|e| {
                    let msg = if e.contains("Unknown elevated command") {
                        "The installed helper does not support 'face_service_install' yet. \
                     Please reinstall miControl (v0.1.18+) and try again."
                            .to_string()
                    } else {
                        format!("install auth service: {e}")
                    };
                    FaceErrorResponse {
                        code: "elevated".into(),
                        message: msg,
                    }
                })?;

        Ok(serde_json::json!({
            "ok": result.get("service_installed").and_then(|v| v.as_bool()).unwrap_or(false),
            "stdout": result.get("output").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            "stderr": "",
            "exit_code": result.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1),
        }))
    }
    #[cfg(not(windows))]
    {
        Err(FaceErrorResponse {
            code: "not_supported".into(),
            message: "Windows only".into(),
        })
    }
}

// ── Model download / install ────────────────────────────────────────────────
// The ONNX models (InsightFace `buffalo_l`: det_10g + w600k_r50) are ~250 MB
// and are downloaded on demand into a staging dir under ProgramData (no admin
// needed), verified with a real ORT session, then copied into
// `C:\Program Files\MiControl\resources\face_models` (needs admin — done via
// the elevated helper or by the installer on the next reinstall).

/// URL of the InsightFace `buffalo_l` model pack (GH release).
pub const BUFFALO_L_URL: &str =
    "https://github.com/deepinsight/insightface/releases/download/v0.7/buffalo_l.zip";

fn models_staging_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(DATA_DIR).join("models_staging")
}

fn models_staging_extract() -> std::path::PathBuf {
    models_staging_dir().join("buffalo_l")
}

/// Check whether the required ONNX models are present.
#[tauri::command]
pub async fn face_models_status() -> Result<serde_json::Value, FaceErrorResponse> {
    let installed = models_present(crate::hw::face::config::MODELS_DIR);
    let staged = models_present(&models_staging_extract().display().to_string());
    Ok(serde_json::json!({
        "installed": installed,
        "staged": staged,
        "installed_dir": crate::hw::face::config::MODELS_DIR,
        "staging_dir": models_staging_extract().display().to_string(),
        "url": BUFFALO_L_URL,
    }))
}

/// True when both det_10g.onnx and w600k_r50.onnx exist in `dir`.
fn models_present(dir: &str) -> bool {
    let det = std::path::Path::new(dir).join("det_10g.onnx");
    let rec = std::path::Path::new(dir).join("w600k_r50.onnx");
    det.exists() && rec.exists()
}

/// Download + extract the InsightFace models into the staging directory.
/// Progress is emitted via the `face-model-progress` Tauri event (u8 percent).
#[tauri::command]
pub async fn face_download_models(
    app: tauri::AppHandle,
) -> Result<serde_json::Value, FaceErrorResponse> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    let staging = models_staging_dir();
    std::fs::create_dir_all(&staging).map_err(|e| FaceErrorResponse {
        code: "io".into(),
        message: format!("create staging dir: {e}"),
    })?;
    let zip_path = staging.join("buffalo_l.zip");
    let target_dir = models_staging_extract();

    // 1. Download with progress.
    let client = reqwest::Client::builder()
        .user_agent("MiControl/0.1 face-unlock")
        .build()
        .map_err(|e| FaceErrorResponse {
            code: "http".into(),
            message: format!("http client: {e}"),
        })?;
    let resp = client
        .get(BUFFALO_L_URL)
        .send()
        .await
        .map_err(|e| FaceErrorResponse {
            code: "http".into(),
            message: format!("download request: {e}"),
        })?;
    if !resp.status().is_success() {
        return Err(FaceErrorResponse {
            code: "http".into(),
            message: format!("download failed: HTTP {}", resp.status()),
        });
    }
    let total = resp.content_length().unwrap_or(0);
    let mut file =
        tokio::io::BufWriter::new(tokio::fs::File::create(&zip_path).await.map_err(|e| {
            FaceErrorResponse {
                code: "io".into(),
                message: format!("create zip: {e}"),
            }
        })?);
    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| FaceErrorResponse {
            code: "http".into(),
            message: format!("read chunk: {e}"),
        })?;
        downloaded += chunk.len() as u64;
        if total > 0 {
            // Avoid wrap/overflow for absurd payloads (clippy: manual-checked-ops).
            let pct = downloaded
                .saturating_mul(100)
                .checked_div(total)
                .unwrap_or(0) as u8;
            let _ = app.emit("face-model-progress", pct);
        }
        file.write_all(&chunk)
            .await
            .map_err(|e| FaceErrorResponse {
                code: "io".into(),
                message: format!("write chunk: {e}"),
            })?;
    }
    file.flush().await.map_err(|e| FaceErrorResponse {
        code: "io".into(),
        message: format!("flush zip: {e}"),
    })?;

    // 2. Extract det_10g + w600k_r50 from the zip.
    let file = std::fs::File::open(&zip_path).map_err(|e| FaceErrorResponse {
        code: "io".into(),
        message: format!("open zip: {e}"),
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| FaceErrorResponse {
        code: "zip".into(),
        message: format!("bad zip: {e}"),
    })?;
    std::fs::create_dir_all(&target_dir).map_err(|e| FaceErrorResponse {
        code: "io".into(),
        message: format!("create extract dir: {e}"),
    })?;
    let wanted = ["det_10g.onnx", "w600k_r50.onnx"];
    let mut extracted = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| FaceErrorResponse {
            code: "zip".into(),
            message: format!("zip entry: {e}"),
        })?;
        let name = entry.name().to_string();
        let base = std::path::Path::new(&name)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if !wanted.contains(&base.as_str()) {
            continue;
        }
        let out = target_dir.join(&base);
        let mut out_f = std::fs::File::create(&out).map_err(|e| FaceErrorResponse {
            code: "io".into(),
            message: format!("create {}: {e}", base),
        })?;
        std::io::copy(&mut entry, &mut out_f).map_err(|e| FaceErrorResponse {
            code: "io".into(),
            message: format!("extract {}: {e}", base),
        })?;
        extracted += 1;
    }
    if extracted < 2 {
        return Err(FaceErrorResponse {
            code: "zip".into(),
            message: "archive did not contain det_10g.onnx / w600k_r50.onnx".into(),
        });
    }

    // 3. Verify loadability with the actual ORT session.
    let mut det = crate::hw::face::models::FaceDetector::with_models(
        &target_dir.display().to_string(),
        crate::hw::face::models::DET_MODEL,
        crate::hw::face::models::REC_MODEL,
    );
    det.load().map_err(|e| FaceErrorResponse {
        code: "model".into(),
        message: format!("downloaded models fail to load: {e}"),
    })?;

    let _ = app.emit("face-model-progress", 100u8);
    Ok(serde_json::json!({
        "ok": true,
        "staging": target_dir.display().to_string(),
    }))
}

/// Move the staged models into `MODELS_DIR` (Program Files — needs admin).
#[tauri::command]
pub async fn face_install_models() -> Result<serde_json::Value, FaceErrorResponse> {
    let src = models_staging_extract();
    let dst = std::path::PathBuf::from(crate::hw::face::config::MODELS_DIR);
    std::fs::create_dir_all(&dst).map_err(|e| FaceErrorResponse {
        code: "io".into(),
        message: format!("cannot create install dir (needs admin): {e}"),
    })?;
    for f in ["det_10g.onnx", "w600k_r50.onnx"] {
        let from = src.join(f);
        if !from.exists() {
            return Err(FaceErrorResponse {
                code: "model".into(),
                message: format!("staged model missing: {from:?}"),
            });
        }
        std::fs::copy(&from, dst.join(f)).map_err(|e| FaceErrorResponse {
            code: "io".into(),
            message: format!("copy {f}: {e} (Program Files needs admin)"),
        })?;
    }
    Ok(serde_json::json!({
        "ok": true,
        "installed_dir": crate::hw::face::config::MODELS_DIR,
    }))
}

/// Remove the downloaded/installed InsightFace models (both the staging copy
/// under ProgramData and the installed copy under Program Files if possible).
///
/// After this, `face_models_status` reports `installed=false`/`staged=false`
/// and the UI re-enables the "Download & install module" button.
#[tauri::command]
pub async fn face_models_remove_all(
    app: tauri::AppHandle,
) -> Result<serde_json::Value, FaceErrorResponse> {
    let mut removed: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    fn remove_matching(
        dir: &std::path::Path,
        removed: &mut Vec<String>,
        warnings: &mut Vec<String>,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return; // dir does not exist — nothing to remove
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // Only remove the model artifacts we manage. Keep unknown files.
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let is_model_file = matches!(
                name.as_str(),
                "det_10g.onnx" | "w600k_r50.onnx" | "buffalo_l.zip"
            );
            // Also remove the extracted "buffalo_l" subdir if it exists.
            if is_model_file {
                match std::fs::remove_file(&path) {
                    Ok(()) => removed.push(path.display().to_string()),
                    Err(e) => warnings.push(format!("remove {}: {e}", path.display())),
                }
            }
        }
        // Remove the empty extraction subdir too.
        let sub = dir.join("buffalo_l");
        if sub.is_dir() {
            match std::fs::remove_dir_all(&sub) {
                Ok(()) => removed.push(sub.display().to_string()),
                Err(e) => warnings.push(format!("remove {}: {e}", sub.display())),
            }
        }
    }

    remove_matching(&models_staging_dir(), &mut removed, &mut warnings);
    let installed_dir = std::path::PathBuf::from(crate::hw::face::config::MODELS_DIR);
    remove_matching(&installed_dir, &mut removed, &mut warnings);

    // Notify listeners (progress UI hides, download button re-enables).
    let _ = app.emit("face-models-removed", ());
    Ok(serde_json::json!({
        "ok": true,
        "removed": removed,
        "warnings": warnings,
        "status": {
            "installed": models_present(crate::hw::face::config::MODELS_DIR),
            "staged": models_present(&models_staging_extract().display().to_string()),
        },
    }))
}

// ── Live camera preview (enrollment wizard) ────────────────────────────────

/// Start the preview capture thread on `index` (default: configured index).
#[tauri::command]
pub async fn face_camera_preview_start(
    index: Option<u32>,
) -> Result<serde_json::Value, FaceErrorResponse> {
    let idx = index.unwrap_or(
        load_store(&store_path())
            .map(|s| s.settings.camera_index)
            .unwrap_or(0),
    );
    crate::hw::face::preview::start(idx).map_err(FaceErrorResponse::from)?;
    Ok(serde_json::json!({ "ok": true, "index": idx }))
}

/// Stop the preview capture thread.
#[tauri::command]
pub async fn face_camera_preview_stop() -> Result<serde_json::Value, FaceErrorResponse> {
    crate::hw::face::preview::stop();
    Ok(serde_json::json!({ "ok": true }))
}

/// Latest preview frame as base64 JPEG (or null). `running` indicates whether
/// the capture thread is alive; `error` holds the last thread failure.
#[tauri::command]
pub async fn face_camera_preview_frame() -> Result<serde_json::Value, FaceErrorResponse> {
    use base64::Engine;
    let frame = crate::hw::face::preview::latest();
    Ok(serde_json::json!({
        "running": true,
        "error": crate::hw::face::preview::last_error(),
        "jpeg": frame.as_ref().map(|f| base64::engine::general_purpose::STANDARD.encode(&f.jpeg)),
        "width": frame.as_ref().map(|f| f.width),
        "height": frame.as_ref().map(|f| f.height),
    }))
}

/// List enrolled face profiles (names + template counts).
#[tauri::command]
pub async fn face_list_templates() -> Result<serde_json::Value, FaceErrorResponse> {
    let store = load_store(&store_path()).unwrap_or_default();
    let profiles: Vec<serde_json::Value> = store
        .profiles
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "templates": p.templates.len(),
                "labels": p.templates.iter().map(|t| t.label.clone()).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(serde_json::json!({ "profiles": profiles }))
}

/// Delete one template (by profile name + index).
#[tauri::command]
pub async fn face_delete_template(
    name: String,
    index: usize,
) -> Result<serde_json::Value, FaceErrorResponse> {
    ensure_data_dir()?;
    let mut store = load_store(&store_path()).unwrap_or_default();
    store
        .remove_template(&name, index)
        .map_err(FaceErrorResponse::from)?;
    save_store(&store_path(), &store).map_err(FaceErrorResponse::from)?;
    Ok(serde_json::json!({ "ok": true }))
}

/// Get the current face settings.
#[tauri::command]
pub async fn face_get_settings() -> Result<FaceSettings, FaceErrorResponse> {
    let store = load_store(&store_path()).unwrap_or_default();
    Ok(store.settings)
}

/// Update face settings (validated).
#[tauri::command]
pub async fn face_set_settings(
    settings: serde_json::Value,
) -> Result<serde_json::Value, FaceErrorResponse> {
    ensure_data_dir()?;
    let mut store = load_store(&store_path()).unwrap_or_default();
    let obj = settings.as_object().cloned().unwrap_or_default();
    let (sanitized, rejected) = FaceSettings::sanitize(&obj);
    store.settings = sanitized;
    save_store(&store_path(), &store).map_err(FaceErrorResponse::from)?;
    Ok(serde_json::json!({ "ok": true, "rejected": rejected }))
}

/// Store the Windows sign-in password in the LSA Secret (elevated).
///
/// `LsaStorePrivateData` with `POLICY_CREATE_SECRET` requires elevation, so
/// this is dispatched through the elevated helper (autonomous bridge service
/// → scheduled task → UAC) instead of calling the LSA directly from the
/// unprivileged app process.
#[tauri::command]
pub async fn face_set_password(
    user: String,
    password: String,
) -> Result<serde_json::Value, FaceErrorResponse> {
    #[cfg(windows)]
    {
        let args = serde_json::json!({ "user": user, "password": password });
        let result = crate::elev_bridge::run_elevated("face_set_password", args)
            .await
            .map_err(|e| {
                let msg = if e.contains("Unknown elevated command") {
                    "The installed helper does not support 'face_set_password' yet. \
                         Please reinstall miControl (v0.1.18+) and try again."
                        .to_string()
                } else {
                    format!("store sign-in password: {e}")
                };
                FaceErrorResponse {
                    code: "elevated".into(),
                    message: msg,
                }
            })?;
        Ok(result)
    }
    #[cfg(not(windows))]
    {
        Err(FaceErrorResponse {
            code: "not_supported".into(),
            message: "LSA secrets require Windows".into(),
        })
    }
}

/// Whether a sign-in password secret exists for `user` (no elevation needed
/// to check — the LSA secret is readable by the SYSTEM service; the unprivileged
/// app cannot read it, so this returns `None` for "unknown/not readable"). This
/// is only a *hint*; the authoritative check happens at unlock time.
#[tauri::command]
pub async fn face_password_configured(
    user: String,
) -> Result<serde_json::Value, FaceErrorResponse> {
    #[cfg(windows)]
    {
        // Read requires POLICY_GET_PRIVATE_INFORMATION (SYSTEM) — the app runs
        // unelevated, so reading directly fails. Route through the bridge so
        // the UI can show "password already stored" accurately.
        let args = serde_json::json!({ "user": user });
        let result = crate::elev_bridge::run_elevated_no_prompt("face_password_configured", args)
            .await
            .unwrap_or(serde_json::json!({ "configured": false, "unknown": true }));
        Ok(result)
    }
    #[cfg(not(windows))]
    {
        Ok(serde_json::json!({ "configured": false, "unknown": true }))
    }
}

/// List local Windows user accounts for the enrollment dropdown.
///
/// Returns `users` (sorted alphabetically) plus `current_user` — the username
/// of the currently logged-in interactive account (used as the dropdown
/// default).
#[tauri::command]
pub async fn face_list_users() -> Result<serde_json::Value, FaceErrorResponse> {
    let users = crate::hw::face::users::list_local_users().map_err(FaceErrorResponse::from)?;
    let current = current_windows_user();
    Ok(serde_json::json!({ "users": users, "current_user": current }))
}

/// Name of the current interactive user (best-effort; may be empty).
#[cfg(windows)]
fn current_windows_user() -> String {
    std::env::var("USERNAME").unwrap_or_default()
}

#[cfg(not(windows))]
fn current_windows_user() -> String {
    String::new()
}

/// Windows Hello consent gate for enrollment.
///
/// Presents the user's configured Windows Hello factor (PIN / fingerprint /
/// face) in a modal attached to the app's main window. Used before the
/// password-once step so we never store a password without confirming the
/// user is present and authentic.
#[tauri::command]
pub async fn face_hello_verify(
    window: tauri::WebviewWindow,
) -> Result<serde_json::Value, FaceErrorResponse> {
    #[cfg(windows)]
    {
        let message = "miControl Face Unlock — Confirm your identity with Windows Hello (PIN, \
                       fingerprint or face) to enable face unlock for your Windows account.";
        let hwnd = window.hwnd().map_err(|e| FaceErrorResponse {
            code: "hello".into(),
            message: format!("cannot get main window handle: {e}"),
        })?;
        let result = crate::hw::face::users::verify_hello(message, hwnd.0)
            .map_err(FaceErrorResponse::from)?;
        Ok(serde_json::to_value(&result).unwrap_or(serde_json::Value::Null))
    }
    #[cfg(not(windows))]
    {
        Err(FaceErrorResponse {
            code: "not_supported".into(),
            message: "Windows Hello consent requires Windows".into(),
        })
    }
}

/// Simple diagnostics: service state, pipe, gallery, models.
#[tauri::command]
pub async fn face_diagnostics() -> Result<serde_json::Value, FaceErrorResponse> {
    let store = load_store(&store_path()).unwrap_or_default();
    Ok(serde_json::json!({
        "service_installed": service_installed(),
        "service_running": service_running(),
        "pipe_available": pipe_available(),
        "enrolled": store.profiles.len(),
        "templates": store.profiles.iter().map(|p| p.templates.len()).sum::<usize>(),
        "models_dir": crate::hw::face::config::MODELS_DIR,
        "data_dir": DATA_DIR,
    }))
}

/// Enroll a face for a profile.
///
/// With the `face` feature this captures frames from the webcam, runs
/// detection + embedding, and averages several frames into a template.
/// Without the `face` feature (e.g. dev builds without a C++ toolchain),
/// it stores a deterministic pseudo-embedding derived from the profile name
/// so the full pipeline (store, matcher, service) is exercisable end-to-end.
///
/// `frames` defaults to 4 (1..=16); `label` is a human tag (e.g. "front").
#[tauri::command]
pub async fn face_enroll(
    name: String,
    frames: Option<u32>,
    label: Option<String>,
) -> Result<serde_json::Value, FaceErrorResponse> {
    let n_frames = frames.unwrap_or(4).clamp(1, 16);
    let label = label.unwrap_or_else(|| "front".to_string());

    ensure_data_dir()?;
    let mut store = load_store(&store_path()).unwrap_or_default();
    let embedding = enroll_embedding(&name, n_frames).map_err(FaceErrorResponse::from)?;
    store
        .add_template(&name, embedding, &label)
        .map_err(FaceErrorResponse::from)?;
    save_store(&store_path(), &store).map_err(FaceErrorResponse::from)?;
    Ok(serde_json::json!({ "ok": true, "name": name, "label": label, "frames": n_frames }))
}

/// Produce an embedding for enrollment (real ORT pipeline).
#[cfg(feature = "face")]
fn enroll_embedding(
    name: &str,
    frames: u32,
) -> Result<Vec<f32>, crate::hw::face::errors::FaceError> {
    use crate::hw::face::camera::Camera;
    use crate::hw::face::models::FaceDetector;

    let mut detector = FaceDetector::default();
    detector.add_models_dir(&models_staging_extract().display().to_string());
    detector.load()?;
    let cam_index = load_store(&store_path())
        .map(|s| s.settings.camera_index)
        .unwrap_or(0);
    let mut camera = Camera::open(cam_index, 20.0)?;
    let mut sum = vec![0.0f32; crate::hw::face::config::EMBEDDING_DIM];
    let mut count = 0u32;

    for _ in 0..frames {
        let frame = camera.read()?;
        let faces = detector.detect(&frame)?;
        if let Some(face) = faces.into_iter().max_by(|a, b| {
            a.det_score
                .partial_cmp(&b.det_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        }) {
            // Run recognition on the strongest face (fills embedding).
            let emb = detector.recognize(
                &frame.data,
                frame.width,
                frame.height,
                &face.kps,
                &face.bbox,
            )?;
            for (s, e) in sum.iter_mut().zip(emb.iter()) {
                *s += e;
            }
            count += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(120));
    }

    if count == 0 {
        return Err(crate::hw::face::errors::FaceError::NoFace(
            "no face detected during enrollment".into(),
        ));
    }
    // Average + L2-normalize.
    let norm = {
        let mut n = 0.0f64;
        for s in sum.iter() {
            n += (*s as f64) * (*s as f64);
        }
        (n.sqrt() as f32).max(1e-6)
    };
    for s in sum.iter_mut() {
        *s /= norm;
    }
    let _ = name;
    Ok(sum)
}

/// Deterministic pseudo-embedding when the camera/ORT pipeline is unavailable.
#[cfg(not(feature = "face"))]
fn enroll_embedding(
    name: &str,
    _frames: u32,
) -> Result<Vec<f32>, crate::hw::face::errors::FaceError> {
    // Hash the name into a stable, seeded 512-d vector (L2-normalized) so
    // the same name always enrolls the same template. The seed is spread
    // aggressively so similar names produce well-separated embeddings.
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    for b in name.bytes() {
        seed = seed.wrapping_mul(0x100_0000_01B3).wrapping_add(b as u64);
        seed ^= seed >> 33;
        seed = seed.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    }
    let dim = crate::hw::face::config::EMBEDDING_DIM;
    let mut out = vec![0.0f32; dim];
    let mut x = seed;
    for v in out.iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x = x.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(seed);
        // Symmetric values in [-1, 1]: independent pseudo-random embeddings
        // then have cosine ≈ 0 (unlike a [0,1] uniform source which is
        // positively correlated).
        *v = ((x >> 33) % 2000) as f32 / 1000.0 - 1.0;
    }
    let norm: f64 = out
        .iter()
        .map(|v| (*v as f64) * (*v as f64))
        .sum::<f64>()
        .sqrt();
    if norm > 1e-9 {
        for v in out.iter_mut() {
            *v = (*v as f64 / norm) as f32;
        }
    }
    Ok(out)
}

#[cfg(all(test, not(feature = "face")))]
mod tests {
    use super::*;
    use crate::hw::face::store::FaceStore;

    #[test]
    fn mock_enrollment_is_deterministic() {
        // The same name always yields the same embedding.
        let a = enroll_embedding("alice", 4).unwrap();
        let b = enroll_embedding("alice", 8).unwrap();
        assert_eq!(a.len(), crate::hw::face::config::EMBEDDING_DIM);
        assert_eq!(a, b, "mock enrollment must be deterministic per name");
    }

    #[test]
    fn mock_enrollment_differs_per_name() {
        let a = enroll_embedding("alice", 4).unwrap();
        let b = enroll_embedding("bob", 4).unwrap();
        // Different names → different embeddings (low cosine).
        let sim = crate::hw::face::matcher::cosine_similarity(&a, &b);
        assert!(sim < 0.5, "different names should not match: sim={sim}");
    }

    #[test]
    fn mock_enroll_store_recognize_roundtrip() {
        // Enroll "alice" into a temp store, then run the auth pipeline with
        // a probe generated from the same name → must unlock.
        let mut store = FaceStore::new();
        store.settings.liveness_enabled = false;
        let emb = enroll_embedding("alice", 4).unwrap();
        store.add_template("alice", emb, "front").unwrap();

        let probe = enroll_embedding("alice", 4).unwrap();
        let mut session = crate::hw::face::service::AuthSession::new(&store);
        session.feed(0.3, 0.0, Some(probe));
        assert!(session.done());
        let r = session.result().unwrap();
        assert!(
            r.success,
            "mock enroll → recognize roundtrip should succeed"
        );
        assert_eq!(r.name.as_deref(), Some("alice"));
    }
}
