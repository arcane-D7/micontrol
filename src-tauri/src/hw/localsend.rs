//! LocalSend protocol v2.2 (MIOT-06).
//!
//! A self-contained implementation of the LocalSend wire protocol for LAN
//! peer discovery and file transfer — no external server required.
//!
//! ```text
//!   UDP multicast discovery  224.0.0.167:53317
//!   HTTP (plain) endpoints   :53317  /api/localsend/v2/{register,
//!                                       prepare-upload, upload, cancel, info}
//! ```
//!
//! What is implemented:
//! - **Discovery**: send an announcement to the multicast group; collect
//!   replies (both UDP `announce:false` fallback replies and HTTP register
//!   replies when our receiver is running). Self packets are filtered by
//!   fingerprint.
//! - **Sender**: `prepare-upload` (file metadata incl. SHA-256) then
//!   `upload` (raw bytes, `Content-Length`-framed) against a discovered peer.
//!   Sending still buffers the file in memory; the bounded path is the
//!   receiver.
//! - **Receiver**: an embedded HTTP server on TCP `53317` that asks the user
//!   before accepting a transfer (Accept/Decline prompt in the UI, with an
//!   opt-in per-device auto-accept list) and streams the body straight to the
//!   user's Downloads folder, bounded by a configurable size limit and
//!   verified against the announced SHA-256 (HTTP 422 on mismatch).
//!
//! Security notes (documented limitation):
//! - LocalSend is designed to negotiate TLS with a self-signed certificate in
//!   HTTPS mode. This module currently advertises plain `http` and, when
//!   talking to an HTTPS peer, disables certificate validation because the
//!   fingerprint (not the CA chain) is the trust anchor. Only use on trusted
//!   LANs.
//! - The receiver never writes anything before consent: `prepare-upload`
//!   blocks until the user answers (or the 60 s window expires) and, with no UI
//!   to ask, the transfer is declined. File names are sanitized and written to
//!   Downloads only (no path traversal, no overwrite).

use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use tauri::Emitter;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::util::panic::lock_or_recover;

/// LocalSend protocol major.minor advertised to peers.
pub const PROTOCOL_VERSION: &str = "2.0";
/// Multicast group used for discovery.
pub const LS_MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);
/// Default UDP + HTTP port.
pub const LS_PORT: u16 = 53317;
/// Device type advertised in info packets.
pub const DEVICE_TYPE: &str = "desktop";
/// Receive buffer size for discovery packets.
const RECV_BUF: usize = 4096;

// ── Wire types ───────────────────────────────────────────────────────────────

/// Info/announcement payload shared by all LocalSend endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalSendInfo {
    pub alias: String,
    pub version: String,
    #[serde(rename = "deviceModel", skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    #[serde(rename = "deviceType", skip_serializing_if = "Option::is_none")]
    pub device_type: Option<String>,
    pub fingerprint: String,
    pub port: u16,
    pub protocol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub announce: Option<bool>,
}

/// A discovered peer, as surfaced to the UI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalSendPeer {
    pub alias: String,
    pub fingerprint: String,
    /// "http" | "https"
    pub protocol: String,
    pub port: u16,
    pub addr: IpAddr,
    #[serde(rename = "deviceType")]
    pub device_type: Option<String>,
    pub download: bool,
}

/// Meta of a single outgoing file.
#[derive(Debug, Clone, Serialize)]
pub struct LocalSendFile {
    pub id: String,
    #[serde(rename = "fileName")]
    pub name: String,
    pub size: u64,
    #[serde(rename = "fileType")]
    pub file_type: String,
    #[serde(rename = "sha256")]
    pub sha256: String,
}

/// Result of a completed send session.
#[derive(Debug, Clone, Serialize)]
pub struct SendReport {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    /// Names of the sent files.
    pub files: Vec<String>,
    pub total_bytes: u64,
}

/// Receiver server status surfaced to the UI.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReceiverStatus {
    pub running: bool,
    pub port: u16,
    pub received_count: u64,
}

// ── Registry / identity helpers ──────────────────────────────────────────────

/// Stable per-process identity so we do not discover ourselves.
fn fingerprint() -> &'static str {
    static FP: OnceLock<String> = OnceLock::new();
    FP.get_or_init(|| {
        let mut bytes = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    })
}

/// Human-readable alias shown to peers (hostname).
fn device_alias() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "MiControl".into())
}

/// Build our info packet; `announce` is None for a plain response and
/// Some(true) for a multicast announcement.
fn our_info(announce: bool) -> LocalSendInfo {
    LocalSendInfo {
        alias: device_alias(),
        version: PROTOCOL_VERSION.into(),
        device_model: Some(std::env::consts::OS.into()),
        device_type: Some(DEVICE_TYPE.into()),
        fingerprint: fingerprint().into(),
        port: LS_PORT,
        protocol: "http".into(),
        download: Some(true),
        announce: Some(announce),
    }
}

/// Random lowercase hex id (32 chars), used for file ids and session tokens.
fn random_hex() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Convert a received info packet (plus sender address) into a peer entry.
fn info_to_peer(info: &LocalSendInfo, addr: IpAddr) -> LocalSendPeer {
    LocalSendPeer {
        alias: info.alias.clone(),
        fingerprint: info.fingerprint.clone(),
        protocol: info.protocol.clone(),
        port: info.port,
        addr,
        device_type: info.device_type.clone(),
        download: info.download.unwrap_or(false),
    }
}

// ── Discovery ─────────────────────────────────────────────────────────────────

/// Bind a UDP socket for multicast discovery. Prefers the standard port; falls
/// back to an ephemeral port when the standard one is taken (e.g. the official
/// LocalSend app is running) so we can still initiate discovery.
fn discovery_socket() -> std::io::Result<tokio::net::UdpSocket> {
    let std_sock = std::net::UdpSocket::bind(format!("0.0.0.0:{LS_PORT}"))
        .or_else(|_| std::net::UdpSocket::bind("0.0.0.0:0"))?;
    std_sock.set_nonblocking(true)?;
    // Joining may fail on some networks; discovery still works for replies to
    // our announcement (peers unicast back), so the error is non-fatal.
    let _ = std_sock.join_multicast_v4(&LS_MULTICAST_GROUP, &Ipv4Addr::UNSPECIFIED);
    tokio::net::UdpSocket::from_std(std_sock)
}

