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

/// Path to the on-disk face gallery.
fn store_path() -> std::path::PathBuf {
    std::path::PathBuf::from(DATA_DIR).join("faces.dat")
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
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(PIPE_NAME)
        .is_ok()
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
    match crate::hw::face::camera::Camera::open(0, 3.0) {
        Ok(_) => true,
        Err(_) => false,
    }
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
        use std::process::Command;
        let exe = std::env::current_exe().map_err(|e| FaceErrorResponse {
            code: "other".into(),
            message: e.to_string(),
        })?;
        // The service exe lives next to the app binary.
        let svc = exe.with_file_name("micontrol_face_svc.exe");
        let output = Command::new(&svc)
            .arg("install")
            .output()
            .map_err(|e| FaceErrorResponse {
                code: "pipe".into(),
                message: format!("launch service installer: {e}"),
            })?;
        Ok(serde_json::json!({
            "ok": output.status.success(),
            "stdout": String::from_utf8_lossy(&output.stdout).to_string(),
            "stderr": String::from_utf8_lossy(&output.stderr).to_string(),
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
    let mut store = load_store(&store_path()).unwrap_or_default();
    let obj = settings.as_object().cloned().unwrap_or_default();
    let (sanitized, rejected) = FaceSettings::sanitize(&obj);
    store.settings = sanitized;
    save_store(&store_path(), &store).map_err(FaceErrorResponse::from)?;
    Ok(serde_json::json!({ "ok": true, "rejected": rejected }))
}

/// Store the Windows sign-in password in the LSA Secret (elevated).
#[tauri::command]
pub async fn face_set_password(
    user: String,
    password: String,
) -> Result<serde_json::Value, FaceErrorResponse> {
    crate::hw::face::credvault::store_password(&user, &password)
        .map_err(FaceErrorResponse::from)?;
    Ok(serde_json::json!({ "ok": true }))
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
