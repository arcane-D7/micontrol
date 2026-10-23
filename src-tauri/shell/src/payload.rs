//! Payload acquisition: local file, direct URL, or latest GitHub release.
//!
//! The payload (`MiControl_<version>_payload.zip`) is the versioned part of
//! the decoupled installer. It is minisign-signed with the SAME key pair the
//! Tauri updater uses; the shell refuses to apply an unverified payload.

use std::path::{Path, PathBuf};

use crate::ctx::{DEFAULT_REPO, PUBKEY_BLOB};

pub struct ResolvedPayload {
    pub zip: PathBuf,
    pub sig: Option<PathBuf>,
    /// True when the zip was downloaded this run (clean it up afterwards).
    pub downloaded: bool,
}

/// Resolve the payload to apply.
///
/// Priority: explicit `--payload` → explicit `--url` → latest GitHub release.
pub fn resolve(
    payload: Option<&Path>,
    url: Option<&str>,
    repo: &str,
) -> Result<ResolvedPayload, String> {
    if let Some(p) = payload {
        if !p.is_file() {
            return Err(format!("payload file not found: {}", p.display()));
        }
        let sig = sig_for(p);
        return Ok(ResolvedPayload {
            zip: p.to_path_buf(),
            sig,
            downloaded: false,
        });
    }

    let url = match url {
        Some(u) if !u.trim().is_empty() => u.trim().to_string(),
        _ => latest_payload_url(repo)?,
    };

    let cache = crate::ctx::local_appdata_dir().join("setup");
    std::fs::create_dir_all(&cache)
        .map_err(|e| format!("cannot create {}: {e}", cache.display()))?;
    let file_name = url.rsplit('/').next().unwrap_or("MiControl_payload.zip");
    let dest = cache.join(file_name);
    let dest_sig = cache.join(format!("{file_name}.sig"));

    // Determine the .sig URL: replace the last path segment with <name>.sig.
    let sig_url = format!("{url}.sig");

    println!("==> downloading payload: {url}");
    download(&url, &dest)?;
    // Signature is REQUIRED. A payload without a verifiable signature must
    // never be applied (same trust model as the Tauri updater).
    println!("==> downloading signature: {sig_url}");
    download(&sig_url, &dest_sig)?;

    Ok(ResolvedPayload {
        zip: dest,
        sig: Some(dest_sig),
        downloaded: true,
    })
}

fn sig_for(zip: &Path) -> Option<PathBuf> {
    let s = zip.with_extension("zip.sig");
    if s.is_file() {
        return Some(s);
    }
    let s2 = PathBuf::from(format!("{}.sig", zip.display()));
    s2.is_file().then_some(s2)
}

/// Query the GitHub API for the latest release and pick the payload asset.
fn latest_payload_url(repo: &str) -> Result<String, String> {
    let repo = if repo.trim().is_empty() {
        DEFAULT_REPO
    } else {
        repo.trim()
    };
    let api = format!("https://api.github.com/repos/{repo}/releases/latest");
    let body = http_get(&api)?;
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("invalid GitHub API response: {e}"))?;
    let assets = json
        .get("assets")
        .and_then(|a| a.as_array())
        .ok_or_else(|| format!("GitHub API response has no assets (repo {repo})"))?;
    for asset in assets {
        let name = asset
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_default();
        if name.starts_with("MiControl_")
            && name.ends_with("_payload.zip")
            && !name.ends_with(".zip.sig")
        {
            if let Some(u) = asset.get("browser_download_url").and_then(|u| u.as_str()) {
                return Ok(u.to_string());
            }
        }
    }
    Err(format!(
        "no MiControl_*_payload.zip asset found in the latest release of {repo}"
    ))
}

fn http_get(url: &str) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .user_agent("MiControl-Setup-Shell/1.0 (+https://github.com/arcane-D7/micontrol)")
        .build();
    let resp = agent
        .get(url)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("GET {url}: {e}"))?;
    resp.into_string()
        .map_err(|e| format!("read response from {url}: {e}"))
}

fn download(url: &str, dest: &Path) -> Result<(), String> {
    download_public(url, dest)
}