/// Send an announcement and collect peers for `duration`.
///
/// Always sends the announcement so peers react (register reply or UDP
/// response). If the embedded receiver is running, HTTP register replies are
/// also processed by the server loop; here we surface everything that comes
/// back over UDP.
pub async fn discover_peers(duration: Duration) -> Vec<LocalSendPeer> {
    let Ok(socket) = discovery_socket() else {
        return Vec::new();
    };
    let announce = serde_json::to_vec(&our_info(true)).unwrap_or_default();
    let group = SocketAddr::new(IpAddr::V4(LS_MULTICAST_GROUP), LS_PORT);
    let _ = socket.send_to(&announce, group).await;

    let mut peers: Vec<LocalSendPeer> = Vec::new();
    let mut buf = [0u8; RECV_BUF];
    let deadline = std::time::Instant::now() + duration;
    while std::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match tokio::time::timeout(remaining, socket.recv_from(&mut buf)).await {
            Ok(Ok((n, from))) => {
                if let Some(info) = parse_info_payload(&buf[..n]) {
                    // If it is an announcement, answer so the peer sees us.
                    if info.announce.unwrap_or(false) {
                        let reply = serde_json::to_vec(&our_info(false)).unwrap_or_default();
                        let _ = socket.send_to(&reply, from).await;
                    }
                    let peer = info_to_peer(&info, from.ip());
                    if !peers.iter().any(|p| p.fingerprint == peer.fingerprint) {
                        peers.push(peer);
                    }
                }
            }
            _ => break,
        }
    }
    peers
}

/// Parse a UDP discovery payload into an info packet; ignores self-sent
/// packets (matching fingerprint).
fn parse_info_payload(payload: &[u8]) -> Option<LocalSendInfo> {
    let info: LocalSendInfo = serde_json::from_slice(payload).ok()?;
    if info.fingerprint == fingerprint() {
        return None;
    }
    Some(info)
}

// ── Sender ───────────────────────────────────────────────────────────────────

/// HTTP client used for both http and https (self-signed) peers.
///
/// # Security
/// HTTPS peers present a locally generated certificate; per the LocalSend
/// design the fingerprint is the trust anchor, so certificate-chain
/// validation is skipped (LAN-only usage). Plain-HTTP peers need no TLS at all.
fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}

/// Best-effort mime guess for the advertised file type.
fn guess_mime(name: &str) -> String {
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    let map: &[(&str, &str)] = &[
        ("png", "image/png"),
        ("jpg", "image/jpeg"),
        ("jpeg", "image/jpeg"),
        ("gif", "image/gif"),
        ("webp", "image/webp"),
        ("bmp", "image/bmp"),
        ("mp4", "video/mp4"),
        ("mkv", "video/x-matroska"),
        ("webm", "video/webm"),
        ("mov", "video/quicktime"),
        ("mp3", "audio/mpeg"),
        ("wav", "audio/wav"),
        ("ogg", "audio/ogg"),
        ("flac", "audio/flac"),
        ("pdf", "application/pdf"),
        ("txt", "text/plain"),
        ("md", "text/markdown"),
        ("json", "application/json"),
        ("zip", "application/zip"),
        ("7z", "application/x-7z-compressed"),
        ("rar", "application/vnd.rar"),
        ("tar", "application/x-tar"),
        ("gz", "application/gzip"),
        ("apk", "application/vnd.android.package-archive"),
        ("exe", "application/x-msdownload"),
        ("msi", "application/x-msi"),
    ];
    map.iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, m)| (*m).to_string())
        .unwrap_or_else(|| "application/octet-stream".into())
}

/// Build the file metadata (id, name, size, sha256) for one file.
fn collect_file_meta(path: &Path) -> Result<(LocalSendFile, PathBuf), String> {
    let meta =
        std::fs::metadata(path).map_err(|e| format!("Cannot read '{}': {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("'{}' is not a file", path.display()));
    }
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();
    // Streaming SHA-256 so large files do not explode memory.
    let mut hasher = Sha256::new();
    let mut f =
        std::fs::File::open(path).map_err(|e| format!("Cannot open '{}': {e}", path.display()))?;
    std::io::copy(&mut f, &mut hasher)
        .map_err(|e| format!("Error hashing '{}': {e}", path.display()))?;
    let sha256 = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((
        LocalSendFile {
            id: random_hex(),
            name: name.clone(),
            size: meta.len(),
            file_type: guess_mime(&name),
            sha256,
        },
        path.to_path_buf(),
    ))
}

/// Build the base URL for a peer ("http://1.2.3.4" — IPv6 gets brackets).
fn peer_base_url(peer: &LocalSendPeer) -> String {
    let host = match peer.addr {
        IpAddr::V6(v6) => format!("[{v6}]"),
        IpAddr::V4(v4) => v4.to_string(),
    };
    format!("{}://{host}:{}", peer.protocol, peer.port)
}

/// Send files to a peer following the LocalSend quote-then-upload flow.
pub async fn send_files(peer: &LocalSendPeer, paths: &[String]) -> Result<SendReport, String> {
    if paths.is_empty() {
        return Err("No files selected".into());
    }
    let client = http_client()?;
    let base = peer_base_url(peer);

    let mut metas = Vec::new();
    let mut files_json = serde_json::Map::new();
    for p in paths {
        let (meta, path) = collect_file_meta(Path::new(p))?;
        files_json.insert(
            meta.id.clone(),
            serde_json::to_value(&meta).map_err(|e| e.to_string())?,
        );
        metas.push((meta, path));
    }

    // 1. prepare-upload (metadata only) → {sessionId, files:{id: token}}
    let prepare_body = serde_json::json!({
        "info": our_info(false),
        "files": serde_json::Value::Object(files_json),
    });
    let resp = client
        .post(format!("{base}/api/localsend/v2/prepare-upload"))
        .json(&prepare_body)
        .send()
        .await
        .map_err(|e| format!("Prepare failed ({base}): {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("Receiver rejected transfer (HTTP {status})"));
    }
    let prepared: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Bad prepare response: {e}"))?;
    let session_id = prepared
        .get("sessionId")
        .and_then(|v| v.as_str())
        .ok_or("Missing sessionId in prepare response")?
        .to_string();
    let tokens = prepared
        .get("files")
        .and_then(|v| v.as_object())
        .ok_or("Missing files map in prepare response")?;

    // 2. upload each file. Parallel is allowed by the protocol but sequential
    //    keeps the report order stable and is kinder to small LANs.
    let mut sent = Vec::new();
    let mut total_bytes = 0u64;
    for (meta, path) in &metas {
        let token = tokens
            .get(&meta.id)
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("No token granted for '{}'", meta.name))?;
        let url = format!(
            "{base}/api/localsend/v2/upload?sessionId={session_id}&fileId={}&token={token}",
            meta.id
        );
        // NOTE: the sender still reads the file into memory (`fs::read`). The
        // receiver is the bounded path (see `UploadSink`); streaming the sender
        // would mean chunked encoding or an exact `size_hint`, and this
        // receiver — like the LocalSend protocol's own — frames uploads by
        // `Content-Length`. Tracked as a follow-up, not a silent compromise.
        let bytes = std::fs::read(path).map_err(|e| format!("Cannot read '{}': {e}", meta.name))?;
        let up = client
            .post(&url)
            .header("Content-Type", "application/octet-stream")
            .body(bytes)
            .send()
            .await
            .map_err(|e| format!("Upload failed for '{}': {e}", meta.name))?;
        let ustatus = up.status();
        if !ustatus.is_success() {
            return Err(format!(
                "Upload of '{}' rejected (HTTP {ustatus})",
                meta.name
            ));
        }
        total_bytes += meta.size;
        sent.push(meta.name.clone());
    }

    Ok(SendReport {
        session_id,
        files: sent,
        total_bytes,
    })
}

// ── Receiver configuration (consent + limits) ───────────────────────────────

/// Registry key holding the receiver settings.
const CONFIG_KEY: &str = r"SOFTWARE\MiControl\LocalSend";
const K_AUTO_ACCEPT: &str = "AutoAcceptFingerprints";
const K_MAX_UPLOAD_MB: &str = "MaxUploadMb";

/// Largest single file accepted by default, in MiB.
pub const DEFAULT_MAX_UPLOAD_MB: u64 = 2048;
/// Upper bound the settings command accepts, so a typo cannot remove the cap.
pub const MAX_MAX_UPLOAD_MB: u64 = 65_536;

/// Cap on the `prepare-upload` JSON body (metadata only).
const MAX_PREPARE_BYTES: usize = 256 * 1024;
/// Cap on the request head (request line + headers).
const MAX_HEAD_BYTES: usize = 64 * 1024;
/// How long the receiver waits for the user's answer before declining.
const CONSENT_TIMEOUT: Duration = Duration::from_secs(60);
/// Files per transfer session.
const MAX_FILES_PER_SESSION: usize = 64;
/// How many devices can be auto-accepted (keeps the registry value small).
const MAX_AUTO_ACCEPT: usize = 32;
/// Concurrent prompts: more than this is a flood, not a user decision.
const MAX_PENDING_PROMPTS: usize = 4;
/// Streaming chunk size for uploads.
const UPLOAD_CHUNK: usize = 64 * 1024;
/// Event the UI listens to in order to show the consent prompt.
pub const EVENT_INCOMING: &str = "localsend-incoming";

fn default_max_upload_mb() -> u64 {
    DEFAULT_MAX_UPLOAD_MB
}

/// Receiver settings, persisted per user.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalSendConfig {
    /// Fingerprints of devices whose transfers are accepted without a prompt.
    /// Empty by default: consent is required until the user opts a device in.
    #[serde(default)]
    pub auto_accept_fingerprints: Vec<String>,
    /// Largest single file the receiver accepts, in MiB.
    #[serde(default = "default_max_upload_mb")]
    pub max_upload_mb: u64,
}

