//! Webcam capture abstraction for face unlock.
//!
//! Mirrors the reference `face_hello/camera.py`:
//! - Windows uses the **MSMF** backend via `nokhwa` (pure-Rust Media Foundation
//!   bindings — no C++/libclang required).
//! - A global capture lock prevents the console (enrollment) and the auth
//!   service from grabbing the camera simultaneously.
//! - `open()` retries with backoff until a frame is readable.

use crate::hw::face::errors::{FaceError, FaceResult};
#[cfg(any(feature = "face", feature = "face-opencv"))]
use nokhwa::pixel_format::RgbFormat;
#[cfg(any(feature = "face", feature = "face-opencv"))]
use nokhwa::utils::{ApiBackend, CameraIndex, RequestedFormatType};

/// Global camera contention lock (one Face operation at a time).
static CAMERA_LOCK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Windows Camera Frame Server — MSMF (Media Foundation) capture from a
/// Session-0 SYSTEM service goes through the FrameServer broker. If that
/// service is stopped, the FrameServerClient DLL can be unloaded mid-capture
/// and the calling process crashes with `0xc0000005` on
/// `FrameServerClient.dll_unloaded` (observed in `micontrol_face_svc.exe`).
///
/// Before opening the camera we make sure the FrameServer service is running
/// (idempotent; harmless when it already is, e.g. interactive session with the
/// webcam in use). Returns `true` when it is running (or became running).
pub fn ensure_frame_server() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::System::Services::{
            CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatus, StartServiceW,
            SC_MANAGER_CONNECT, SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_START,
            SERVICE_START_PENDING, SERVICE_STATUS, SERVICE_STOPPED,
        };
        // SAFETY: simple SCM connect with read + start access.
        let mgr = match unsafe { OpenSCManagerW(None, None, SC_MANAGER_CONNECT) } {
            Ok(m) => m,
            Err(_) => return false,
        };
        let name: Vec<u16> = "FrameServer\0".encode_utf16().collect();
        // SAFETY: mgr valid, name null-terminated.
        let svc = unsafe {
            OpenServiceW(
                mgr,
                windows::core::PCWSTR(name.as_ptr()),
                SERVICE_QUERY_STATUS | SERVICE_START,
            )
        };
        if let Ok(svc) = svc {
            // SAFETY: status buffer valid.
            let mut si = SERVICE_STATUS::default();
            let st = unsafe { QueryServiceStatus(svc, &mut si) };
            if st.is_ok() {
                match si.dwCurrentState {
                    SERVICE_RUNNING => {
                        unsafe {
                            CloseServiceHandle(svc).ok();
                        }
                        unsafe {
                            CloseServiceHandle(mgr).ok();
                        }
                        return true;
                    }
                    SERVICE_STOPPED | SERVICE_START_PENDING => {
                        // SAFETY: no args — service takes none.
                        let _ = unsafe { StartServiceW(svc, None) };
                        let _ = unsafe { CloseServiceHandle(svc) };
                        let _ = unsafe { CloseServiceHandle(mgr) };
                        // Wait briefly for the broker to reach RUNNING.
                        for _ in 0..20 {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            if frame_server_running() {
                                return true;
                            }
                        }
                        return frame_server_running();
                    }
                    _ => {
                        let _ = unsafe { CloseServiceHandle(svc) };
                        let _ = unsafe { CloseServiceHandle(mgr) };
                        return true; // unknown state — try anyway
                    }
                }
            }
            let _ = unsafe { CloseServiceHandle(svc) };
            let _ = unsafe { CloseServiceHandle(mgr) };
            return st.is_ok();
        }
        unsafe {
            CloseServiceHandle(mgr).ok();
        }
        false
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// Cheap probe: is the FrameServer service running right now?
#[cfg(windows)]
fn frame_server_running() -> bool {
    use windows::Win32::System::Services::{
        CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatus, SC_MANAGER_CONNECT,
        SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_STATUS,
    };
    // SAFETY: connect + query only.
    let Ok(mgr) = (unsafe { OpenSCManagerW(None, None, SC_MANAGER_CONNECT) }) else {
        return false;
    };
    let name: Vec<u16> = "FrameServer\0".encode_utf16().collect();
    // SAFETY: mgr valid.
    let Ok(svc) = (unsafe {
        OpenServiceW(
            mgr,
            windows::core::PCWSTR(name.as_ptr()),
            SERVICE_QUERY_STATUS,
        )
    }) else {
        unsafe {
            CloseServiceHandle(mgr).ok();
        }
        return false;
    };
    // SAFETY: status buffer valid.
    let mut si = SERVICE_STATUS::default();
    let running =
        unsafe { QueryServiceStatus(svc, &mut si) }.is_ok() && si.dwCurrentState == SERVICE_RUNNING;
    unsafe {
        CloseServiceHandle(svc).ok();
        CloseServiceHandle(mgr).ok();
    }
    running
}

/// One RGB frame (rows × cols × 3). Opaque to the rest of the crate so the
/// backend can be swapped without changing callers.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Pixel data, row-major, RGB order.
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
}

impl Frame {
    /// Convert RGB → BGR in place (for models expecting BGR input).
    pub fn to_bgr(&self) -> Vec<u8> {
        let mut out = self.data.clone();
        for px in out.chunks_exact_mut(3) {
            px.swap(0, 2);
        }
        out
    }
}

/// Number of cameras currently enumerable (for UI validation).
#[allow(dead_code)]
pub fn camera_count() -> usize {
    #[cfg(any(feature = "face", feature = "face-opencv"))]
    {
        match nokhwa::query(ApiBackend::MediaFoundation) {
            Ok(cams) => cams.len(),
            Err(_) => 0,
        }
    }
    #[cfg(not(any(feature = "face", feature = "face-opencv")))]
    {
        0
    }
}