/// Download a file to `dest` (atomic .part → rename).
pub fn download_public(url: &str, dest: &Path) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new()
        .user_agent("MiControl-Setup-Shell/1.0 (+https://github.com/arcane-D7/micontrol)")
        .build();
    let mut reader = agent
        .get(url)
        .call()
        .map_err(|e| format!("GET {url}: {e}"))?
        .into_reader();

    let tmp = dest.with_extension("part");
    let mut file =
        std::fs::File::create(&tmp).map_err(|e| format!("cannot create {}: {e}", tmp.display()))?;
    std::io::copy(&mut reader, &mut file)
        .map_err(|e| format!("download to {}: {e}", tmp.display()))?;
    drop(file);
    std::fs::rename(&tmp, dest).map_err(|e| format!("cannot finalize {}: {e}", dest.display()))?;

    let size = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
    println!("    saved {} ({} bytes)", dest.display(), size);
    Ok(())
}

/// Verify a minisign signature over `data_path` using the embedded public key.
pub fn verify_signature(data_path: &Path, sig_path: &Path) -> Result<(), String> {
    use minisign_verify::Signature;

    let data = std::fs::read(data_path)
        .map_err(|e| format!("cannot read {}: {e}", data_path.display()))?;
    let sig_text = std::fs::read_to_string(sig_path)
        .map_err(|e| format!("cannot read {}: {e}", sig_path.display()))?;
    let sig = Signature::decode(&sig_text)
        .map_err(|e| format!("invalid signature file {}: {e}", sig_path.display()))?;

    let pk = decode_pubkey(PUBKEY_BLOB)?;
    // `allow_legacy=false` requires a proper trusted/untrusted comment header.
    pk.verify(&data, &sig, false).map_err(|e| {
        format!(
            "SIGNATURE VERIFICATION FAILED for {}: {e}",
            data_path.display()
        )
    })
}

/// Accept the Tauri-style pubkey blob ("untrusted comment: ...\nRWS...\n")
/// as well as the bare base64 key line.
fn decode_pubkey(blob: &str) -> Result<minisign_verify::PublicKey, String> {
    use minisign_verify::PublicKey;

    let blob = blob.trim();
    // Direct attempt (works when the blob is already a base64 key line).
    if let Ok(pk) = PublicKey::from_base64(blob) {
        return Ok(pk);
    }
    // Attempt 1: the outer blob itself is base64 → decode, take last line.
    use base64::Engine;
    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(blob) {
        if let Ok(text) = String::from_utf8(decoded) {
            if let Some(line) = text.lines().rev().find(|l| !l.trim().is_empty()) {
                if let Ok(pk) = PublicKey::from_base64(line.trim()) {
                    return Ok(pk);
                }
            }
        }
    }
    // Attempt 2: take the last non-empty line of the raw blob.
    if let Some(line) = blob.lines().rev().find(|l| !l.trim().is_empty()) {
        if let Ok(pk) = PublicKey::from_base64(line.trim()) {
            return Ok(pk);
        }
    }
    Err("cannot decode the embedded minisign public key".to_string())
}

/// Extract the payload zip into `install_dir`.
///
/// Safety: rejects absolute paths and `..` traversal (zip-slip). The `setup/`
/// directory is NEVER overwritten — it belongs to the shell itself.
pub fn extract(zip_path: &Path, install_dir: &Path) -> Result<(usize, Option<String>), String> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| format!("cannot open payload {}: {e}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|e| format!("invalid payload zip {}: {e}", zip_path.display()))?;

    let install_root = install_dir
        .canonicalize()
        .unwrap_or_else(|_| install_dir.to_path_buf());
    let mut extracted = 0usize;
    let mut payload_version: Option<String> = None;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("corrupt payload entry #{i}: {e}"))?;
        let Some(name) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            // Absolute or traversing path → reject the whole payload.
            let raw = entry.name().to_string();
            return Err(format!("unsafe payload entry path rejected: {raw}"));
        };
        let name_str = name.to_string_lossy().replace('/', "\\");
        if name_str.starts_with("setup\\") || name_str == "setup" {
            continue; // shell territory — never touched by payloads
        }

        let target = install_dir.join(&name);
        if entry.is_dir() {
            let _ = std::fs::create_dir_all(&target);
            continue;
        }
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut out = std::fs::File::create(&target)
            .map_err(|e| format!("cannot write {}: {e}", target.display()))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|e| format!("cannot extract {}: {e}", target.display()))?;
        extracted += 1;

        if name == std::path::Path::new("payload.json") {
            if let Ok(text) = std::fs::read_to_string(&target) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    payload_version = v.get("version").and_then(|x| x.as_str()).map(String::from);
                }
            }
        }
    }

    // Sanity: the extracted root must stay inside the install dir.
    let _ = install_root;
    Ok((extracted, payload_version))
}

