//! Local speech transcription (MIOT-08) via the sherpa-onnx CLI.
//!
//! sherpa-onnx is a lightweight, standalone on-device ASR toolkit
//! (k2-fsa/sherpa-onnx). Speech recognition here never leaves the machine:
//! we orchestrate the `sherpa-onnx` CLI binary over a local `.wav` file and
//! return the transcribed text, with no heavy Rust ASR dependency (zero build
//! risk for this crate).
//!
//! Orchestration responsibilities:
//! - detect the `sherpa-onnx` binary on PATH (with a winget hint);
//! - ensure an int8 quantized model (e.g. paraformer) + `tokens.txt` live in
//!   the app-data model dir, downloading them on first use (best-effort, via
//!   reqwest streaming so the UI can show progress/errors);
//! - build the CLI argv (pure, unit-tested) and run the binary, capturing
//!   stdout text.
//!
//! When the binary or the model is missing, commands return a *friendly*
//! error (never a panic) describing exactly what to install/download, so the
//! frontend can render guidance instead of a crash.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

/// Default model: FunASR paraformer int8 (small, good zh/en/noise handling).
/// Public URLs kept in one place so the download helper and the UI hint agree.
pub const MODEL_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-paraformer-zh-small-2024-03-09.tar.bz2";
pub const MODEL_SUBDIR: &str = "sherpa-onnx-paraformer-zh-small-2024-03-09";
pub const SHERPA_EXE: &str = "sherpa-onnx";

/// Which ASR graph to build argv for. Paraformer is the default int8 model;
/// `Whisper` is offered for callers that bring their own whisper assets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    Paraformer,
    Whisper,
}

/// sherpa-onnx is not on winget; we ship a pinned Windows x64 static build
/// from the official GitHub releases. Used by `install_binary()` and the
/// status hint.
///
/// Asset layout (v1.13.6, verified): the tarball contains a `bin/` dir with
/// `sherpa-onnx.exe` (+ DLLs on shared builds; the static MT build is
/// fully standalone).
pub const BIN_VERSION: &str = "v1.13.6";
pub const BIN_ASSET: &str = "sherpa-onnx-v1.13.6-win-x64-static-MT-Release.tar.bz2";
pub const BIN_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.6/sherpa-onnx-v1.13.6-win-x64-static-MT-Release.tar.bz2";

/// App-data locations for the downloaded assets.
pub fn model_dir() -> Result<PathBuf, String> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .ok_or_else(|| "cannot locate app data directory".to_string())?;
    Ok(base
        .join("MiControl")
        .join("sherpa-onnx")
        .join(MODEL_SUBDIR))
}

/// Detect the `sherpa-onnx` CLI on PATH **or** in MiControl's app-data bin
/// dir (where `install_binary()` puts it). Returns `Ok(None)` when absent.
pub fn find_binary() -> Result<Option<PathBuf>, String> {
    let exe = if cfg!(windows) {
        "sherpa-onnx.exe"
    } else {
        "sherpa-onnx"
    };
    let out = Command::new(exe)
        .arg("--version")
        .output()
        .map_err(|e| format!("failed to probe sherpa-onnx: {e}"))?;
    if out.status.success() {
        return Ok(Some(PathBuf::from(exe)));
    }

    // Fallback: our managed install dir.
    if let Ok(dir) = binary_dir() {
        let candidate = dir.join(exe);
        let out2 = Command::new(&candidate).arg("--version").output();
        if let Ok(o) = out2 {
            if o.status.success() {
                return Ok(Some(candidate));
            }
        }
    }
    Ok(None)
}

/// Build the argv for a local-file transcription. Pure and unit-testable.
pub fn build_argv(kind: ModelKind, model_dir: &Path, wav: &Path, num_threads: u32) -> Vec<String> {
    let mut args = Vec::new();
    match kind {
        ModelKind::Paraformer => {
            args.push("--paraformer".into());
            args.push(model_dir.join("model.int8.onnx").display().to_string());
        }
        ModelKind::Whisper => {
            args.push("--whisper-encoder".into());
            args.push(model_dir.join("encoder.int8.onnx").display().to_string());
            args.push("--whisper-decoder".into());
            args.push(model_dir.join("decoder.int8.onnx").display().to_string());
        }
    }
    args.push("--tokens".into());
    args.push(model_dir.join("tokens.txt").display().to_string());
    args.push("--num-threads".into());
    args.push(num_threads.to_string());
    args.push("--wav".into());
    args.push(wav.display().to_string());
    args
}

/// Check whether the model assets are present on disk.
pub fn ensure_model_assets() -> Result<PathBuf, String> {
    let dir = model_dir()?;
    for required in ["model.int8.onnx", "tokens.txt"] {
        if !dir.join(required).exists() {
            return Err(format!(
                "missing model asset '{required}' — run 'download_model' first (see {} )",
                MODEL_URL
            ));
        }
    }
    Ok(dir)
}

