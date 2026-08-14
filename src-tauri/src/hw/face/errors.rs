//! Structured errors for the face-unlock module.

use serde::{Deserialize, Serialize};

/// Face unlock operation errors.
#[derive(Debug, Clone, thiserror::Error)]
pub enum FaceError {
    /// Camera could not be opened or read a frame.
    #[error("Camera error: {0}")]
    Camera(String),

    /// ONNX model load or inference failure.
    #[error("Model error: {0}")]
    Model(String),

    /// No face detected in the frame.
    #[error("No face detected: {0}")]
    NoFace(String),

    /// Liveness check failed (blink/pose challenge not satisfied).
    #[error("Liveness failed: {0}")]
    Liveness(String),

    /// Passive anti-spoofing rejected the frame (photo/video replay).
    #[error("Anti-spoof rejection: {0}")]
    Antispoof(String),

    /// No enrolled templates match, or the match is ambiguous (margin too small).
    #[error("Face mismatch: {0}")]
    Mismatch(String),

    /// The face store (gallery) failed to load/save/decrypt.
    #[error("Store error: {0}")]
    Store(String),

    /// LSA Secret read/write failure.
    #[error("Credential vault error: {0}")]
    CredVault(String),

    /// The auth service pipe is unavailable or rejected the request.
    #[error("Service pipe error: {0}")]
    Pipe(String),

    /// A required model file is missing.
    #[error("Missing model: {0}")]
    MissingModel(String),

    /// The operation is only supported on Windows.
    #[error("Not supported: {0}")]
    NotSupported(String),

    /// Local user enumeration failed.
    #[error("User enumeration error: {0}")]
    Users(String),

    /// Windows Hello consent gate failed.
    #[error("Windows Hello error: {0}")]
    Hello(String),

    /// Generic/other error.
    #[error("Face error: {0}")]
    Other(String),
}

/// Convenience alias.
pub type FaceResult<T> = Result<T, FaceError>;

/// A serializable error response for the Tauri frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaceErrorResponse {
    pub code: String,
    pub message: String,
}

impl From<FaceError> for FaceErrorResponse {
    fn from(e: FaceError) -> Self {
        let code = match &e {
            FaceError::Camera(_) => "camera",
            FaceError::Model(_) | FaceError::MissingModel(_) => "model",
            FaceError::NoFace(_) => "no_face",
            FaceError::Liveness(_) => "liveness",
            FaceError::Antispoof(_) => "antispoof",
            FaceError::Mismatch(_) => "mismatch",
            FaceError::Store(_) => "store",
            FaceError::CredVault(_) => "credvault",
            FaceError::Pipe(_) => "pipe",
            FaceError::NotSupported(_) => "not_supported",
            FaceError::Users(_) => "users",
            FaceError::Hello(_) => "hello",
            FaceError::Other(_) => "other",
        };
        FaceErrorResponse {
            code: code.to_string(),
            message: e.to_string(),
        }
    }
}

impl From<String> for FaceError {
    fn from(s: String) -> Self {
        FaceError::Other(s)
    }
}

impl From<&str> for FaceError {
    fn from(s: &str) -> Self {
        FaceError::Other(s.to_string())
    }
}

impl From<std::io::Error> for FaceError {
    fn from(e: std::io::Error) -> Self {
        FaceError::Other(format!("I/O: {e}"))
    }
}