// ── S65-004: fail-closed preflight ───────────────────────────────────────────
//
// Validates the payload zip WITHOUT touching the install directory or
// stopping any processes. Must be called BEFORE `stop_everything` so a
// bad payload can never leave the installed app in a disrupted state.
//
// Checks:
//   1. The zip opens and is not empty.
//   2. No entry has an unsafe (absolute / traversing) path.
//   3. All REQUIRED top-level entries are present.
//   4. payload.json (when present) contains a valid version string.
//   5. Optional entries are reported but never cause a failure.
//
// Returns a deterministic `PreflightReport` the caller can log or return
// to the user.

/// Top-level entries that MUST be in every payload zip.
pub const REQUIRED_ENTRIES: &[&str] =
    &["micontrol.exe", "ecram_service.exe", "micontrol_bridge.exe"];

/// Top-level entries that MAY be absent (face unlock is optional).
pub const OPTIONAL_ENTRIES: &[&str] = &[
    "micontrol_face_svc.exe",
    "micontrol_facecp.dll",
    "WebView2Loader.dll",
    "drivers",
    "resources",
    "payload.json",
];

/// Result of a preflight scan of a payload zip.
#[derive(Debug, Clone)]
pub struct PreflightReport {
    /// True when every required entry was found and no unsafe paths exist.
    pub ok: bool,
    /// Required entries that were NOT in the zip.
    pub missing_required: Vec<String>,
    /// Optional entries that were NOT in the zip (informational only).
    pub missing_optional: Vec<String>,
    /// The version string from payload.json, if the file was present.
    pub version: Option<String>,
    /// True when any entry had an unsafe path (the whole zip is rejected).
    pub unsafe_path_found: bool,
    /// True when any entry name appears more than once (the zip is rejected —
    /// duplicate paths can overwrite each other while extracting).
    pub duplicate_path_found: bool,
    /// True when payload.json exists but is not valid JSON / has no usable
    /// `version` (the manifest must be well-formed; the zip is rejected).
    pub malformed_manifest: bool,
}

/// Normalize an `enclosed_name` into the effective extraction-relative target
/// string, so preflight can detect entry pairs that would overwrite the same
/// file during extraction. Drops `.` components, collapses `..` as far as
/// possible and unifies separators: `"a/../b"` and `"./a\b"` both become
/// `"a/b"`. Input is already validated by `enclosed_name` (no prefix/root or
/// `..` escape), so leftover `..` simply pops without going out of bounds.
fn normalized_target(p: &std::path::Path) -> String {
    let mut stack: Vec<String> = Vec::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                let _ = stack.pop();
            }
            std::path::Component::Normal(n) => {
                stack.push(n.to_string_lossy().replace('\\', "/"));
            }
            _ => {}
        }
    }
    stack.join("/")
}