/// A camera handle. Drop releases the capture lock and the device.
pub struct Camera {
    index: u32,
    inner: Option<nokhwa::Camera>,
    #[allow(dead_code)]
    opened_once: bool,
}

impl Camera {
    /// The camera index this handle was opened with.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// Acquire the global lock; fails if another Face operation holds it.
    fn acquire_lock() -> FaceResult<()> {
        use std::sync::atomic::Ordering;
        if CAMERA_LOCK.swap(true, Ordering::SeqCst) {
            return Err(FaceError::Camera(
                "camera already in use by another face operation".into(),
            ));
        }
        Ok(())
    }

    fn release_lock() {
        use std::sync::atomic::Ordering;
        CAMERA_LOCK.store(false, Ordering::SeqCst);
    }

    /// Open the camera at `index`, retrying with backoff until a frame reads.
    /// `timeout_s` caps the total retry time.
    pub fn open(index: u32, timeout_s: f32) -> FaceResult<Self> {
        Self::acquire_lock()?;
        // Make sure the Windows Camera Frame Server is up (Session-0 SYSTEM
        // services crash with FrameServerClient.dll_unloaded if it is not).
        if !ensure_frame_server() {
            log::warn!("[face.camera] FrameServer unavailable — capture may fail");
        }
        let start = std::time::Instant::now();
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            match Self::try_open(index) {
                Ok(cam) => {
                    log::info!("[face.camera] MSMF ready on attempt {attempt}");
                    return Ok(cam);
                }
                Err(e) => {
                    log::debug!("[face.camera] open attempt {attempt} failed: {e}");
                    if start.elapsed().as_secs_f32() >= timeout_s {
                        // CRITICAL: release the global lock on failure — the
                        // lock is otherwise only released in `Drop`, which
                        // never runs when `open()` returns Err. A stuck lock
                        // would make every later camera operation fail with
                        // "camera already in use by another face operation".
                        Self::release_lock();
                        return Err(FaceError::Camera(format!(
                            "unable to open camera after {timeout_s:.0}s ({attempt} attempts): {e}"
                        )));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(350));
                }
            }
        }
    }

    #[cfg(any(feature = "face", feature = "face-opencv"))]
    fn try_open(index: u32) -> FaceResult<Self> {
        use nokhwa::utils::{CameraFormat, FrameFormat, RequestedFormat};

        let camid = nokhwa::query(ApiBackend::MediaFoundation)
            .map(|cams| {
                cams.get(index as usize)
                    .map(|info| info.index().clone())
                    .unwrap_or(CameraIndex::Index(index))
            })
            .unwrap_or(CameraIndex::Index(index));

        // Try progressive strategies — some webcams don't offer MJPEG at the
        // requested resolution, and `Closest` fails hard ("Failed to fulfill
        // requested format") when the decoder's format set is absent entirely.
        let strategies: Vec<RequestedFormat> = vec![
            // 1. Preferred: MJPEG 720p (fast, hardware-accelerated).
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(
                CameraFormat::new_from(1280, 720, FrameFormat::MJPEG, 30),
            )),
            // 2. Highest resolution the device actually offers.
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestResolution),
            // 3. Whatever the camera can do.
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::None),
        ];

        let mut last_err = String::new();
        for req in strategies {
            match nokhwa::Camera::with_backend(camid.clone(), req, ApiBackend::MediaFoundation) {
                Ok(mut cam) => {
                    let opened = cam.open_stream().and_then(|_| {
                        // Try one read to confirm the device actually produces frames.
                        cam.frame().map(|_| ())
                    });
                    match opened {
                        Ok(()) => {
                            return Ok(Self {
                                index,
                                inner: Some(cam),
                                opened_once: true,
                            });
                        }
                        Err(e) => {
                            last_err = format!("nokhwa stream/frame: {e}");
                            let _ = cam.stop_stream();
                        }
                    }
                }
                Err(e) => {
                    last_err = format!("nokhwa open: {e}");
                }
            }
        }
        Err(FaceError::Camera(format!(
            "all capture strategies failed: {last_err}"
        )))
    }

    #[cfg(not(any(feature = "face", feature = "face-opencv")))]
    fn try_open(_index: u32) -> FaceResult<Self> {
        Err(FaceError::NotSupported(
            "face feature disabled (camera capture requires nokhwa)".into(),
        ))
    }

    /// Read one full RGB frame (blocking).
    #[cfg(any(feature = "face", feature = "face-opencv"))]
    pub fn read(&mut self) -> FaceResult<Frame> {
        use nokhwa::pixel_format::RgbFormat;
        let cam = self
            .inner
            .as_mut()
            .ok_or_else(|| FaceError::Camera("camera not open".into()))?;
        let buf = cam
            .frame()
            .map_err(|e| FaceError::Camera(format!("read: {e}")))?;
        let res = buf.resolution();
        let img = buf
            .decode_image::<RgbFormat>()
            .map_err(|e| FaceError::Camera(format!("decode: {e}")))?;
        Ok(Frame {
            data: img.into_raw(),
            width: res.width() as usize,
            height: res.height() as usize,
        })
    }

    #[cfg(not(any(feature = "face", feature = "face-opencv")))]
    pub fn read(&mut self) -> FaceResult<Frame> {
        Err(FaceError::NotSupported("face feature disabled".into()))
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        if let Some(mut cam) = self.inner.take() {
            let _ = cam.stop_stream();
        }
        Self::release_lock();
    }
}
