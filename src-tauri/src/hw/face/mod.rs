//! Face unlock module — Windows Hello-style face recognition using an RGB
//! webcam, implemented natively in Rust.
//!
//! This is a reimplementation (in Rust) of the approach validated by
//! `everglow01/Windows-Face-Hello` (v1.0.6) and related open-source projects.
//! It does NOT make Windows Hello itself accept a non-IR webcam — instead it
//! provides a custom Credential Provider + LocalSystem auth service that
//! delivers the same "look at the camera to unlock" experience.
//!
//! ## Components
//!
//! - [`matcher`] — cosine similarity + margin (anti-misrouting) matching.
//! - [`store`] — DPAPI machine-scope encrypted face gallery (feature vectors).
//! - [`liveness`] — EAR blink + head-pose challenge math (no model deps).
//! - [`config`] — defaults for thresholds, lockout, liveness, anti-spoof.
//! - [`models`] — ONNX model loading & preprocessing (detection + embedding).
//! - [`camera`] — webcam capture abstraction (Windows DSHOW backend).
//! - [`service`] — the auth state machine (liveness → antispoof → recognize).
//! - [`credvault`] — LSA Secret read/write for the sign-in password.
//!
//! ## Security model (from the reference implementation)
//!
//! - The sign-in password is stored in an **LSA Secret** and read only by the
//!   Credential Provider in SYSTEM context — it never crosses the named pipe.
//! - The face gallery stores **feature vectors only** (no photos), encrypted
//!   with DPAPI machine scope so the SYSTEM service can decrypt what the
//!   admin console wrote.
//! - The auth pipe DACL allows only SYSTEM + Administrators, and the
//!   Credential Provider verifies the server's process SID is LocalSystem
//!   before sending anything.
//! - Lockout: 5 consecutive biometric failures lock the service for 30 s;
//!   the CP gives 3 attempts then falls back to password/PIN.

pub mod camera;
pub mod config;
pub mod credvault;
pub mod errors;
pub mod liveness;
pub mod matcher;
pub mod models;
pub mod pipe_server;
pub mod preview;
pub mod service;
pub mod store;
pub mod users;

pub use config::FaceSettings;
pub use errors::{FaceError, FaceResult};

/// Fixed embedding dimensionality (ArcFace / InsightFace normed embeddings).
pub const EMBEDDING_DIM: usize = 512;
/// Fixed embedding dimensionality for SFace (OpenCV Zoo) — 128-d.
pub const SFACE_EMBEDDING_DIM: usize = 128;