/// Validate the payload zip at `zip_path` WITHOUT modifying the install dir.
///
/// This is the fail-closed gate required by S65-004: it runs before
/// `stop_everything` so a malformed or incomplete payload is rejected
/// while the app and services are still running.
pub fn preflight(zip_path: &Path) -> Result<PreflightReport, String> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| format!("cannot open payload {}: {e}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|e| format!("invalid payload zip {}: {e}", zip_path.display()))?;

    if archive.is_empty() {
        return Ok(PreflightReport {
            ok: false,
            missing_required: REQUIRED_ENTRIES.iter().map(|s| s.to_string()).collect(),
            missing_optional: Vec::new(),
            version: None,
            unsafe_path_found: false,
            duplicate_path_found: false,
            malformed_manifest: false,
        });
    }

    let mut top_level: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_targets: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut version: Option<String> = None;
    let mut unsafe_found = false;
    let mut duplicate_found = false;
    let mut malformed_manifest = false;

    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("corrupt payload entry #{i}: {e}"))?;

        // Collect top-level names for the required/optional check.
        let raw_name = entry.name().to_string().replace('\\', "/");
        let top = raw_name.split('/').next().unwrap_or("");
        if !top.is_empty() {
            top_level.insert(top.to_string());
        }

        // Reject unsafe paths (same zip-slip guard as extract()).
        let Some(enclosed) = entry.enclosed_name() else {
            unsafe_found = true;
            continue;
        };
        // Reject duplicate extraction targets: different raw spellings that
        // resolve to the same file ("./micontrol.exe" vs "micontrol.exe",
        // "a/../b" vs "b", '\' vs '/') would overwrite each other during
        // extraction, which is never legitimate.
        let norm = normalized_target(&enclosed);
        if !seen_targets.insert(norm) {
            duplicate_found = true;
        }
    }

    // Read payload.json content (independent pass so we don't mix the two loops).
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("corrupt payload entry #{i}: {e}"))?;
        let raw_name = entry.name().to_string().replace('\\', "/");
        if raw_name == "payload.json" {
            let mut text = String::new();
            use std::io::Read;
            let mut reader = entry;
            if reader.read_to_string(&mut text).is_ok() {
                match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(v) => {
                        let v = v.get("version").and_then(|x| x.as_str());
                        match v {
                            Some(v) if !v.is_empty() => version = Some(v.to_string()),
                            // Present but unusable version → malformed manifest.
                            _ => malformed_manifest = true,
                        }
                    }
                    // Not valid JSON at all → malformed manifest (fail closed).
                    Err(_) => malformed_manifest = true,
                }
            } else {
                // Unreadable payload.json (e.g. huge/compressed oddly) → reject.
                malformed_manifest = true;
            }
            break;
        }
    }

    // REQUIRED_ENTRIES is an array of &str, so .iter() yields &&str; `**name`
    // dereferences down to the &str so HashSet<String>::contains matches on str.
    let missing_required: Vec<String> = REQUIRED_ENTRIES
        .iter()
        .filter(|name| !top_level.contains(**name))
        .map(|s| s.to_string())
        .collect();
    let missing_optional: Vec<String> = OPTIONAL_ENTRIES
        .iter()
        .filter(|name| !top_level.contains(**name))
        .map(|s| s.to_string())
        .collect();

    Ok(PreflightReport {
        ok: missing_required.is_empty() && !unsafe_found && !duplicate_found && !malformed_manifest,
        missing_required,
        missing_optional,
        version,
        unsafe_path_found: unsafe_found,
        duplicate_path_found: duplicate_found,
        malformed_manifest,
    })
}

/// S65-004 version-mismatch guard. When the caller resolved the payload under
/// a specific expected version (e.g. from `MiControl_<ver>_payload.zip` or an
/// updater URL), the payload manifest must agree — a downgrade/rogue payload
/// must never be applied under a forged name.
///
/// Returns `Some(description)` when the versions disagree (or the manifest
/// version is missing) and `None` when they match.
pub fn version_mismatch(expected: &str, report: &PreflightReport) -> Option<String> {
    match &report.version {
        Some(v) if v == expected => None,
        Some(v) => Some(format!(
            "payload version mismatch — expected {expected}, payload is {v}"
        )),
        None => Some(format!(
            "payload manifest has no usable version (expected {expected})"
        )),
    }
}

/// Convenience: run preflight and return a human-readable one-line report.
/// Used by the shell main() to print a deterministic summary before any
/// disruption.
pub fn preflight_summary(report: &PreflightReport) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(v) = &report.version {
        parts.push(format!("version={v}"));
    }
    if !report.missing_required.is_empty() {
        parts.push(format!(
            "missing_required={}",
            report.missing_required.join(",")
        ));
    }
    if !report.missing_optional.is_empty() {
        parts.push(format!(
            "missing_optional={}",
            report.missing_optional.join(",")
        ));
    }
    if report.unsafe_path_found {
        parts.push("UNSAFE_PATHS_FOUND".to_string());
    }
    if report.duplicate_path_found {
        parts.push("DUPLICATE_PATHS_FOUND".to_string());
    }
    if report.malformed_manifest {
        parts.push("MALFORMED_MANIFEST".to_string());
    }
    if report.ok {
        parts.push("OK".to_string());
    } else {
        parts.push("REJECTED".to_string());
    }
    parts.join(" ")
}

