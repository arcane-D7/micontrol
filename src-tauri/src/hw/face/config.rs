//! Face-unlock configuration defaults.
//!
//! Mirrors the tunable parameters of the reference implementation
//! (`everglow01/Windows-Face-Hello` face_hello/config.py) so the behavior
//! matches the hardened, real-hardware-validated defaults.

use serde::{Deserialize, Serialize};

/// Named pipe for the face auth service.
pub const PIPE_NAME: &str = r"\\.\pipe\micontrol_face";

/// Pipe open-mode flag: fail if an instance with this name already exists
/// (anti-squatting — prevents a malicious process from pre-creating the pipe).
pub const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;

/// Where the face gallery, logs and runtime data live.
#[cfg(windows)]
pub const DATA_DIR: &str = r"C:\ProgramData\MiControl\face";

/// Where bundled ONNX models live (installed with the app).
#[cfg(windows)]
pub const MODELS_DIR: &str = r"C:\Program Files\MiControl\resources\face_models";

/// LSA Secret name prefix — the sign-in password secret is `L$FaceHello_<user>`.
pub const LSA_SECRET_PREFIX: &str = "L$FaceHello_";

/// Embedding dimensionality for ArcFace (InsightFace w600k_r50).
pub const EMBEDDING_DIM: usize = 512;

/// Max number of profiles in the gallery.
pub const MAX_PROFILES: usize = 1000;

/// Max templates per profile (same-name "add angle" appends, FIFO cap).
pub const MAX_TEMPLATES_PER_NAME: usize = 5;

/// Face store magic + format version.
pub const STORE_MAGIC: &[u8] = b"MICONTROL_FACE1\n";

/// Default settings (validated, mirrors reference defaults).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FaceSettings {
    /// Recognition: minimum cosine similarity to accept.
    pub match_threshold: f32,
    /// Anti-misrouting: minimum gap between best and best-other-person.
    pub match_margin: f32,
    /// Enable active liveness (blink/turn challenge).
    pub liveness_enabled: bool,
    /// Enable passive anti-spoofing (MiniFASNet).
    pub antispoof_enabled: bool,
    /// Anti-spoof real-score gate (softmax index 1 ≥ this).
    pub antispoof_threshold: f32,
    /// Anti-spoof samples before fail-open.
    pub antispoof_max_frames: u32,
    /// Consecutive biometric failures before lockout.
    pub lockout_max_fails: u32,
    /// Lockout duration (seconds).
    pub lockout_seconds: u32,
    /// Reject if 2+ faces in frame (optional multi-person protection).
    pub multi_face_protection_enabled: bool,
    /// Master switch — hide tile entirely when disabled.
    pub face_unlock_enabled: bool,
    /// Show tile at boot/sign-in.
    pub face_unlock_logon_enabled: bool,
    /// Show tile at workstation unlock (Win+L).
    pub face_unlock_workstation_enabled: bool,
    /// Recommended re-enrollment interval (days); reminder only.
    pub renew_days: u32,
    /// Camera index (0=default).
    pub camera_index: u32,
    /// UI language (ISO code; unused by core logic).
    pub language: String,
}

impl Default for FaceSettings {
    fn default() -> Self {
        Self {
            match_threshold: 0.40,
            match_margin: 0.05,
            liveness_enabled: true,
            antispoof_enabled: true,
            antispoof_threshold: 0.55,
            antispoof_max_frames: 10,
            lockout_max_fails: 5,
            lockout_seconds: 30,
            multi_face_protection_enabled: false,
            face_unlock_enabled: true,
            face_unlock_logon_enabled: true,
            face_unlock_workstation_enabled: true,
            renew_days: 60,
            camera_index: 0,
            language: "en".to_string(),
        }
    }
}