/// Download the int8 paraformer model + tokens into the app-data model dir.
/// Best-effort: streams the tarball to a temp file and extracts the two files
/// we need. Returns the model dir when done.
pub fn download_model() -> Result<PathBuf, String> {
    let dir = model_dir()?;
    if dir.join("model.int8.onnx").exists() && dir.join("tokens.txt").exists() {
        return Ok(dir);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("create model dir: {e}"))?;

    // Stream-download with reqwest (already a dependency, rustls TLS).
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("build runtime: {e}"))?;
    rt.block_on(async move {
        let client = reqwest::Client::builder()
            .user_agent("MiControl/0.1 (sherpa-onnx model fetch)")
            .build()
            .map_err(|e| format!("build http client: {e}"))?;
        let resp = client
            .get(MODEL_URL)
            .send()
            .await
            .map_err(|e| format!("download failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("download returned HTTP {}", resp.status()));
        }
        let bytes = resp.bytes().await.map_err(|e| format!("read body: {e}"))?;

        // The tarball unpacks to a folder named `sherpa-onnx-paraformer-zh-small-2024-03-09`
        // containing model.int8.onnx + tokens.txt (+ others). Keep only those
        // two to avoid shipping unused assets.
        let tmp = dir.join("model.tar.bz2");
        std::fs::write(&tmp, &bytes).map_err(|e| format!("write temp tarball: {e}"))?;

        // Extract with `tar` (bundled with Windows 10+; also on macOS/Linux).
        let out = Command::new("tar")
            .current_dir(&dir)
            .args(["-xjf", tmp.to_str().unwrap_or("model.tar.bz2")])
            .output()
            .map_err(|e| format!("extract failed: {e}"))?;
        let _ = std::fs::remove_file(&tmp);
        if !out.status.success() {
            return Err(format!(
                "tar extract failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }

        // Move the two needed files up to the model dir root (trim the folder).
        let nested = dir.join(MODEL_SUBDIR);
        for f in ["model.int8.onnx", "tokens.txt"] {
            let src = nested.join(f);
            if src.exists() {
                std::fs::rename(&src, dir.join(f)).map_err(|e| format!("move {f}: {e}"))?;
            }
        }
        let _ = std::fs::remove_dir_all(&nested);

        if !dir.join("model.int8.onnx").exists() || !dir.join("tokens.txt").exists() {
            return Err(
                "model downloaded but expected assets missing — check the tarball layout".into(),
            );
        }
        Ok(dir)
    })
}

/// Install the sherpa-onnx CLI into MiControl's app-data bin dir.
///
/// sherpa-onnx is not published on winget, so we download the pinned static
/// Windows x64 release from GitHub, extract it, and copy the `.exe` into
/// `%APPDATA%\MiControl\sherpa-onnx\bin`. `find_binary()` is then updated to
/// also look there (see below).
pub fn install_binary() -> Result<PathBuf, String> {
    let bin_dir = binary_dir()?;
    let exe = bin_dir.join(exe_name());
    if exe.exists() {
        return Ok(exe);
    }
    std::fs::create_dir_all(&bin_dir).map_err(|e| format!("create bin dir: {e}"))?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("build runtime: {e}"))?;
    rt.block_on(async move {
        let client = reqwest::Client::builder()
            .user_agent("MiControl/0.2 (sherpa-onnx install)")
            .build()
            .map_err(|e| format!("build http client: {e}"))?;
        let resp = client
            .get(BIN_URL)
            .send()
            .await
            .map_err(|e| format!("download failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("download returned HTTP {}", resp.status()));
        }
        let bytes = resp.bytes().await.map_err(|e| format!("read body: {e}"))?;

        let tmp = bin_dir.join("sherpa-onnx.tar.bz2");
        std::fs::write(&tmp, &bytes).map_err(|e| format!("write tarball: {e}"))?;

        let extract_dir = bin_dir.join("extract");
        std::fs::create_dir_all(&extract_dir).map_err(|e| format!("create extract dir: {e}"))?;
        let out = Command::new("tar")
            .current_dir(&extract_dir)
            .args(["-xjf", tmp.to_str().unwrap_or("sherpa-onnx.tar.bz2")])
            .output()
            .map_err(|e| format!("extract failed: {e}"))?;
        let _ = std::fs::remove_file(&tmp);
        if !out.status.success() {
            return Err(format!(
                "tar extract failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }

        // Locate the exe inside the extracted tree (bin/ or root).
        let found = find_exe_in_dir(&extract_dir);
        let Some(src) = found else {
            let _ = std::fs::remove_dir_all(&extract_dir);
            return Err("sherpa-onnx.exe not found in the downloaded archive layout".into());
        };
        std::fs::rename(&src, &exe).map_err(|e| format!("move sherpa-onnx.exe: {e}"))?;
        let _ = std::fs::remove_dir_all(&extract_dir);
        Ok(exe)
    })
}

/// `%APPDATA%\MiControl\sherpa-onnx\bin`
fn binary_dir() -> Result<PathBuf, String> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .ok_or_else(|| "cannot locate app data directory".to_string())?;
    Ok(base.join("MiControl").join("sherpa-onnx").join("bin"))
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "sherpa-onnx.exe"
    } else {
        "sherpa-onnx"
    }
}

fn find_exe_in_dir(dir: &Path) -> Option<PathBuf> {
    let exe = exe_name();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&d) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.file_name().and_then(|s| s.to_str()) == Some(exe) {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Run sherpa-onnx on `wav_path` and return the transcribed text.
pub fn transcribe(wav_path: &str) -> Result<String, String> {
    let bin = find_binary()?.ok_or_else(|| {
        "sherpa-onnx is not installed — click Install in the UI or download from GitHub".to_string()
    })?;
    let model = ensure_model_assets()?;
    let args = build_argv(ModelKind::Paraformer, &model, Path::new(wav_path), 2);

    let out = Command::new(&bin)
        .args(&args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("failed to run sherpa-onnx: {e}"))?;

    // sherpa-onnx prints the recognized text on stdout; keep the first
    // non-empty line (trimmed) so the UI shows clean results.
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let text = stdout
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();

    if !out.status.success() {
        let detail = if !stderr.trim().is_empty() {
            stderr.trim()
        } else {
            stdout.trim()
        };
        return Err(format!(
            "sherpa-onnx exited with {}: {}",
            out.status,
            detail.chars().take(400).collect::<String>()
        ));
    }
    if text.is_empty() {
        return Err("sherpa-onnx produced no transcript (empty audio?)".into());
    }
    Ok(text.to_string())
}

/// Status snapshot for the UI (installed? model present?).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionStatus {
    pub binary_installed: bool,
    pub binary_path: Option<String>,
    pub model_ready: bool,
    pub model_dir: Option<String>,
    pub install_hint: String,
    pub missing: Vec<String>,
}

pub fn status() -> TranscriptionStatus {
    let bin = find_binary().ok().flatten();
    let model = ensure_model_assets().ok();
    let mut missing = Vec::new();
    if bin.is_none() {
        missing.push("sherpa-onnx".into());
    }
    if model.is_none() {
        missing.push("model".into());
    }
    TranscriptionStatus {
        binary_installed: bin.is_some(),
        binary_path: bin.as_ref().map(|b| b.display().to_string()),
        model_ready: model.is_some(),
        model_dir: model.as_ref().map(|d| d.display().to_string()),
        install_hint: "sherpa-onnx is not on winget — MiControl can download the official Windows build for you (Install button)".into(),
        missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_paraformer_matches_expected() {
        let model = Path::new("C:/opt/model");
        let wav = Path::new("C:/tmp/a.wav");
        let args = build_argv(ModelKind::Paraformer, model, wav, 2);
        let model_onnx = model.join("model.int8.onnx").display().to_string();
        let tokens = model.join("tokens.txt").display().to_string();
        assert_eq!(
            args,
            vec![
                "--paraformer",
                model_onnx.as_str(),
                "--tokens",
                tokens.as_str(),
                "--num-threads",
                "2",
                "--wav",
                "C:/tmp/a.wav",
            ]
        );
    }

    #[test]
    fn argv_whisper_uses_encoder_decoder() {
        let model = Path::new("/m/model");
        let wav = Path::new("/m/a.wav");
        let args = build_argv(ModelKind::Whisper, model, wav, 1);
        assert!(args.contains(&"--whisper-encoder".to_string()));
        assert!(args.contains(&"--whisper-decoder".to_string()));
        assert!(args.contains(&model.join("encoder.int8.onnx").display().to_string()));
        assert!(args.contains(&model.join("decoder.int8.onnx").display().to_string()));
        assert!(args.contains(&"/m/a.wav".to_string()));
    }

    #[test]
    fn argv_always_has_wav_and_tokens() {
        let args = build_argv(ModelKind::Paraformer, Path::new("m"), Path::new("w"), 4);
        let flag_ix = args
            .iter()
            .position(|a| a == "--wav")
            .expect("--wav present");
        assert_eq!(args[flag_ix + 1], "w");
        assert!(args.contains(&"--tokens".to_string()));
    }

    #[test]
    fn status_never_panics_without_tooling() {
        // In CI/locally sherpa-onnx may be absent — status must still be a
        // well-formed object, with `missing` populated.
        let s = status();
        assert!(!s.binary_installed || s.binary_path.is_some());
        assert!(!s.model_ready || s.model_dir.is_some());
        assert!(!s.install_hint.is_empty());
    }
}