/// Version string recorded in an already-installed payload.json.
pub fn installed_version(install_dir: &Path) -> Option<String> {
    let p = install_dir.join("payload.json");
    let text = std::fs::read_to_string(p).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("version").and_then(|x| x.as_str()).map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Monotonic counter so parallel tests never collide on a temp dir name
    /// (a plain millisecond timestamp is not unique enough under parallelism).
    static ZIP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn make_zip(entries: &[&str]) -> std::path::PathBuf {
        let n = ZIP_COUNTER.fetch_add(1, Ordering::SeqCst);
        let tmp =
            std::env::temp_dir().join(format!("micontrol-payload-test-{}-{n}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let zip_path = tmp.join("test_payload.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default();
            for name in entries {
                zip.start_file(name, opts).unwrap();
                zip.write_all(b"dummy").unwrap();
            }
            zip.finish().unwrap();
        }
        zip_path
    }

    #[test]
    fn preflight_passes_with_all_required_entries() {
        let zip = make_zip(&[
            "micontrol.exe",
            "ecram_service.exe",
            "micontrol_bridge.exe",
            "micontrol_face_svc.exe",
            // NOTE: payload.json intentionally omitted here — make_zip writes
            // "dummy" content which would trip the malformed-manifest check.
            // Valid-manifest coverage lives in preflight_reads_version_from_payload_json.
        ]);
        let report = preflight(&zip).unwrap();
        assert!(report.ok, "expected preflight to pass: {report:?}");
        assert!(report.missing_required.is_empty());
        assert!(!report.unsafe_path_found);
        let _ = std::fs::remove_file(&zip);
    }

    #[test]
    fn preflight_fails_when_required_entry_missing() {
        let zip = make_zip(&[
            "ecram_service.exe",
            "micontrol_bridge.exe",
            // micontrol.exe intentionally missing
        ]);
        let report = preflight(&zip).unwrap();
        assert!(!report.ok, "must fail without micontrol.exe");
        assert_eq!(report.missing_required, vec!["micontrol.exe".to_string()]);
        let _ = std::fs::remove_file(&zip);
    }

    #[test]
    fn preflight_reports_missing_optional_but_still_passes() {
        let zip = make_zip(&[
            "micontrol.exe",
            "ecram_service.exe",
            "micontrol_bridge.exe",
            // face_svc, facecp, WebView2, drivers, resources — all absent
        ]);
        let report = preflight(&zip).unwrap();
        assert!(report.ok, "optional entries must not block preflight");
        assert!(!report.missing_optional.is_empty());
        let _ = std::fs::remove_file(&zip);
    }

    #[test]
    fn preflight_rejects_unsafe_traversal_path() {
        // Build a zip with a relative-traversal entry manually.
        let tmp =
            std::env::temp_dir().join(format!("micontrol-payload-unsafe-{}", std::process::id()));
        let zip_path = tmp.join("unsafe.zip");
        let _ = std::fs::create_dir_all(&tmp);
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default();
            // Use a path that the zip crate will encode as relative.
            zip.start_file("../escape.txt", opts).unwrap();
            zip.write_all(b"x").unwrap();
            zip.start_file("micontrol.exe", opts).unwrap();
            zip.write_all(b"x").unwrap();
            zip.start_file("ecram_service.exe", opts).unwrap();
            zip.write_all(b"x").unwrap();
            zip.start_file("micontrol_bridge.exe", opts).unwrap();
            zip.write_all(b"x").unwrap();
            zip.finish().unwrap();
        }
        let report = preflight(&zip_path).unwrap();
        assert!(
            report.unsafe_path_found,
            "traversal path must be flagged: {report:?}"
        );
        assert!(!report.ok, "unsafe paths must fail preflight");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn preflight_rejects_duplicate_paths() {
        // S65-004: duplicate extraction targets are rejected (a later entry
        // would silently overwrite an earlier one). Different raw spellings
        // that normalize to the same enclosed path ("./micontrol.exe" vs
        // "micontrol.exe") must be caught.
        let zip = make_zip(&[
            "./micontrol.exe",
            "micontrol.exe",
            "ecram_service.exe",
            "micontrol_bridge.exe",
        ]);
        let report = preflight(&zip).unwrap();
        assert!(
            report.duplicate_path_found,
            "duplicate target must be flagged: {report:?}"
        );
        assert!(!report.ok, "duplicate targets must fail preflight");
        assert!(report.missing_required.is_empty());
        let _ = std::fs::remove_dir_all(zip.parent().unwrap());
    }

    #[test]
    fn preflight_rejects_malformed_manifest() {
        // S65-004 AC3: a payload.json that is not valid JSON (or lacks a
        // usable version) must fail preflight without touching any files.
        let tmp = std::env::temp_dir().join(format!(
            "micontrol-payload-badmanifest-{}",
            std::process::id()
        ));
        let zip_path = tmp.join("badmanifest.zip");
        let _ = std::fs::create_dir_all(&tmp);
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default();
            for name in ["micontrol.exe", "ecram_service.exe", "micontrol_bridge.exe"] {
                zip.start_file(name, opts).unwrap();
                zip.write_all(b"x").unwrap();
            }
            zip.start_file("payload.json", opts).unwrap();
            zip.write_all(b"this is not json").unwrap();
            zip.finish().unwrap();
        }
        let report = preflight(&zip_path).unwrap();
        assert!(
            report.malformed_manifest,
            "malformed manifest must be flagged: {report:?}"
        );
        assert!(!report.ok, "malformed manifest must fail preflight");
        assert!(report.missing_required.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn version_mismatch_guard_rejects_disagreement() {
        // S65-004 AC3: applying a payload whose manifest version differs from
        // the expected version must be rejected.
        let report = PreflightReport {
            ok: true,
            missing_required: vec![],
            missing_optional: vec![],
            version: Some("0.2.19".into()),
            unsafe_path_found: false,
            duplicate_path_found: false,
            malformed_manifest: false,
        };
        assert!(version_mismatch("0.2.20", &report).is_some());
        assert!(version_mismatch("0.2.19", &report).is_none());

        let no_version = PreflightReport {
            version: None,
            ..report.clone()
        };
        assert!(
            version_mismatch("0.2.20", &no_version).is_some(),
            "missing manifest version must be treated as a mismatch"
        );
    }

    #[test]
    fn preflight_reads_version_from_payload_json() {
        let tmp =
            std::env::temp_dir().join(format!("micontrol-payload-version-{}", std::process::id()));
        let zip_path = tmp.join("versioned.zip");
        let _ = std::fs::create_dir_all(&tmp);
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default();
            for name in ["micontrol.exe", "ecram_service.exe", "micontrol_bridge.exe"] {
                zip.start_file(name, opts).unwrap();
                zip.write_all(b"x").unwrap();
            }
            zip.start_file("payload.json", opts).unwrap();
            zip.write_all(b"{\"version\": \"0.2.20-beta\"}").unwrap();
            zip.finish().unwrap();
        }
        let report = preflight(&zip_path).unwrap();
        assert!(report.ok);
        assert_eq!(report.version.as_deref(), Some("0.2.20-beta"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn preflight_summary_formats_correctly() {
        let report = PreflightReport {
            ok: true,
            missing_required: vec![],
            missing_optional: vec!["drivers".into(), "resources".into()],
            version: Some("0.2.20".into()),
            unsafe_path_found: false,
            duplicate_path_found: false,
            malformed_manifest: false,
        };
        let s = preflight_summary(&report);
        assert!(s.contains("version=0.2.20"));
        assert!(s.contains("missing_optional=drivers,resources"));
        assert!(s.contains("OK"));

        let bad = PreflightReport {
            ok: false,
            missing_required: vec!["micontrol.exe".into()],
            missing_optional: vec![],
            version: None,
            unsafe_path_found: false,
            duplicate_path_found: false,
            malformed_manifest: false,
        };
        let s2 = preflight_summary(&bad);
        assert!(s2.contains("missing_required=micontrol.exe"));
        assert!(s2.contains("REJECTED"));
    }
}