impl FaceSettings {
    /// Validate and sanitize a settings map (reject unknown/invalid keys).
    /// Returns `(validated_settings, rejected_keys)`.
    pub fn sanitize(
        map: &serde_json::Map<String, serde_json::Value>,
    ) -> (FaceSettings, Vec<String>) {
        let mut s = FaceSettings::default();
        let mut rejected = Vec::new();

        macro_rules! take_f32 {
            ($key:expr, $field:ident) => {
                if let Some(v) = map.get($key) {
                    match v.as_f64() {
                        Some(f) if f.is_finite() => s.$field = f as f32,
                        _ => rejected.push($key.to_string()),
                    }
                }
            };
        }
        macro_rules! take_u32 {
            ($key:expr, $field:ident) => {
                if let Some(v) = map.get($key) {
                    match v.as_u64() {
                        Some(n) if n <= u32::MAX as u64 => s.$field = n as u32,
                        _ => rejected.push($key.to_string()),
                    }
                }
            };
        }
        macro_rules! take_bool {
            ($key:expr, $field:ident) => {
                if let Some(v) = map.get($key) {
                    match v.as_bool() {
                        Some(b) => s.$field = b,
                        None => rejected.push($key.to_string()),
                    }
                }
            };
        }
        macro_rules! take_str {
            ($key:expr, $field:ident) => {
                if let Some(v) = map.get($key) {
                    match v.as_str() {
                        Some(x) => s.$field = x.to_string(),
                        None => rejected.push($key.to_string()),
                    }
                }
            };
        }

        take_f32!("match_threshold", match_threshold);
        take_f32!("match_margin", match_margin);
        take_bool!("liveness_enabled", liveness_enabled);
        take_bool!("antispoof_enabled", antispoof_enabled);
        take_f32!("antispoof_threshold", antispoof_threshold);
        take_u32!("antispoof_max_frames", antispoof_max_frames);
        take_u32!("lockout_max_fails", lockout_max_fails);
        take_u32!("lockout_seconds", lockout_seconds);
        take_bool!(
            "multi_face_protection_enabled",
            multi_face_protection_enabled
        );
        take_bool!("face_unlock_enabled", face_unlock_enabled);
        take_bool!("face_unlock_logon_enabled", face_unlock_logon_enabled);
        take_bool!(
            "face_unlock_workstation_enabled",
            face_unlock_workstation_enabled
        );
        take_u32!("renew_days", renew_days);
        take_u32!("camera_index", camera_index);
        take_str!("language", language);

        // Clamp sensible ranges.
        s.match_threshold = s.match_threshold.clamp(0.0, 1.0);
        s.match_margin = s.match_margin.clamp(0.0, 1.0);
        s.antispoof_threshold = s.antispoof_threshold.clamp(0.0, 1.0);
        s.antispoof_max_frames = s.antispoof_max_frames.clamp(1, 60);
        s.renew_days = s.renew_days.clamp(0, 3650);
        s.camera_index = s.camera_index.min(8);

        (s, rejected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_are_valid() {
        let s = FaceSettings::default();
        assert_eq!(s.match_threshold, 0.40);
        assert!(s.liveness_enabled);
        assert!(s.antispoof_enabled);
        assert_eq!(s.lockout_max_fails, 5);
        assert_eq!(s.lockout_seconds, 30);
    }

    #[test]
    fn sanitize_accepts_valid_and_rejects_invalid() {
        let map = json!({
            "match_threshold": 0.5,
            "lockout_max_fails": 3,
            "liveness_enabled": false,
            "language": "pt",
        });
        let (s, rejected) = FaceSettings::sanitize(map.as_object().unwrap());
        assert!(rejected.is_empty(), "unexpected rejections: {rejected:?}");
        assert_eq!(s.match_threshold, 0.5);
        assert_eq!(s.lockout_max_fails, 3);
        assert!(!s.liveness_enabled);
        assert_eq!(s.language, "pt");
    }

    #[test]
    fn sanitize_rejects_wrong_types() {
        let map = json!({
            "match_threshold": "high",
            "lockout_max_fails": true,
            "liveness_enabled": 42,
        });
        let (s, rejected) = FaceSettings::sanitize(map.as_object().unwrap());
        assert_eq!(rejected.len(), 3);
        // Falls back to defaults for rejected keys.
        assert_eq!(s.match_threshold, 0.40);
        assert_eq!(s.lockout_max_fails, 5);
        assert!(s.liveness_enabled);
    }

    #[test]
    fn sanitize_clamps_out_of_range() {
        let map = json!({
            "match_threshold": 99.0,
            "antispoof_max_frames": 999,
        });
        let (s, _) = FaceSettings::sanitize(map.as_object().unwrap());
        assert_eq!(s.match_threshold, 1.0);
        assert_eq!(s.antispoof_max_frames, 60);
    }
}
