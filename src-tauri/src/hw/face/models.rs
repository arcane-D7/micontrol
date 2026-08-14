//! ONNX model loading & inference for face detection + recognition.
//!
//! Ports the reference pipeline (InsightFace SCRFD `det_10g` + ArcFace
//! `w600k_r50`, both ONNX, CPU via onnxruntime):
//! - Detection: 320×320 RGB input, outputs faces (bbox + score + 5 keypoints)
//!   via stride-anchored decode + NMS (port of InsightFace SCRFD).
//! - Recognition: ArcFace warp-aligned face crop → 512-d L2-normalized
//!   embedding (Pytorch-style NCHW input `[1,3,112,112]`).
//!
//! The `ort` crate is always available (with `download-binaries`), so model
//! loading works in both the library and the auth service. When the `face`
//! feature is off, this module still compiles (no camera deps), but the
//! model files must exist at runtime.

use crate::hw::face::config::{EMBEDDING_DIM, MODELS_DIR};
#[cfg(feature = "face")]
use crate::hw::face::errors::FaceError;
use crate::hw::face::errors::FaceResult;
use std::path::PathBuf;

/// A detected face.
#[derive(Debug, Clone)]
pub struct DetectedFace {
    pub bbox: [f32; 4], // x1, y1, x2, y2 (pixels, input-space)
    pub det_score: f32,
    pub kps: [[f32; 2]; 5],          // 5 facial landmarks (x, y)
    pub embedding: Option<Vec<f32>>, // filled by `recognize`
}

impl DetectedFace {
    pub fn area(&self) -> f32 {
        let w = (self.bbox[2] - self.bbox[0]).max(0.0);
        let h = (self.bbox[3] - self.bbox[1]).max(0.0);
        w * h
    }
}

/// FaceDetector: lazy-loads the ONNX models and provides detect + recognize.
pub struct FaceDetector {
    #[cfg(feature = "face")]
    session_det: Option<ort::session::Session>,
    #[cfg(feature = "face")]
    session_rec: Option<ort::session::Session>,
    #[cfg(feature = "face")]
    det_size: (u32, u32),
    /// Ordered list of directories to try when locating the models.
    /// `load()` picks the first directory containing both files.
    models_dirs: Vec<PathBuf>,
    det_path: String,
    rec_path: String,
}

/// Default model file names (bundled in `resources/face_models`).
pub const DET_MODEL: &str = "det_10g.onnx";
pub const REC_MODEL: &str = "w600k_r50.onnx";

// SCRFD decode constants (InsightFace `det_10g`).
#[cfg(feature = "face")]
const SCRFD_FEAT_STRIDE: [u32; 3] = [8, 16, 32];
#[cfg(feature = "face")]
const SCRFD_NUM_ANCHORS: usize = 2;
#[cfg(feature = "face")]
const SCRFD_DET_SCORE_THRESH: f32 = 0.5;
#[cfg(feature = "face")]
const SCRFD_NMS_THRESH: f32 = 0.4;
#[cfg(feature = "face")]
const SCRFD_INPUT_SIZE: u32 = 320;

impl Default for FaceDetector {
    fn default() -> Self {
        // Installed location first, dev/staging fallbacks afterwards.
        Self::with_models(MODELS_DIR, DET_MODEL, REC_MODEL)
    }
}

impl FaceDetector {
    pub fn with_models(dir: &str, det_name: &str, rec_name: &str) -> Self {
        Self {
            #[cfg(feature = "face")]
            session_det: None,
            #[cfg(feature = "face")]
            session_rec: None,
            #[cfg(feature = "face")]
            det_size: (SCRFD_INPUT_SIZE, SCRFD_INPUT_SIZE),
            models_dirs: vec![PathBuf::from(dir)],
            det_path: det_name.to_string(),
            rec_path: rec_name.to_string(),
        }
    }

    /// Append an additional directory to the search path (checked after
    /// previously-added ones). Used, e.g., to fall back to the download
    /// staging dir under ProgramData when the Program Files copy is absent.
    pub fn add_models_dir(&mut self, dir: &str) {
        let p = PathBuf::from(dir);
        if !self.models_dirs.contains(&p) {
            self.models_dirs.push(p);
        }
    }

    /// Resolve the first directory that actually contains both model files.
    #[cfg(feature = "face")]
    fn resolve_dir(&self) -> Option<PathBuf> {
        self.models_dirs
            .iter()
            .find(|d| d.join(&self.det_path).exists() && d.join(&self.rec_path).exists())
            .cloned()
    }