impl Default for LocalSendConfig {
    fn default() -> Self {
        Self {
            auto_accept_fingerprints: Vec::new(),
            max_upload_mb: DEFAULT_MAX_UPLOAD_MB,
        }
    }
}

/// Parse the comma/semicolon separated fingerprint list stored in the registry.
fn parse_auto_accept(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in raw.split([',', ';', '\n']) {
        let fp = part.trim();
        if fp.is_empty() || fp.len() > 128 {
            continue;
        }
        if out.len() < MAX_AUTO_ACCEPT && !out.iter().any(|x| x.eq_ignore_ascii_case(fp)) {
            out.push(fp.to_string());
        }
    }
    out
}

impl LocalSendConfig {
    pub fn load() -> Self {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_CURRENT_USER;

        let mut cfg = Self::default();
        let Ok(Some(key)) = RegKeyGuard::open_read(HKEY_CURRENT_USER, CONFIG_KEY) else {
            return cfg;
        };
        if let Ok(Some(raw)) = key.read_string(K_AUTO_ACCEPT) {
            cfg.auto_accept_fingerprints = parse_auto_accept(&raw);
        }
        if let Ok(Some(mb)) = key.read_u32(K_MAX_UPLOAD_MB) {
            cfg.max_upload_mb = (mb as u64).clamp(1, MAX_MAX_UPLOAD_MB);
        }
        cfg
    }

    pub fn save(&self) {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_CURRENT_USER;

        if let Ok(key) = RegKeyGuard::create_write(HKEY_CURRENT_USER, CONFIG_KEY) {
            let _ = key.write_string(K_AUTO_ACCEPT, &self.auto_accept_fingerprints.join(","));
            let _ = key.write_u32(
                K_MAX_UPLOAD_MB,
                self.max_upload_mb.clamp(1, MAX_MAX_UPLOAD_MB) as u32,
            );
        }
    }

    /// Per-file and per-transfer byte cap.
    pub fn max_upload_bytes(&self) -> u64 {
        self.max_upload_mb
            .clamp(1, MAX_MAX_UPLOAD_MB)
            .saturating_mul(1024 * 1024)
    }

    pub fn is_auto_accepted(&self, fingerprint: &str) -> bool {
        self.auto_accept_fingerprints
            .iter()
            .any(|fp| fp.eq_ignore_ascii_case(fingerprint))
    }

    /// Opt a device in. Returns `true` when the list changed.
    pub fn allow_device(&mut self, fingerprint: &str) -> bool {
        let fp = fingerprint.trim();
        if fp.is_empty() || fp.len() > 128 || self.is_auto_accepted(fp) {
            return false;
        }
        if self.auto_accept_fingerprints.len() >= MAX_AUTO_ACCEPT {
            self.auto_accept_fingerprints.remove(0);
        }
        self.auto_accept_fingerprints.push(fp.to_string());
        true
    }

    /// Opt a device out. Returns `true` when the list changed.
    pub fn forget_device(&mut self, fingerprint: &str) -> bool {
        let before = self.auto_accept_fingerprints.len();
        self.auto_accept_fingerprints
            .retain(|fp| !fp.eq_ignore_ascii_case(fingerprint.trim()));
        self.auto_accept_fingerprints.len() != before
    }
}

// ── Receiver (embedded HTTP server) ──────────────────────────────────────────

/// Immutable receiver globals.
struct ReceiverState {
    running: AtomicBool,
    received_count: AtomicU64,
    sessions: std::sync::Mutex<HashMap<String, ReceiverSession>>,
    /// Transfers waiting for the user's answer, keyed by session id.
    pending: std::sync::Mutex<HashMap<String, PendingConsent>>,
    /// Handle used to ask the UI for consent; set by `start_receiver`.
    app: OnceLock<tauri::AppHandle>,
}

#[derive(Default)]
struct ReceiverSession {
    /// fileId → (token, file name, size, announced sha256)
    files: HashMap<String, (String, String, u64, Option<String>)>,
}

/// A `prepare-upload` waiting for the user to accept or decline.
struct PendingConsent {
    /// Fingerprint of the device that asked (used by "remember this device",
    /// so the UI can only whitelist the device that is actually waiting).
    fingerprint: String,
    decision: tokio::sync::oneshot::Sender<bool>,
}

static RECEIVER: OnceLock<ReceiverState> = OnceLock::new();

