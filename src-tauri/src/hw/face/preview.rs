//! Live webcam preview for the enrollment wizard.
//!
//! The nokhwa `Camera` is not `Send`, so a small dedicated capture thread
//! owns the camera and publishes the latest frame as a JPEG byte buffer.
//! The frontend polls `face_camera_preview_frame` (~10 Hz) and renders an
//! `<img src="data:image/jpeg;base64,...">`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::thread::JoinHandle;

use super::camera::Camera;
use super::errors::{FaceError, FaceResult};

/// Latest JPEG frame + its dimensions (for aspect-ratio CSS).
#[derive(Debug, Clone)]
pub struct PreviewFrame {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

struct PreviewState {
    frame: Mutex<Option<PreviewFrame>>,
    running: AtomicBool,
    last_err: Mutex<Option<String>>,
    /// Handle to the current capture thread, if any. `start` joins the prior
    /// thread before spawning a new one so the camera lock is always released
    /// (React StrictMode in dev mounts/unmounts the modal twice, which would
    /// otherwise leave two threads fighting over the same webcam).
    handle: Mutex<Option<JoinHandle<()>>>,
}

static STATE: LazyLock<PreviewState> = LazyLock::new(|| PreviewState {
    frame: Mutex::new(None),
    running: AtomicBool::new(false),
    last_err: Mutex::new(None),
    handle: Mutex::new(None),
});

/// Start (or restart) the preview capture thread on the given camera index.
/// Blocks until the previous thread (if any) has exited, then spawns a new
/// one. The first usable frame may take ~1s.
pub fn start(index: u32) -> FaceResult<()> {
    stop_and_join();

    *STATE.frame.lock().unwrap() = None;
    *STATE.last_err.lock().unwrap() = None;
    STATE.running.store(true, Ordering::SeqCst);

    std::thread::Builder::new()
        .name("face-preview".into())
        .spawn(move || {
            let mut camera = match Camera::open(index, 6.0) {
                Ok(c) => c,
                Err(e) => {
                    *STATE.last_err.lock().unwrap() = Some(format!("open: {e}"));
                    return;
                }
            };
            while STATE.running.load(Ordering::SeqCst) {
                match camera.read() {
                    Ok(cap) => {
                        // Encode RGB → JPEG (quality 72 keeps the preview snappy
                        // even at 720p).
                        let img = image::ImageBuffer::<image::Rgb<u8>, _>::from_raw(
                            cap.width as u32,
                            cap.height as u32,
                            cap.data,
                        );
                        if let Some(img) = img {
                            let dynimg = image::DynamicImage::ImageRgb8(img);
                            let mut buf = std::io::Cursor::new(Vec::new());
                            {
                                let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(
                                    &mut buf, 72,
                                );
                                if enc.encode_image(&dynimg).is_ok() {
                                    let pv = PreviewFrame {
                                        jpeg: buf.get_ref().clone(),
                                        width: cap.width as u32,
                                        height: cap.height as u32,
                                    };
                                    *STATE.frame.lock().unwrap() = Some(pv);
                                    *STATE.last_err.lock().unwrap() = None;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        *STATE.last_err.lock().unwrap() = Some(format!("read: {e}"));
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
            // Thread exits → camera dropped here, releasing the capture lock.
        })
        .map(|handle| {
            *STATE.handle.lock().unwrap() = Some(handle);
        })
        .map_err(|e| FaceError::Camera(format!("spawn preview: {e}")))?;

    Ok(())
}

/// Stop the preview thread and release the camera (fires-and-forgets join;
/// use [`stop_and_join`] when the caller itself wants to block).
pub fn stop() {
    stop_and_join();
}

/// Set the running flag to false and wait for the capture thread to exit.
/// This guarantees the camera (and its global lock) is released before
/// returning, which is required before re-acquiring it in `start`.
fn stop_and_join() {
    STATE.running.store(false, Ordering::SeqCst);
    let handle = STATE.handle.lock().unwrap().take();
    if let Some(h) = handle {
        // The thread checks the flag every ~80 ms; joining bounds the wait.
        // Camera::open retries internally for up to 6 s, so give it slack.
        let _ = h.join();
    }
    *STATE.frame.lock().unwrap() = None;
}

/// Fetch the latest preview frame (None if not started / no frame yet).
pub fn latest() -> Option<PreviewFrame> {
    STATE.frame.lock().unwrap().clone()
}

/// Last error from the capture thread (None when healthy).
pub fn last_error() -> Option<String> {
    STATE.last_err.lock().unwrap().clone()
}
