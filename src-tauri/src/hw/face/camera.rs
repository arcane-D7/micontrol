//! Webcam capture abstraction for face unlock.
//!
//! Mirrors the reference `face_hello/camera.py`:
//! - Windows uses the **DSHOW** backend (MSMF can block for ~33 min in C++
//!   when the device is unavailable — DSHOW with retry/backoff is safer).
//! - A global capture lock prevents the console (enrollment) and the auth
//!   service from grabbing the camera simultaneously.
//! - `open()` retries with backoff until a frame is readable.
//!
//! When the `face` feature is disabled (no `opencv` crate), this module
//! degrades to a stub so the core library still compiles everywhere.

use crate::hw::face::errors::{FaceError, FaceResult};
use std::sync::atomic::{AtomicBool, Ordering};

/// Global camera contention lock (one Face operation at a time).
static CAMERA_LOCK: AtomicBool = AtomicBool::new(false);

/// One BGR frame (rows × cols × 3). Opaque to the rest of the crate so the
/// backend can be swapped without changing callers.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Pixel data, row-major, BGR order.
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
}

impl Frame {
    /// Convert BGR → RGB in place (for models expecting RGB input).
    pub fn to_rgb(&self) -> Vec<u8> {
        let mut out = self.data.clone();
        for px in out.chunks_exact_mut(3) {
            px.swap(0, 2);
        }
        out
    }
}

/// A camera handle. Drop releases the capture lock and the device.
#[allow(dead_code)] // only exercised when the `face` feature is enabled
pub struct Camera {
    index: u32,
    #[cfg(feature = "face-opencv")]
    inner: Option<opencv::videoio::VideoCapture>,
}

impl Camera {
    /// Acquire the global lock; fails if another Face operation holds it.
    #[allow(dead_code)]
    fn acquire_lock() -> FaceResult<()> {
        if CAMERA_LOCK.swap(true, Ordering::SeqCst) {
            return Err(FaceError::Camera(
                "camera already in use by another face operation".into(),
            ));
        }
        Ok(())
    }

    fn release_lock() {
        CAMERA_LOCK.store(false, Ordering::SeqCst);
    }

    /// Open the camera at `index`, retrying with backoff until a frame reads.
    /// `timeout_s` caps the total retry time.
    #[cfg(feature = "face-opencv")]
    pub fn open(index: u32, timeout_s: f32) -> FaceResult<Self> {
        use std::time::{Duration, Instant};

        Self::acquire_lock()?;
        let start = Instant::now();
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let t0 = Instant::now();
            // DSHOW backend.
            let mut cap = match opencv::videoio::VideoCapture::new(
                index as i32,
                opencv::videoio::CAP_DSHOW,
            ) {
                Ok(c) => c,
                Err(e) => {
                    log::debug!("[face.camera] open attempt {attempt} failed: {e}");
                    cap_fail();
                    continue_or_timeout(start, timeout_s, attempt)?;
                    continue;
                }
            };
            if !cap.is_opened().unwrap_or(false) {
                let _ = cap.release();
                log::debug!("[face.camera] attempt {attempt}: not opened");
                continue_or_timeout(start, timeout_s, attempt)?;
                continue;
            }
            // Try reading one frame.
            match cap.read() {
                Ok(true) => {
                    log::info!("[face.camera] DSHOW ready on attempt {attempt}");
                    return Ok(Self {
                        index,
                        inner: Some(cap),
                    });
                }
                Ok(false) | Err(_) => {
                    let _ = cap.release();
                    log::debug!("[face.camera] attempt {attempt}: read failed");
                    continue_or_timeout(start, timeout_s, attempt)?;
                    continue;
                }
            }
        }
    }

    #[cfg(not(feature = "face-opencv"))]
    pub fn open(_index: u32, _timeout_s: f32) -> FaceResult<Self> {
        Err(FaceError::NotSupported(
            "face feature disabled (camera capture requires the opencv crate)".into(),
        ))
    }

    /// Read one frame. Errors if not opened or the read fails.
    #[cfg(feature = "face-opencv")]
    pub fn read(&mut self) -> FaceResult<Frame> {
        let cap = self
            .inner
            .as_mut()
            .ok_or_else(|| FaceError::Camera("camera not open".into()))?;
        let mut mat = opencv::core::Mat::default();
        match cap.read(&mut mat) {
            Ok(true) => {}
            _ => return Err(FaceError::Camera("failed to read frame".into())),
        }
        let w = mat.cols() as usize;
        let h = mat.rows() as usize;
        let channels = mat.channels() as usize;
        let mut data = vec![0u8; w * h * channels];
        // Copy row-major. opencv Mat may have padding rows (step != cols*ch).
        let step = mat.step1() as usize;
        let raw = mat.data_bytes().unwrap_or(&[]);
        if channels == 3 && step == w * 3 && raw.len() >= w * h * 3 {
            data.copy_from_slice(&raw[..w * h * 3]);
        } else {
            // Handle padded rows.
            for r in 0..h {
                let src = &raw[r * step..r * step + w * channels];
                let dst = &mut data[r * w * channels..(r + 1) * w * channels];
                dst.copy_from_slice(src);
            }
        }
        Ok(Frame {
            data,
            width: w,
            height: h,
        })
    }

    #[cfg(not(feature = "face-opencv"))]
    pub fn read(&mut self) -> FaceResult<Frame> {
        Err(FaceError::NotSupported("face feature disabled".into()))
    }
}

/// Helper: small helper for the retry loop.
#[cfg(feature = "face-opencv")]
fn cap_fail() {
    // No-op placeholder for logging symmetry.
}

#[cfg(feature = "face-opencv")]
fn continue_or_timeout(start: std::time::Instant, timeout_s: f32, attempt: u32) -> FaceResult<()> {
    use std::time::Duration;
    if start.elapsed().as_secs_f32() >= timeout_s {
        return Err(FaceError::Camera(format!(
            "unable to open camera after {timeout_s:.0}s ({attempt} attempts)"
        )));
    }
    std::thread::sleep(Duration::from_millis(300));
    Ok(())
}

impl Drop for Camera {
    fn drop(&mut self) {
        #[cfg(feature = "face-opencv")]
        if let Some(mut cap) = self.inner.take() {
            let _ = cap.release();
        }
        Self::release_lock();
    }
}