fn receiver_state() -> &'static ReceiverState {
    RECEIVER.get_or_init(|| ReceiverState {
        running: AtomicBool::new(false),
        received_count: AtomicU64::new(0),
        sessions: std::sync::Mutex::new(HashMap::new()),
        pending: std::sync::Mutex::new(HashMap::new()),
        app: OnceLock::new(),
    })
}

fn app_handle() -> Option<tauri::AppHandle> {
    receiver_state().app.get().cloned()
}

/// One file announced by a sender, as shown in the consent prompt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IncomingFile {
    pub name: String,
    pub size: u64,
}

/// Payload of [`EVENT_INCOMING`]: everything the UI needs to ask the user.
#[derive(Debug, Clone, Serialize)]
pub struct IncomingTransfer {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub alias: String,
    pub fingerprint: String,
    #[serde(rename = "deviceType")]
    pub device_type: Option<String>,
    pub files: Vec<IncomingFile>,
    #[serde(rename = "totalBytes")]
    pub total_bytes: u64,
    #[serde(rename = "expiresInSeconds")]
    pub expires_in_seconds: u64,
}

/// How an incoming transfer is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConsentAction {
    /// The device is on the user's auto-accept list.
    AutoAccept,
    /// Ask the user through the UI.
    Prompt,
    /// No UI to ask: decline. Silently accepting without consent is exactly
    /// the behaviour this module must not have.
    Deny,
}

fn consent_action(auto_accepted: bool, ui_available: bool) -> ConsentAction {
    if auto_accepted {
        ConsentAction::AutoAccept
    } else if ui_available {
        ConsentAction::Prompt
    } else {
        ConsentAction::Deny
    }
}

/// Fingerprint of the device waiting on `session_id`, if the prompt is still
/// pending.
pub fn pending_device(session_id: &str) -> Option<String> {
    lock_or_recover(&receiver_state().pending)
        .get(session_id)
        .map(|p| p.fingerprint.clone())
}

/// Answer a pending consent prompt. Returns `false` when the session is gone
/// (already answered, cancelled, or the 60 s window expired).
pub fn respond_consent(session_id: &str, accept: bool) -> bool {
    let pending = lock_or_recover(&receiver_state().pending).remove(session_id);
    match pending {
        Some(p) => p.decision.send(accept).is_ok(),
        None => false,
    }
}

/// Start the embedded HTTP receiver on port 53317 (idempotent).
///
/// `app` is kept so incoming transfers can ask the UI for consent; without it
/// every transfer is declined.
pub fn start_receiver(app: tauri::AppHandle) -> Result<u16, String> {
    let state = receiver_state();
    let _ = state.app.set(app);
    if state.running.load(Ordering::Relaxed) {
        return Ok(LS_PORT);
    }
    let listener = std::net::TcpListener::bind(("0.0.0.0", LS_PORT))
        .map_err(|e| format!("Cannot listen on port {LS_PORT} (is LocalSend running?): {e}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("Cannot set non-blocking: {e}"))?;
    let listener = tokio::net::TcpListener::from_std(listener)
        .map_err(|e| format!("Cannot wrap listener: {e}"))?;
    state.running.store(true, Ordering::Relaxed);
    tokio::spawn(async move {
        // Poll the running flag every second even while idle, so stop_receiver()
        // releases the port promptly (otherwise a restart would hit "in use").
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if !receiver_state().running.load(Ordering::Relaxed) {
                        break;
                    }
                }
                res = listener.accept() => match res {
                    Ok((stream, _)) => {
                        tokio::spawn(async move {
                            let _ = handle_connection(stream).await;
                        });
                    }
                    Err(_) => {
                        if !receiver_state().running.load(Ordering::Relaxed) {
                            break;
                        }
                    }
                }
            }
        }
    });
    Ok(LS_PORT)
}

/// Stop the embedded receiver (the accept task exits on next error or when a
/// new connection checks the flag; in-flight transfers finish).
pub fn stop_receiver() {
    receiver_state().running.store(false, Ordering::Relaxed);
    // Nobody can answer a prompt once the receiver is stopped.
    let pending = std::mem::take(&mut *lock_or_recover(&receiver_state().pending));
    for (_, p) in pending {
        let _ = p.decision.send(false);
    }
}

/// Current receiver status.
pub fn receiver_status() -> ReceiverStatus {
    let state = receiver_state();
    ReceiverStatus {
        running: state.running.load(Ordering::Relaxed),
        port: LS_PORT,
        received_count: state.received_count.load(Ordering::Relaxed),
    }
}

/// Downloads directory (Windows USERPROFILE → HOME → current dir fallback).
fn downloads_dir() -> PathBuf {
    if let Ok(profile) = std::env::var("USERPROFILE") {
        let d = PathBuf::from(profile).join("Downloads");
        if d.is_dir() {
            return d;
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let d = PathBuf::from(home).join("Downloads");
        if d.is_dir() {
            return d;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| ".".into())
}

/// Strip path components and characters illegal on Windows.
///
/// Splits on both `/` and `\` manually instead of using `Path::file_name()`
/// because on Windows `Path` interprets `a:name` as a drive prefix and would
/// silently drop the `a` (the sanitize test asserts `a:b…` becomes `ab…`).
fn sanitize_file_name(name: &str) -> String {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("received_file");
    let cleaned: String = base
        .chars()
        .filter(|c| {
            !matches!(
                c,
                '/' | '\\' | '\0' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            )
        })
        .filter(|c| !c.is_control())
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    // Windows paths are limited to ~260 characters; leave room for the
    // " (n)" suffix and the temporary `.part` name.
    let trimmed: String = trimmed.chars().take(180).collect();
    if trimmed.is_empty() {
        "received_file".into()
    } else {
        trimmed
    }
}

/// Build a non-colliding path: `x.txt`, `x (1).txt`, `x (2).txt`, …
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let stem = Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let ext = Path::new(name)
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    for i in 1..10_000 {
        let candidate_name = if ext.is_empty() {
            format!("{stem} ({i})")
        } else {
            format!("{stem} ({i}).{ext}")
        };
        let candidate = dir.join(&candidate_name);
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!(
        "{stem}-{}.txt",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    ))
}

/// Parse an HTTP request line + headers from the read buffer.
///
/// Returns `(method, raw_target, header_map, header_len)` where `header_len`
/// is the byte offset of the first body byte (just past the blank line that
/// ends the headers).
fn parse_http_head(buf: &[u8]) -> Option<(String, String, HashMap<String, String>, usize)> {
    let head_end = find_head_end(buf)?;
    let head = std::str::from_utf8(&buf[..head_end]).ok()?;
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    // `find_head_end` already returns the index just past `\r\n\r\n`, i.e. the
    // first body byte. (It used to add 2 here, which silently dropped the first
    // two bytes of every body and made the size/checksum checks fail.)
    Some((method, target, headers, head_end))
}

/// Index just past `\r\n\r\n` (end of headers).
fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

/// Write a minimal HTTP/1.1 response and close.
async fn write_response(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await
}

/// Query parameter extraction (`?a=1&b=2` → map).
fn parse_query(target: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(q) = target.split_once('?') else {
        return out;
    };
    for pair in q.1.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            out.insert(k.to_string(), v.to_string());
        }
    }
    out
}