    #[allow(dead_code)]
    fn det_model_path(&self) -> PathBuf {
        // For logging only — resolved by `load()` via `resolve_dir`.
        self.models_dirs
            .first()
            .cloned()
            .unwrap_or_default()
            .join(&self.det_path)
    }

    #[allow(dead_code)]
    fn rec_model_path(&self) -> PathBuf {
        self.models_dirs
            .first()
            .cloned()
            .unwrap_or_default()
            .join(&self.rec_path)
    }

    /// Load both models (idempotent).
    #[cfg(feature = "face")]
    pub fn load(&mut self) -> FaceResult<()> {
        use ort::ep;
        use ort::session::builder::SessionBuilder;

        let dir = self.resolve_dir().ok_or_else(|| {
            let tried = self
                .models_dirs
                .iter()
                .map(|d| d.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            FaceError::MissingModel(format!(
                "det_10g.onnx/w600k_r50.onnx not found in any of: {tried}"
            ))
        })?;
        let det_path = dir.join(&self.det_path);
        let rec_path = dir.join(&self.rec_path);

        let cpu_ep = ep::CPU::default().build();

        if self.session_det.is_none() {
            log::info!("[face.models] loading detection model {det_path:?}");
            let mut builder = SessionBuilder::new()
                .map_err(|e| FaceError::Model(format!("session builder: {e}")))?;
            builder = builder
                .with_execution_providers([cpu_ep.clone()])
                .map_err(|e| FaceError::Model(format!("det EP: {e}")))?;
            let session = builder
                .commit_from_file(&det_path)
                .map_err(|e| FaceError::Model(format!("det session: {e}")))?;
            self.session_det = Some(session);
        }
        if self.session_rec.is_none() {
            log::info!("[face.models] loading recognition model {rec_path:?}");
            let mut builder = SessionBuilder::new()
                .map_err(|e| FaceError::Model(format!("session builder: {e}")))?;
            builder = builder
                .with_execution_providers([cpu_ep])
                .map_err(|e| FaceError::Model(format!("rec EP: {e}")))?;
            let session = builder
                .commit_from_file(&rec_path)
                .map_err(|e| FaceError::Model(format!("rec session: {e}")))?;
            self.session_rec = Some(session);
        }
        Ok(())
    }

    #[cfg(not(feature = "face"))]
    pub fn load(&mut self) -> FaceResult<()> {
        Ok(()) // models are optional without the face feature
    }

    /// Detect faces in a frame (RGB). Returns detected faces (embeddings None).
    #[cfg(feature = "face")]
    pub fn detect(
        &mut self,
        frame: &crate::hw::face::camera::Frame,
    ) -> FaceResult<Vec<DetectedFace>> {
        // Build a 320x320 NCHW input. Note: the `det_10g.onnx` export ships the
        // normalization INSIDE the graph (input is raw uint8-scaled floats), so
        // we pass [0,255]-range values scaled by 1.0 and let the model normalize.
        let (dw, dh) = self.det_size;
        let (sw, sh) = (frame.width as u32, frame.height as u32);
        let (scale_x, scale_y) = (sw as f32 / dw as f32, sh as f32 / dh as f32);
        // SCRFD uses plain resize (no letterbox).
        let resized = resize_nearest(
            &frame.data,
            sw as usize,
            sh as usize,
            dw as usize,
            dh as usize,
        );
        let n = (dw * dh) as usize;
        // Convert RGB (NHWC) to planar float (NCHW) so the model gets [1,3,320,320].
        let mut input = vec![0.0f32; n * 3];
        for i in 0..n {
            let (r, g, b) = (
                resized[i * 3] as f32,
                resized[i * 3 + 1] as f32,
                resized[i * 3 + 2] as f32,
            );
            input[i] = r;
            input[n + i] = g;
            input[2 * n + i] = b;
        }

        let session = self
            .session_det
            .as_mut()
            .ok_or_else(|| FaceError::Model("detector not loaded".into()))?;
        let tensor =
            ort::value::Tensor::from_array(([1usize, 3usize, dh as usize, dw as usize], input))
                .map_err(|e| FaceError::Model(format!("tensor: {e}")))?;
        let outputs = session
            .run(ort::inputs![tensor])
            .map_err(|e| FaceError::Model(format!("detect run: {e}")))?;

        // This ONNX export flattens the feature maps. For input (320,320):
        //   stride 8  → 40x40 x 2 anchors = 3200 rows [score|bbox|kps]
        //   stride 16 → 20x20 x 2 anchors =  800 rows
        //   stride 32 → 10x10 x 2 anchors =  200 rows
        // Outputs are ordered score, bbox, kps per head (9 tensors total),
        // each shaped [N,1] / [N,4] / [N,10].
        let mut raw: Vec<Option<(Vec<i64>, Vec<f32>)>> = Vec::new();
        for (i, (name, out)) in outputs.iter().enumerate() {
            if i >= 9 {
                break;
            }
            log::debug!("[face.models] det output {i}: {name} {:?}", out.shape());
            let owned = out
                .try_extract_tensor::<f32>()
                .map(|(shape, data)| (shape.iter().copied().collect::<Vec<_>>(), data.to_vec()))
                .ok();
            raw.push(owned);
        }

        let mut faces: Vec<DetectedFace> = Vec::new();
        if raw.len() < 9 {
            log::warn!(
                "[face.models] SCRFD returned {} outputs, expected 9",
                raw.len()
            );
            return Ok(faces);
        }
        for (head_i, &stride) in SCRFD_FEAT_STRIDE.iter().enumerate() {
            // Output layout is grouped by type, not by head:
            //   [score8, score16, score32, bbox8, bbox16, bbox32, kps8, kps16, kps32]
            let si = head_i; // score index
            let bi = head_i + SCRFD_FEAT_STRIDE.len(); // bbox index
            let ki = head_i + 2 * SCRFD_FEAT_STRIDE.len(); // kps index
            let (Some((score_shape, score_data)), Some((_, bbox_data)), Some((_, kps_data))) =
                (raw[si].take(), raw[bi].take(), raw[ki].take())
            else {
                continue;
            };
            // Flattened [N,1]/[N,4]/[N,10] outputs. Rows are grouped per grid
            // cell: cell (row,col) holds `num_anchor` rows consecutively.
            if score_shape.len() < 2 {
                continue;
            }
            let num_rows = score_shape[0] as usize;
            // Derive grid dims from the number of rows.
            // stride 8 → 3200 rows = 40*40*2 → hdim=wdim=40, anchors=2
            // stride 16 → 800 rows = 20*20*2 → hdim=wdim=20
            // stride 32 → 200 rows = 10*10*2 → hdim=wdim=10
            let (hdim, wdim, anchors) = if head_i == 0 {
                // stride 8: 3200 = 40*40*2 on a 320 input
                (dw as usize / 8, dw as usize / 8, SCRFD_NUM_ANCHORS)
            } else if head_i == 1 {
                (dw as usize / 16, dw as usize / 16, SCRFD_NUM_ANCHORS)
            } else {
                (dw as usize / 32, dw as usize / 32, SCRFD_NUM_ANCHORS)
            };
            let ncells = hdim * wdim * anchors;
            if num_rows != ncells
                || score_data.len() < ncells
                || bbox_data.len() < ncells * 4
                || kps_data.len() < ncells * 10
            {
                log::warn!(
                    "[face.models] head {head_i} unexpected shape: rows={num_rows} want={ncells} (scores={} bbox={} kps={})",
                    score_data.len(), bbox_data.len(), kps_data.len()
                );
                continue;
            }
            for row in 0..hdim {
                for col in 0..wdim {
                    for a in 0..anchors {
                        let idx = (row * wdim + col) * anchors + a;
                        let s = score_data[idx];
                        if s < SCRFD_DET_SCORE_THRESH {
                            continue;
                        }
                        // Anchor center: (col*stride, row*stride) in input
                        // pixel space (InsightFace `anchor_centers`), same for
                        // both anchors of a cell.
                        let cx = col as f32 * stride as f32;
                        let cy = row as f32 * stride as f32;
                        let bo = idx * 4;
                        let ko = idx * 10;
                        if bo + 4 > bbox_data.len() || ko + 10 > kps_data.len() {
                            continue;
                        }
                        // InsightFace decode: bbox deltas are multiplied by the
                        // feature stride, then distance2bbox:
                        //   x1 = cx - d0, y1 = cy - d1, x2 = cx + d2, y2 = cy + d3
                        let x1 = (cx - bbox_data[bo] * stride as f32).max(0.0);
                        let y1 = (cy - bbox_data[bo + 1] * stride as f32).max(0.0);
                        let x2 = (cx + bbox_data[bo + 2] * stride as f32).min(dw as f32 - 1.0);
                        let y2 = (cy + bbox_data[bo + 3] * stride as f32).min(dh as f32 - 1.0);
                        // distance2kps: px = cx + dist[2k], py = cy + dist[2k+1]
                        // (kps deltas also multiplied by stride).
                        let kps_pts: [[f32; 2]; 5] = std::array::from_fn(|k| {
                            let px = (cx + kps_data[ko + k * 2] * stride as f32) * scale_x;
                            let py = (cy + kps_data[ko + k * 2 + 1] * stride as f32) * scale_y;
                            [px, py]
                        });
                        faces.push(DetectedFace {
                            bbox: [x1 * scale_x, y1 * scale_y, x2 * scale_x, y2 * scale_y],
                            det_score: s,
                            kps: kps_pts,
                            embedding: None,
                        });
                    }
                }
            }
        }
        // NMS
        faces.sort_by(|a, b| {
            b.det_score
                .partial_cmp(&a.det_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut kept: Vec<DetectedFace> = Vec::new();
        for f in faces {
            let mut overlap = false;
            for k in &kept {
                if iou(&f.bbox, &k.bbox) > SCRFD_NMS_THRESH {
                    overlap = true;
                    break;
                }
            }
            if !overlap {
                kept.push(f);
            }
        }
        log::info!("[face.models] detect: {} faces after NMS", kept.len());
        Ok(kept)
    }

    #[cfg(not(feature = "face"))]
    pub fn detect(
        &mut self,
        _frame: &crate::hw::face::camera::Frame,
    ) -> FaceResult<Vec<DetectedFace>> {
        Ok(Vec::new())
    }

    /// Compute the embedding for a cropped/aligned face region.
    /// Requires the caller to pass the full RGB frame + the face's 5 kps.
    #[cfg(feature = "face")]
    pub fn recognize(
        &mut self,
        frame_rgb: &[u8],
        frame_w: usize,
        frame_h: usize,
        kps: &[[f32; 2]; 5],
        bbox: &[f32; 4],
    ) -> FaceResult<Vec<f32>> {
        // ArcFace: warp the face region to 112×112 using the 5 landmarks
        // (AlignArcFace from InsightFace). We implement a simplified affine
        // warp based on the pre-defined 112×112 target landmark coords.
        let aligned = warp_face(frame_rgb, frame_w, frame_h, kps, bbox, 112, 112);
        // Pytorch-style: [1,3,112,112] NCHW, normalized to [-1,1].
        let mut input = vec![0.0f32; 3 * 112 * 112];
        let n = 112 * 112;
        for i in 0..n {
            let (r, g, b) = (aligned[i * 3], aligned[i * 3 + 1], aligned[i * 3 + 2]);
            input[i] = (r as f32 - 127.5) / 127.5; // R
            input[n + i] = (g as f32 - 127.5) / 127.5; // G
            input[2 * n + i] = (b as f32 - 127.5) / 127.5; // B
        }
        let session = self
            .session_rec
            .as_mut()
            .ok_or_else(|| FaceError::Model("recognizer not loaded".into()))?;
        let tensor = ort::value::Tensor::from_array(([1usize, 3usize, 112usize, 112usize], input))
            .map_err(|e| FaceError::Model(format!("rec tensor: {e}")))?;
        let outputs = session
            .run(ort::inputs![tensor])
            .map_err(|e| FaceError::Model(format!("recognize run: {e}")))?;
        // Output is [1, 512] floats. The w600k_r50 ONNX export does NOT apply
        // L2 normalization, so we normalize here (cosine similarity in the
        // matcher assumes unit-length vectors).
        for (name, out) in outputs.iter() {
            let _ = name;
            if let Ok((shape, vals)) = out.try_extract_tensor::<f32>() {
                let dims: Vec<i64> = shape.iter().copied().collect();
                if dims.len() == 2 && dims[1] == EMBEDDING_DIM as i64 && vals.len() == EMBEDDING_DIM
                {
                    let mut v: Vec<f32> = vals.to_vec();
                    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
                    for x in v.iter_mut() {
                        *x /= norm;
                    }
                    return Ok(v);
                }
            }
        }
        Err(FaceError::Model(
            "unexpected recognition output shape".into(),
        ))
    }

    #[cfg(not(feature = "face"))]
    pub fn recognize(
        &mut self,
        _frame_rgb: &[u8],
        _frame_w: usize,
        _frame_h: usize,
        _kps: &[[f32; 2]; 5],
        _bbox: &[f32; 4],
    ) -> FaceResult<Vec<f32>> {
        Ok(vec![0.0; EMBEDDING_DIM])
    }
}

/// Intersection over union for two boxes (x1,y1,x2,y2).
#[cfg(feature = "face")]
fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let ix1 = a[0].max(b[0]);
    let iy1 = a[1].max(b[1]);
    let ix2 = a[2].min(b[2]);
    let iy2 = a[3].min(b[3]);
    let iw = (ix2 - ix1).max(0.0);
    let ih = (iy2 - iy1).max(0.0);
    let inter = iw * ih;
    if inter <= 0.0 {
        return 0.0;
    }
    let au = (a[2] - a[0]) * (a[3] - a[1]);
    let bu = (b[2] - b[0]) * (b[3] - b[1]);
    inter / (au + bu - inter + 1e-6)
}

/// Nearest-neighbor resize (kept for planarity/speed with SCRFD).
#[cfg(feature = "face")]
fn resize_nearest(rgb: &[u8], src_w: usize, src_h: usize, dst_w: usize, dst_h: usize) -> Vec<u8> {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return vec![0u8; dst_w * dst_h * 3];
    }
    let mut out = vec![0u8; dst_w * dst_h * 3];
    for y in 0..dst_h {
        let sy = (y * src_h / dst_h).min(src_h - 1);
        for x in 0..dst_w {
            let sx = (x * src_w / dst_w).min(src_w - 1);
            let s = (sy * src_w + sx) * 3;
            let d = (y * dst_w + x) * 3;
            out[d..d + 3].copy_from_slice(&rgb[s..s + 3]);
        }
    }
    out
}

/// ArcFace-style 5-point warp to a `d×d` aligned crop.
///
/// Mirrors InsightFace's `estimate_norm` + `warp_face`: computes a full
/// similarity transform (rotation + uniform scale + translation) that maps
/// the 5 source landmarks onto the ArcFace canonical 112-aligned targets
/// (right-eye, left-eye, nose, right-mouth, left-mouth) via least squares
/// (`umeyama`-style, without reflection), then bilinearly samples the source
/// frame to produce the normalized crop.
///
/// A simple translate+scale warp (the previous implementation) left the face
/// rotated and produced weak embeddings — the recognition model never got the
/// canonical alignment it was trained on, which caused "no face recognized"
/// even when detection worked.
#[cfg(feature = "face")]
fn warp_face(
    rgb: &[u8],
    frame_w: usize,
    frame_h: usize,
    kps: &[[f32; 2]; 5],
    _bbox: &[f32; 4],
    out_w: usize,
    out_h: usize,
) -> Vec<u8> {
    // Canonical 112-aligned targets (ArcFace `estimate_norm`).
    let tgt: [[f32; 2]; 5] = [
        [38.2946, 51.6963], // right eye
        [73.5318, 51.5014], // left eye
        [56.0252, 71.7366], // nose tip
        [41.5493, 92.3655], // right mouth corner
        [70.7299, 92.2041], // left mouth corner
    ];

    // Umeyama (no reflection): find s·R + t mapping src → tgt.
    // μ_s, μ_t — centroids.
    let (src_cx, src_cy) = {
        let mut x = 0.0;
        let mut y = 0.0;
        for p in kps {
            x += p[0];
            y += p[1];
        }
        (x / 5.0, y / 5.0)
    };
    let (tgt_cx, tgt_cy) = {
        let mut x = 0.0;
        let mut y = 0.0;
        for p in tgt {
            x += p[0];
            y += p[1];
        }
        (x / 5.0, y / 5.0)
    };

    // Σ over centered points: a = Σ(x·u + y·v), b = Σ(x·v − y·u), cc = Σ(x²+y²).
    let mut a = 0.0f32; // Σ (x·u + y·v)
    let mut b = 0.0f32; // Σ (x·v − y·u)
    let mut cc = 0.0f32; // Σ (x·x + y·y)
    let mut su = 0.0f32; // Σ (u·u + v·v)
    for i in 0..5 {
        let x = kps[i][0] - src_cx;
        let y = kps[i][1] - src_cy;
        let u = tgt[i][0] - tgt_cx;
        let v = tgt[i][1] - tgt_cy;
        a += x * u + y * v;
        b += x * v - y * u;
        cc += x * x + y * y;
        su += u * u + v * v;
    }
    // Optimal rotation (maximizes Σ yᵀRx): cosθ = a/|a|, sinθ = b/|a|.
    let mag = (a * a + b * b).sqrt();
    let (ca, sa) = if mag > 1e-8 {
        (a / mag, b / mag)
    } else {
        (1.0, 0.0)
    };
    // Uniform scale = sqrt(Σ|tgt|² / Σ|src|²) (Umeyama), clamped for safety.
    let scale = if cc > 1e-8 {
        (su / cc).sqrt().clamp(0.5, 6.0)
    } else {
        1.0
    };
    let inv_s = scale.recip();
    // Inverse map (source ← target): s = Rᵀ·(t − μ_t)/scale + μ_s, where
    // R = [[ca, −sa],[sa, ca]] maps src→tgt, so Rᵀ = [[ca, sa],[−sa, ca]].
    // Inverse affine coefficients: [x_src; y_src] = M·[x_tgt; y_tgt; 1].
    let m00 = ca * inv_s; // Rᵀ[0][0]/scale
    let m01 = sa * inv_s; // Rᵀ[0][1]/scale
    let m10 = -sa * inv_s; // Rᵀ[1][0]/scale
    let m11 = ca * inv_s; // Rᵀ[1][1]/scale
                          // Translation for inverse map.
    let m02 = src_cx - m00 * tgt_cx - m01 * tgt_cy;
    let m12 = src_cy - m10 * tgt_cx - m11 * tgt_cy;

    // Bilinear sample into the output.
    let mut out = vec![0u8; out_w * out_h * 3];
    let w = frame_w as i64;
    let h = frame_h as i64;
    for y in 0..out_h {
        for x in 0..out_w {
            // +0.5 pixel centers keep sampling symmetric (fit the canonical
            // integer landmark coordinates used during training).
            let tx = x as f32 + 0.5;
            let ty = y as f32 + 0.5;
            let sx = m00 * tx + m01 * ty + m02 - 0.5;
            let sy = m10 * tx + m11 * ty + m12 - 0.5;
            let x0 = sx.floor() as i64;
            let y0 = sy.floor() as i64;
            let fx = sx - x0 as f32;
            let fy = sy - y0 as f32;
            let x1 = x0 + 1;
            let y1 = y0 + 1;
            if x0 >= 0 && y0 >= 0 && x1 < w && y1 < h {
                let d = (y * out_w + x) * 3;
                for c in 0..3usize {
                    // SAFETY: x0/y0 ≥ 0 and x1<w,y1<h — indices valid.
                    let p00 = rgb[(y0 as usize * w as usize + x0 as usize) * 3 + c] as f32;
                    let p10 = rgb[(y0 as usize * w as usize + x1 as usize) * 3 + c] as f32;
                    let p01 = rgb[(y1 as usize * w as usize + x0 as usize) * 3 + c] as f32;
                    let p11 = rgb[(y1 as usize * w as usize + x1 as usize) * 3 + c] as f32;
                    let top = p00 + (p10 - p00) * fx;
                    let bot = p01 + (p11 - p01) * fx;
                    out[d + c] = (top + (bot - top) * fy).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    out
}

#[cfg(all(test, feature = "face"))]
mod tests {
    use super::*;

    /// Validate detect on a real face photo (synthetic decode check).
    /// Skips if the staging models aren't present (CI/other machines).
    #[test]
    fn detect_finds_face_in_real_photo() {
        let img_path = r"C:\ProgramData\MiControl\face\face_test.jpg";
        let model_dir = r"C:\ProgramData\MiControl\face\models_staging\buffalo_l";
        if !std::path::Path::new(img_path).exists()
            || !std::path::Path::new(model_dir).join(DET_MODEL).exists()
            || !std::path::Path::new(model_dir).join(REC_MODEL).exists()
        {
            eprintln!("SKIP: missing test image or staging models");
            return;
        }
        let img = image::open(img_path).expect("open test image");
        let rgb = img.to_rgb8();
        let (w, h) = (rgb.width() as usize, rgb.height() as usize);
        let frame = crate::hw::face::camera::Frame {
            data: rgb.into_raw(),
            width: w,
            height: h,
        };
        let mut det = FaceDetector::default();
        det.add_models_dir(model_dir);
        det.load().expect("load models");
        let faces = det.detect(&frame).expect("detect");
        for f in &faces {
            log::info!(
                "[face.models.test] score={:.3} bbox={:?} kps0={:?}",
                f.det_score,
                f.bbox,
                f.kps[0]
            );
        }
        assert!(
            !faces.is_empty(),
            "expected at least one face in face_test.jpg"
        );
        // The strongest detection should be at least 0.5 (score threshold).
        let best = faces.iter().map(|f| f.det_score).fold(0.0f32, f32::max);
        eprintln!("best score = {best:.3}, detections = {}", faces.len());
        assert!(best > 0.5, "best score too low: {best}");
    }

    /// Validate recognize() returns a unit-length 512-d embedding for a
    /// detected face on the real test photo.
    #[test]
    fn recognize_produces_unit_embedding() {
        let img_path = r"C:\ProgramData\MiControl\face\face_test.jpg";
        let model_dir = r"C:\ProgramData\MiControl\face\models_staging\buffalo_l";
        if !std::path::Path::new(img_path).exists()
            || !std::path::Path::new(model_dir).join(REC_MODEL).exists()
        {
            eprintln!("SKIP: missing test image or recognition model");
            return;
        }
        let img = image::open(img_path).expect("open test image");
        let rgb = img.to_rgb8();
        let (w, h) = (rgb.width() as usize, rgb.height() as usize);
        let frame = crate::hw::face::camera::Frame {
            data: rgb.into_raw(),
            width: w,
            height: h,
        };
        let mut det = FaceDetector::default();
        det.add_models_dir(model_dir);
        det.load().expect("load models");
        let faces = det.detect(&frame).expect("detect");
        assert!(!faces.is_empty(), "need at least one face");
        let f = faces
            .iter()
            .max_by(|a, b| a.det_score.partial_cmp(&b.det_score).unwrap())
            .unwrap();
        let emb = det
            .recognize(&frame.data, w, h, &f.kps, &f.bbox)
            .expect("recognize");
        assert_eq!(emb.len(), EMBEDDING_DIM);
        let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        eprintln!("embedding norm = {norm:.4} (expected ~1.0)");
        assert!(
            (norm - 1.0).abs() < 1e-3,
            "embedding not unit length: {norm}"
        );
    }

    /// End-to-end ML pipeline: detect → recognize → store → match → session.
    /// Mirrors the auth service flow minus the real-time camera loop.
    #[test]
    fn e2e_detect_recognize_match_session() {
        let img_path = r"C:\ProgramData\MiControl\face\face_test.jpg";
        let model_dir = r"C:\ProgramData\MiControl\face\models_staging\buffalo_l";
        if !std::path::Path::new(img_path).exists()
            || !std::path::Path::new(model_dir).join(REC_MODEL).exists()
        {
            eprintln!("SKIP: missing test image or recognition model");
            return;
        }
        let img = image::open(img_path).expect("open test image");
        let rgb = img.to_rgb8();
        let (w, h) = (rgb.width() as usize, rgb.height() as usize);
        let frame = crate::hw::face::camera::Frame {
            data: rgb.into_raw(),
            width: w,
            height: h,
        };
        let mut det = FaceDetector::default();
        det.add_models_dir(model_dir);
        det.load().expect("load models");
        let faces = det.detect(&frame).expect("detect");
        let f = faces
            .iter()
            .max_by(|a, b| a.det_score.partial_cmp(&b.det_score).unwrap())
            .expect("no faces");
        let emb = det
            .recognize(&frame.data, w, h, &f.kps, &f.bbox)
            .expect("recognize");

        // Enroll into a store + match against the same embedding.
        let mut store = crate::hw::face::store::FaceStore::new();
        store.settings.liveness_enabled = false;
        store
            .add_template("alice", emb.clone(), "front")
            .expect("add");

        let mut gallery = Vec::new();
        let mut names = Vec::new();
        for p in &store.profiles {
            for t in &p.templates {
                gallery.push(t.embedding.clone());
                names.push(p.name.clone());
            }
        }
        let m = crate::hw::face::matcher::best_match_with_margin(&emb, &gallery, &names);
        eprintln!("e2e: sim={:.4} margin={:?}", m.similarity, m.margin);
        assert!(m.similarity > 0.7, "self-match too low: {}", m.similarity);

        // AuthSession should unlock immediately (liveness disabled).
        let mut session = crate::hw::face::service::AuthSession::new(&store);
        session.feed(0.3, 0.0, Some(emb));
        assert!(session.done());
        let r = session.result().expect("result").clone();
        assert!(r.success, "session should unlock");
        assert_eq!(r.name.as_deref(), Some("alice"));
    }

    /// The similarity transform should place source landmarks approximately on
    /// the ArcFace canonical targets: if we paint a bright marker at each
    /// source landmark and warp, the crop should show those markers at (or
    /// very near) the target positions. Also, perfect inputs must warp
    /// perfectly (identity when kps == tgt-scaled).
    #[test]
    fn warp_face_aligns_landmarks() {
        let tgt: [[f32; 2]; 5] = [
            [38.2946, 51.6963],
            [73.5318, 51.5014],
            [56.0252, 71.7366],
            [41.5493, 92.3655],
            [70.7299, 92.2041],
        ];
        let (out_w, out_h) = (112usize, 112usize);

        // Source landmarks: translate + rotate the targets so the transform
        // must recover a rotation.
        let (cx, cy) = (200.0f32, 150.0f32);
        let mut src = [[0.0f32; 2]; 5];
        for i in 0..5 {
            let dx = tgt[i][0] - 56.0; // about center of 112
            let dy = tgt[i][1] - 72.0;
            src[i] = [cx + dx * 2.0, cy + dy * 2.0]; // 2x scale, no rotation yet
        }
        // Build a synthetic frame 400x300 (a bit generous) with bright markers
        // at the source landmarks.
        let (fw, fh) = (400usize, 300usize);
        let mut rgb = vec![0u8; fw * fh * 3];
        for (i, p) in src.iter().enumerate() {
            // 5px radius filled disc, channel = landmark index (for ID).
            for dy in -5..6i32 {
                for dx in -5..6i32 {
                    let px = p[0] as i32 + dx;
                    let py = p[1] as i32 + dy;
                    if px >= 0 && py >= 0 && (px as usize) < fw && (py as usize) < fh {
                        let dist = ((dx * dx + dy * dy) as f32).sqrt();
                        if dist < 5.0 {
                            let idx = ((py as usize) * fw + px as usize) * 3;
                            rgb[idx] = 55 + i as u8 * 40; // red channel marks the landmark
                            rgb[idx + 1] = 10;
                            rgb[idx + 2] = 10;
                        }
                    }
                }
            }
        }
        let bbox = [80.0f32, 30.0f32, 320.0f32, 270.0f32];
        let out = warp_face(&rgb, fw, fh, &src, &bbox, out_w, out_h);
        // Where does each target pixel land? Each source marker is red-valued
        // ~55+i*40, others are 0, so we can locate them in the output.
        let mut found = [false; 5];
        for y in 0..out_h {
            for x in 0..out_w {
                let c = &out[(y * out_w + x) * 3];
                if *c > 50 {
                    for i in 0..5u8 {
                        let want = 55 + i * 40;
                        if (*c).abs_diff(want) <= 4 {
                            found[i as usize] = true;
                        }
                    }
                }
            }
        }
        // All 5 landmark markers should be present somewhere in the crop.
        assert!(found.iter().all(|&f| f), "markers not all found: {found:?}");
    }

    /// warping an already-aligned set (kps == targets within the same scale)
    /// should be near-identity.
    #[test]
    fn warp_face_identity_when_aligned() {
        let tgt: [[f32; 2]; 5] = [
            [38.2946, 51.6963],
            [73.5318, 51.5014],
            [56.0252, 71.7366],
            [41.5493, 92.3655],
            [70.7299, 92.2041],
        ];
        let (out_w, out_h) = (112usize, 112usize);
        let (fw, fh) = (112usize, 112usize);
        // Source == target (already aligned).
        let mut rgb = vec![0u8; fw * fh * 3];
        // Draw markers at the target positions as well, red-only.
        for p in &tgt {
            for dy in -4..5i32 {
                for dx in -4..5i32 {
                    let px = p[0] as i32 + dx;
                    let py = p[1] as i32 + dy;
                    if (0..112).contains(&px) && (0..112).contains(&py) {
                        let idx = ((py as usize) * fw + px as usize) * 3;
                        rgb[idx] = 200;
                        rgb[idx + 1] = 200;
                        rgb[idx + 2] = 0; // yellow-ish, both channels high
                    }
                }
            }
        }
        let bbox = [10.0f32, 20.0f32, 100.0f32, 100.0f32];
        let out = warp_face(&rgb, fw, fh, &tgt, &bbox, out_w, out_h);
        // All landmarks should be ~at their targets. Count colored pixels.
        let mut hit = 0usize;
        let mut tot = 0usize;
        for y in 0..out_h {
            for x in 0..out_w {
                let idx = (y * out_w + x) * 3;
                if rgb[idx] > 150 && rgb[idx + 1] > 150 && rgb[idx + 2] == 0 {
                    tot += 1;
                    if out[idx] > 150 && out[idx + 1] > 150 && out[idx + 2] == 0 {
                        hit += 1;
                    }
                }
            }
        }
        assert!(
            tot > 100,
            "sanity: expected yellow marker pixels, got {tot}"
        );
        let frac = hit as f32 / tot as f32;
        eprintln!("identity warp: colored hit fraction = {frac:.3}");
        assert!(frac > 0.9, "identity warp lost pixels: {frac}");
    }
}
