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

    let mut store = load_store(&store_path()).unwrap_or_default();
    let embedding = enroll_embedding(&name, n_frames).map_err(FaceErrorResponse::from)?;
    store
        .add_template(&name, embedding, &label)
        .map_err(FaceErrorResponse::from)?;
    save_store(&store_path(), &store).map_err(FaceErrorResponse::from)?;
    Ok(serde_json::json!({ "ok": true, "name": name, "label": label, "frames": n_frames }))
}

/// Produce an embedding for enrollment.
#[cfg(feature = "face")]
fn enroll_embedding(
    name: &str,
    frames: u32,
) -> Result<Vec<f32>, crate::hw::face::errors::FaceError> {
    use crate::hw::face::camera::Camera;
    use crate::hw::face::models::FaceDetector;

    let mut detector = FaceDetector::default();
    detector.load()?;
    let mut camera = Camera::open(0, 20.0)?;
    let mut sum = vec![0.0f32; crate::hw::face::config::EMBEDDING_DIM];
    let mut count = 0u32;

    for _ in 0..frames {
        let frame = camera.read()?;
        let faces = detector.detect(&frame)?;
        if let Some(face) = faces.into_iter().max_by(|a, b| {
            a.area()
                .partial_cmp(&b.area())
                .unwrap_or(std::cmp::Ordering::Equal)
        }) {
            if let Some(emb) = face.embedding {
                for (s, e) in sum.iter_mut().zip(emb.iter()) {
                    *s += e;
                }
                count += 1;
            }
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
        *s /= norm * count as f32;
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

#[cfg(test)]
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