/// Read a bounded JSON body, reusing whatever the head read already buffered.
async fn read_small_body(
    stream: &mut tokio::net::TcpStream,
    head: &[u8],
    head_len: usize,
    content_length: Option<u64>,
) -> Result<Vec<u8>, u16> {
    let want = content_length.unwrap_or(0);
    if want > MAX_PREPARE_BYTES as u64 {
        return Err(413);
    }
    let mut body = head[head_len..].to_vec();
    body.truncate(want as usize);
    while (body.len() as u64) < want {
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).await.map_err(|_| 400u16)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
        if body.len() > MAX_PREPARE_BYTES {
            return Err(413);
        }
    }
    Ok(body)
}

/// Handle one accepted connection: read the request head, dispatch, respond.
///
/// The body is *not* buffered here: `upload` streams it straight to disk and
/// the JSON endpoints read a bounded body. The old code read `content-length`
/// bytes into a `Vec` before dispatching, so a 10 GB upload meant 10 GB of RAM.
async fn handle_connection(mut stream: tokio::net::TcpStream) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(8192);
    let mut tmp = [0u8; 1024];
    // Read until the header terminator or a bounded cap.
    loop {
        if find_head_end(&buf).is_some() {
            break;
        }
        if buf.len() > MAX_HEAD_BYTES {
            return write_response(
                &mut stream,
                431,
                "Request Header Fields Too Large",
                "text/plain",
                b"head too large",
            )
            .await;
        }
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    // parse_http_head returns owned Strings, releasing any borrow of `buf`.
    let (method, target, headers, head_len) = match parse_http_head(&buf) {
        Some(v) => v,
        None => {
            return write_response(&mut stream, 400, "Bad Request", "text/plain", b"bad head")
                .await;
        }
    };
    let content_length: Option<u64> = headers
        .get("content-length")
        .and_then(|cl| cl.trim().parse().ok());
    let chunked = headers
        .get("transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    let query = parse_query(&target);
    let path = target.split('?').next().unwrap_or(&target).to_string();

    match (method.as_str(), path.as_str()) {
        ("POST", "/api/localsend/v2/register") => {
            let info = serde_json::to_vec(&our_info(false)).unwrap_or_default();
            write_response(&mut stream, 200, "OK", "application/json", &info).await
        }
        ("POST", "/api/localsend/v2/prepare-upload") => {
            let body = match read_small_body(&mut stream, &buf, head_len, content_length).await {
                Ok(body) => body,
                Err(code) => {
                    return write_response(
                        &mut stream,
                        code,
                        "Payload Too Large",
                        "text/plain",
                        b"body too large",
                    )
                    .await
                }
            };
            match handle_prepare_upload(&body).await {
                Ok(resp) => write_response(&mut stream, 200, "OK", "application/json", &resp).await,
                Err(code) => {
                    write_response(&mut stream, code, "Rejected", "application/json", b"{}").await
                }
            }
        }
        ("POST", "/api/localsend/v2/upload") => {
            // Chunked framing would be written into the file verbatim (the
            // upload path streams the raw body), so refuse it instead of
            // corrupting the file.
            if chunked {
                return write_response(
                    &mut stream,
                    411,
                    "Length Required",
                    "text/plain",
                    b"Content-Length required",
                )
                .await;
            }
            let code = handle_upload(&mut stream, &buf, head_len, content_length, &query).await;
            let (status, reason, out) = match code {
                200 => (200, "OK", b"".to_vec()),
                400 => (400, "Bad Request", b"size mismatch".to_vec()),
                403 => (403, "Forbidden", b"invalid token".to_vec()),
                409 => (409, "Conflict", b"blocked".to_vec()),
                411 => (411, "Length Required", b"length required".to_vec()),
                413 => (413, "Payload Too Large", b"file too large".to_vec()),
                422 => (422, "Unprocessable", b"checksum mismatch".to_vec()),
                _ => (500, "Error", b"server error".to_vec()),
            };
            write_response(&mut stream, status, reason, "text/plain", &out).await
        }
        ("POST", "/api/localsend/v2/cancel") => {
            if let Some(sid) = query.get("sessionId") {
                lock_or_recover(&receiver_state().sessions).remove(sid);
                // A cancel also answers a pending prompt, as a decline.
                let _ = respond_consent(sid, false);
            }
            write_response(&mut stream, 200, "OK", "text/plain", b"").await
        }
        ("GET", "/api/localsend/v2/info") => {
            let info = serde_json::to_vec(&our_info(false)).unwrap_or_default();
            write_response(&mut stream, 200, "OK", "application/json", &info).await
        }
        _ => write_response(&mut stream, 404, "Not Found", "text/plain", b"not found").await,
    }
}

/// `prepare-upload` request body.
#[derive(Deserialize)]
struct PrepareUploadRequest {
    info: LocalSendInfo,
    files: HashMap<String, PrepareFile>,
}
#[derive(Deserialize)]
struct PrepareFile {
    #[serde(rename = "fileName", default)]
    file_name: String,
    #[serde(default)]
    size: u64,
    #[serde(rename = "sha256")]
    sha256: Option<String>,
}

/// Validate a `prepare-upload` and describe what the user is being asked to
/// accept. Pure, so the limits are unit-testable without a socket.
fn describe_transfer(
    req: &PrepareUploadRequest,
    max_bytes: u64,
) -> Result<(Vec<IncomingFile>, u64), u16> {
    if req.files.is_empty() || req.files.len() > MAX_FILES_PER_SESSION {
        return Err(400);
    }
    let mut files = Vec::with_capacity(req.files.len());
    let mut total: u64 = 0;
    for f in req.files.values() {
        if f.size > max_bytes {
            return Err(413);
        }
        total = total.saturating_add(f.size);
        files.push(IncomingFile {
            name: sanitize_file_name(&f.file_name),
            size: f.size,
        });
    }
    if total > max_bytes {
        return Err(413);
    }
    Ok((files, total))
}

/// Ask the user (or the auto-accept list) before granting upload tokens.
///
/// Returns the JSON body or an HTTP error code. Nothing is written to disk
/// until the user accepts: the tokens handed out here are the only way an
/// `upload` can name a file.
async fn handle_prepare_upload(body: &[u8]) -> Result<Vec<u8>, u16> {
    let req: PrepareUploadRequest = serde_json::from_slice(body).map_err(|_| 400u16)?;
    // Ignore announcements coming from ourselves (should not happen, but the
    // sender fingerprint is the identity guard).
    if req.info.fingerprint == fingerprint() {
        return Err(403);
    }

    let cfg = LocalSendConfig::load();
    let (incoming, total_bytes) = describe_transfer(&req, cfg.max_upload_bytes())?;
    let session_id = random_hex();

    match consent_action(
        cfg.is_auto_accepted(&req.info.fingerprint),
        app_handle().is_some(),
    ) {
        ConsentAction::Deny => {
            log::warn!(
                "[localsend] declined transfer from '{}' ({}): no UI available to ask for consent",
                req.info.alias,
                req.info.fingerprint
            );
            return Err(403);
        }
        ConsentAction::AutoAccept => {
            log::info!(
                "[localsend] auto-accepting {} file(s) ({} bytes) from '{}' (device on the auto-accept list)",
                incoming.len(),
                total_bytes,
                req.info.alias
            );
        }
        ConsentAction::Prompt => {
            let Some(app) = app_handle() else {
                return Err(403);
            };
            if lock_or_recover(&receiver_state().pending).len() >= MAX_PENDING_PROMPTS {
                log::warn!(
                    "[localsend] refusing transfer from '{}': {} prompts already waiting",
                    req.info.alias,
                    MAX_PENDING_PROMPTS
                );
                return Err(409);
            }
            let (tx, rx) = tokio::sync::oneshot::channel();
            lock_or_recover(&receiver_state().pending).insert(
                session_id.clone(),
                PendingConsent {
                    fingerprint: req.info.fingerprint.clone(),
                    decision: tx,
                },
            );
            let payload = IncomingTransfer {
                session_id: session_id.clone(),
                alias: req.info.alias.clone(),
                fingerprint: req.info.fingerprint.clone(),
                device_type: req.info.device_type.clone(),
                files: incoming.clone(),
                total_bytes,
                expires_in_seconds: CONSENT_TIMEOUT.as_secs(),
            };
            if let Err(e) = app.emit(EVENT_INCOMING, &payload) {
                lock_or_recover(&receiver_state().pending).remove(&session_id);
                log::warn!("[localsend] could not ask for consent: {e}");
                return Err(500);
            }
            log::info!(
                "[localsend] waiting for consent: {} file(s) ({} bytes) from '{}'",
                incoming.len(),
                total_bytes,
                req.info.alias
            );
            let accepted = matches!(
                tokio::time::timeout(CONSENT_TIMEOUT, rx).await,
                Ok(Ok(true))
            );
            // The responder removes the entry; remove again in case of timeout.
            lock_or_recover(&receiver_state().pending).remove(&session_id);
            if !accepted {
                log::info!(
                    "[localsend] transfer from '{}' declined or timed out",
                    req.info.alias
                );
                return Err(403);
            }
            log::info!(
                "[localsend] transfer from '{}' accepted by the user",
                req.info.alias
            );
        }
    }

    let mut files = HashMap::new();
    let mut granted = serde_json::Map::new();
    for (file_id, f) in req.files {
        let token = random_hex();
        files.insert(
            file_id.clone(),
            (token.clone(), f.file_name, f.size, f.sha256),
        );
        granted.insert(file_id, serde_json::Value::String(token));
    }
    lock_or_recover(&receiver_state().sessions)
        .insert(session_id.clone(), ReceiverSession { files });
    let resp = serde_json::json!({
        "sessionId": session_id,
        "files": serde_json::Value::Object(granted),
    });
    serde_json::to_vec(&resp).map_err(|_| 500)
}

/// Bounded, streaming destination for one upload.
///
/// The body is written to a `.part` file as it arrives and hashed on the fly,
/// so a 10 GB upload costs one 64 KiB chunk of RAM instead of 10 GB. `limit` is
/// enforced on every write and `expected` (when the sender announced a size)
/// turns a short or long body into an error instead of a truncated file.
struct UploadSink {
    file: tokio::fs::File,
    hasher: Sha256,
    written: u64,
    expected: Option<u64>,
    limit: u64,
    path: PathBuf,
}

impl UploadSink {
    async fn create(path: &Path, expected: Option<u64>, limit: u64) -> Result<Self, u16> {
        let file = tokio::fs::File::create(path).await.map_err(|e| {
            log::warn!("[localsend] cannot create '{}': {e}", path.display());
            500u16
        })?;
        Ok(Self {
            file,
            hasher: Sha256::new(),
            written: 0,
            expected,
            limit,
            path: path.to_path_buf(),
        })
    }

    /// True once the announced number of bytes has arrived.
    fn is_complete(&self) -> bool {
        self.expected.is_some_and(|e| self.written >= e)
    }

    async fn write(&mut self, chunk: &[u8]) -> Result<(), u16> {
        if chunk.is_empty() {
            return Ok(());
        }
        let next = self.written.saturating_add(chunk.len() as u64);
        if next > self.limit {
            log::warn!(
                "[localsend] upload exceeds the {} MiB limit — aborting",
                self.limit / (1024 * 1024)
            );
            return Err(413);
        }
        if let Some(expected) = self.expected {
            if next > expected {
                log::warn!("[localsend] sender sent more than the announced {expected} bytes");
                return Err(400);
            }
        }
        self.file.write_all(chunk).await.map_err(|e| {
            log::warn!("[localsend] write failed: {e}");
            500u16
        })?;
        self.hasher.update(chunk);
        self.written = next;
        Ok(())
    }

    /// Flush, verify size and checksum, and close the file (Windows will not
    /// rename a file that is still open).
    async fn finish(mut self, expected_sha: Option<&str>) -> Result<(), u16> {
        if let Some(expected) = self.expected {
            if self.written != expected {
                log::warn!(
                    "[localsend] short upload: {} of {expected} bytes",
                    self.written
                );
                self.abort().await;
                return Err(400);
            }
        }
        if let Err(e) = self.file.flush().await {
            log::warn!("[localsend] flush failed: {e}");
            self.abort().await;
            return Err(500);
        }
        if let Some(expected) = expected_sha {
            let hasher = std::mem::replace(&mut self.hasher, Sha256::new());
            let got: String = hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            if !got.eq_ignore_ascii_case(expected) {
                log::warn!("[localsend] checksum mismatch (announced {expected}, got {got})");
                self.abort().await;
                return Err(422);
            }
        }
        drop(self.file);
        Ok(())
    }

    /// Remove the partial file.
    async fn abort(self) {
        drop(self.file);
        let _ = tokio::fs::remove_file(&self.path).await;
    }
}

/// Handle an upload: validate the token, stream the body to Downloads with
/// SHA-256 verification (422 on mismatch). Returns the HTTP status code.
async fn handle_upload(
    stream: &mut tokio::net::TcpStream,
    head: &[u8],
    head_len: usize,
    content_length: Option<u64>,
    query: &HashMap<String, String>,
) -> u16 {
    let (sid, fid, token) = match (
        query.get("sessionId"),
        query.get("fileId"),
        query.get("token"),
    ) {
        (Some(s), Some(f), Some(t)) => (s.clone(), f.clone(), t.clone()),
        _ => return 400,
    };
    let (name, expected_size, sha) = {
        let sessions = lock_or_recover(&receiver_state().sessions);
        let Some(session) = sessions.get(&sid) else {
            return 403;
        };
        let Some((tok, name, size, sha)) = session.files.get(&fid) else {
            return 403;
        };
        if *tok != token {
            return 403;
        }
        (name.clone(), *size, sha.clone())
    };

    let limit = LocalSendConfig::load().max_upload_bytes();
    if expected_size > limit {
        return 413;
    }
    if let Some(cl) = content_length {
        if cl > limit {
            return 413;
        }
        // A sender that announces one size and sends another is not something
        // to guess about.
        if expected_size > 0 && cl != expected_size {
            return 400;
        }
    }
    let expected = if expected_size > 0 {
        Some(expected_size)
    } else {
        content_length
    };
    // Without either an announced size or a Content-Length there is no way to
    // know where the body ends.
    if expected.is_none() {
        return 411;
    }

    let safe = sanitize_file_name(&name);
    let dir = downloads_dir();
    let dest = unique_path(&dir, &safe);
    // Write to a sibling `.part` file and rename on success, so a failed or
    // rejected transfer never leaves a half file under the real name.
    let part = dir.join(format!(".{}.{}.part", safe, random_hex()));

    let mut sink = match UploadSink::create(&part, expected, limit).await {
        Ok(sink) => sink,
        Err(code) => return code,
    };
    // Bytes already read past the headers belong to the body.
    if let Err(code) = sink.write(&head[head_len..]).await {
        sink.abort().await;
        return code;
    }

    let mut chunk = vec![0u8; UPLOAD_CHUNK];
    while !sink.is_complete() {
        let n = match stream.read(&mut chunk).await {
            Ok(n) => n,
            Err(e) => {
                log::warn!("[localsend] read failed mid-upload: {e}");
                sink.abort().await;
                return 400;
            }
        };
        if n == 0 {
            break;
        }
        if let Err(code) = sink.write(&chunk[..n]).await {
            sink.abort().await;
            return code;
        }
    }

    if let Err(code) = sink.finish(sha.as_deref()).await {
        return code;
    }
    // Another transfer may have taken the name while this one streamed.
    let dest = if dest.exists() {
        unique_path(&dir, &safe)
    } else {
        dest
    };
    if let Err(e) = tokio::fs::rename(&part, &dest).await {
        log::warn!(
            "[localsend] could not move '{}' into place: {e}",
            dest.display()
        );
        let _ = tokio::fs::remove_file(&part).await;
        return 500;
    }
    receiver_state()
        .received_count
        .fetch_add(1, Ordering::Relaxed);
    log::info!("[localsend] received '{}' -> {}", safe, dest.display());
    200
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn info_json_roundtrip() {
        let info = our_info(true);
        let json = serde_json::to_string(&info).unwrap();
        let back: LocalSendInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, back);
        assert!(json.contains("\"announce\":true"));
        assert!(json.contains("\"protocol\":\"http\""));
    }

    #[test]
    fn parse_info_payload_accepts_peer() {
        let peer_info = LocalSendInfo {
            alias: "Nice Orange".into(),
            version: "2.0".into(),
            device_model: Some("Samsung".into()),
            device_type: Some("mobile".into()),
            fingerprint: random_hex(),
            port: 53317,
            protocol: "http".into(),
            download: Some(true),
            announce: Some(true),
        };
        let payload = serde_json::to_vec(&peer_info).unwrap();
        let parsed = parse_info_payload(&payload).expect("should parse");
        assert_eq!(parsed.alias, "Nice Orange");
        assert_eq!(parsed.protocol, "http");
        assert!(parsed.announce.unwrap_or(false));
    }

    #[test]
    fn parse_info_payload_ignores_self() {
        let mut info = our_info(true);
        info.announce = Some(true);
        let payload = serde_json::to_vec(&info).unwrap();
        assert!(parse_info_payload(&payload).is_none());
    }

    #[test]
    fn sanitize_file_name_strips_path() {
        assert_eq!(sanitize_file_name("..\\..\\evil.txt"), "evil.txt");
        assert_eq!(sanitize_file_name("/etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name("a:b*c?d\"e<f>g|h"), "abcdefgh");
        assert_eq!(sanitize_file_name("   "), "received_file");
        assert_eq!(sanitize_file_name(".."), "received_file");
    }

    /// A 400-character name would blow past the Windows path limit once the
    /// `.part` suffix and the " (n)" collision suffix are added.
    #[test]
    fn sanitize_file_name_bounds_the_length() {
        let long = "a".repeat(400);
        assert_eq!(sanitize_file_name(&long).chars().count(), 180);
    }

    #[test]
    fn unique_path_avoids_collision() {
        let dir = std::env::temp_dir().join(format!("ls-test-{}", random_hex()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = unique_path(&dir, "report.pdf");
        assert_eq!(first, dir.join("report.pdf"));
        std::fs::write(&first, b"x").unwrap();
        let second = unique_path(&dir, "report.pdf");
        assert_eq!(second, dir.join("report (1).pdf"));
        std::fs::write(&second, b"x").unwrap();
        let third = unique_path(&dir, "report.pdf");
        assert_eq!(third, dir.join("report (2).pdf"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn query_parser_splits_pairs() {
        let q = parse_query("/x?sessionId=a&fileId=b&token=c");
        assert_eq!(q.get("sessionId").map(String::as_str), Some("a"));
        assert_eq!(q.get("fileId").map(String::as_str), Some("b"));
        assert_eq!(q.get("token").map(String::as_str), Some("c"));
    }

    #[test]
    fn peer_base_url_handles_v6() {
        let p = LocalSendPeer {
            alias: "x".into(),
            fingerprint: "fp".into(),
            protocol: "https".into(),
            port: 53317,
            addr: "::1".parse().unwrap(),
            device_type: None,
            download: false,
        };
        assert_eq!(peer_base_url(&p), "https://[::1]:53317");
    }

    #[test]
    fn guess_mime_covers_common_types() {
        assert_eq!(guess_mime("photo.png"), "image/png");
        assert_eq!(guess_mime("sound.mp3"), "audio/mpeg");
        assert_eq!(guess_mime("archive.zip"), "application/zip");
        assert_eq!(guess_mime("blob.xyz"), "application/octet-stream");
    }

    #[tokio::test]
    async fn discover_returns_empty_without_network() {
        // Bounded sanity check: should complete within duration without peers.
        let peers = discover_peers(Duration::from_millis(150)).await;
        assert!(peers.len() < 5); // never our own self
        assert!(!peers.iter().any(|p| p.fingerprint == fingerprint()));
    }

    // ── Consent / limits ────────────────────────────────────────────────────

    /// The whole point of the fix: a fresh install must ask, not accept.
    #[test]
    fn config_defaults_require_consent() {
        let cfg = LocalSendConfig::default();
        assert!(
            cfg.auto_accept_fingerprints.is_empty(),
            "no device may be auto-accepted by default"
        );
        assert_eq!(cfg.max_upload_mb, DEFAULT_MAX_UPLOAD_MB);
        assert!(!cfg.is_auto_accepted("ABCD"));
    }

    #[test]
    fn consent_requires_a_ui_and_never_defaults_to_accepting() {
        assert_eq!(consent_action(true, true), ConsentAction::AutoAccept);
        // Opted-in device still auto-accepts if the UI is gone (user's choice).
        assert_eq!(consent_action(true, false), ConsentAction::AutoAccept);
        assert_eq!(consent_action(false, true), ConsentAction::Prompt);
        // No UI to ask = decline, never a silent write.
        assert_eq!(consent_action(false, false), ConsentAction::Deny);
    }

    #[test]
    fn auto_accept_list_parses_and_dedupes() {
        assert_eq!(
            parse_auto_accept("aa11, bb22;  AA11 ,, c:d\n"),
            vec!["aa11".to_string(), "bb22".to_string(), "c:d".to_string()]
        );
        assert!(parse_auto_accept("").is_empty());
        assert!(parse_auto_accept(" , ; ").is_empty());
        // A hostile/oversized value cannot grow the list without bound.
        let many = (0..100)
            .map(|i| format!("fp{i}"))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(parse_auto_accept(&many).len(), MAX_AUTO_ACCEPT);
    }

    #[test]
    fn auto_accept_add_and_forget_are_case_insensitive() {
        let mut cfg = LocalSendConfig::default();
        assert!(cfg.allow_device("AA:BB"));
        assert!(cfg.is_auto_accepted("aa:bb"));
        assert!(!cfg.allow_device("aa:bb"), "duplicate must not add twice");
        assert!(!cfg.allow_device("   "), "empty fingerprint is ignored");
        assert!(cfg.forget_device("aA:Bb"));
        assert!(!cfg.is_auto_accepted("AA:BB"));
        assert!(!cfg.forget_device("AA:BB"));
    }

    #[test]
    fn responding_to_an_unknown_session_reports_expired() {
        assert!(!respond_consent("does-not-exist", true));
    }

    #[test]
    fn describe_transfer_enforces_the_limits_and_sanitizes_names() {
        let req = prepare_req(&[("..\\..\\evil.txt", 10)]);
        let (files, total) = describe_transfer(&req, 100).expect("within limits");
        assert_eq!(total, 10);
        assert_eq!(files[0].name, "evil.txt");

        assert_eq!(
            describe_transfer(&prepare_req(&[]), 100).expect_err("empty transfer"),
            400
        );
        assert_eq!(
            describe_transfer(&prepare_req(&[("big", 101)]), 100).expect_err("over the cap"),
            413
        );
        // Two files that fit individually but not together are refused too.
        assert_eq!(
            describe_transfer(&prepare_req(&[("a", 60), ("b", 60)]), 100)
                .expect_err("over the cap in total"),
            413
        );
    }

    // ── Streaming upload ────────────────────────────────────────────────────

    /// `parse_http_head` used to report `head_end + 2`, so the first two body
    /// bytes were skipped and every size/checksum check failed.
    #[test]
    fn parse_http_head_points_at_the_first_body_byte() {
        let raw =
            b"POST /api/localsend/v2/upload HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\nHELLO";
        let (method, target, headers, head_len) = parse_http_head(raw).expect("valid head");
        assert_eq!(method, "POST");
        assert_eq!(target, "/api/localsend/v2/upload");
        assert_eq!(headers.get("content-length").map(String::as_str), Some("5"));
        assert_eq!(&raw[head_len..], b"HELLO");
    }

    fn prepare_req(files: &[(&str, u64)]) -> PrepareUploadRequest {
        let mut map = HashMap::new();
        for (i, (name, size)) in files.iter().enumerate() {
            map.insert(
                format!("f{i}"),
                PrepareFile {
                    file_name: (*name).to_string(),
                    size: *size,
                    sha256: None,
                },
            );
        }
        PrepareUploadRequest {
            info: our_info(false),
            files: map,
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ls-{tag}-{}", random_hex()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn upload_sink_streams_hashes_and_writes_the_body() {
        let dir = temp_dir("sink");
        let path = dir.join("a.bin");
        // Larger than one streaming chunk, so the multi-chunk path is used.
        let payload: Vec<u8> = (0..200 * 1024).map(|i| (i % 251) as u8).collect();
        let sha = sha256_hex(&payload);

        let mut sink = UploadSink::create(&path, Some(payload.len() as u64), 1024 * 1024)
            .await
            .expect("sink");
        for chunk in payload.chunks(UPLOAD_CHUNK) {
            sink.write(chunk).await.expect("write");
        }
        assert!(sink.is_complete());
        sink.finish(Some(&sha)).await.expect("finish");

        assert_eq!(std::fs::read(&path).unwrap(), payload);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn upload_sink_refuses_more_than_the_limit_and_leaves_no_partial_file() {
        let dir = temp_dir("limit");
        let path = dir.join("big.bin");
        let mut sink = UploadSink::create(&path, None, 1024).await.expect("sink");
        assert_eq!(
            sink.write(&[0u8; 2048]).await.expect_err("over the cap"),
            413
        );
        sink.abort().await;
        assert!(!path.exists(), "a refused upload must not leave a file");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn upload_sink_rejects_a_short_or_extra_long_body() {
        let dir = temp_dir("short");
        let path = dir.join("short.bin");
        let mut sink = UploadSink::create(&path, Some(100), 1024)
            .await
            .expect("sink");
        sink.write(&[1u8; 50]).await.expect("write");
        assert_eq!(sink.finish(None).await.expect_err("short body"), 400);
        assert!(!path.exists(), "a short upload must not be kept");

        // More bytes than announced is refused at the write, not after.
        let path2 = dir.join("long.bin");
        let mut sink = UploadSink::create(&path2, Some(10), 1024)
            .await
            .expect("sink");
        assert_eq!(sink.write(&[1u8; 11]).await.expect_err("long body"), 400);
        sink.abort().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn upload_sink_deletes_the_file_on_checksum_mismatch() {
        let dir = temp_dir("sha");
        let path = dir.join("bad.bin");
        let mut sink = UploadSink::create(&path, Some(4), 1024)
            .await
            .expect("sink");
        sink.write(b"data").await.expect("write");
        assert_eq!(
            sink.finish(Some(&"0".repeat(64)))
                .await
                .expect_err("checksum mismatch"),
            422
        );
        assert!(!path.exists(), "a corrupt upload must not be kept");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
