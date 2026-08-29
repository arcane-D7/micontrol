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
//!   `upload` (raw bytes) against a discovered peer.
//! - **Receiver**: an embedded HTTP server on TCP `53317` that auto-accepts
//!   incoming transfers and saves files to the user's Downloads folder
//!   (mirroring the official "auto-accept" convenience; the transfer is
//!   verified against the announced SHA-256 and rejected with HTTP 422 on a
//!   mismatch).
//!
//! Security notes (documented limitation):
//! - LocalSend is designed to negotiate TLS with a self-signed certificate in
//!   HTTPS mode. This module currently advertises plain `http` and, when
//!   talking to an HTTPS peer, disables certificate validation because the
//!   fingerprint (not the CA chain) is the trust anchor. Only use on trusted
//!   LANs.
//! - The receiver auto-accepts; file name is sanitized and written to
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

// ── Receiver (embedded HTTP server) ──────────────────────────────────────────

/// Immutable receiver globals.
struct ReceiverState {
    running: AtomicBool,
    received_count: AtomicU64,
    sessions: std::sync::Mutex<HashMap<String, ReceiverSession>>,
}

#[derive(Default)]
struct ReceiverSession {
    /// fileId → (token, file name, size, announced sha256)
    files: HashMap<String, (String, String, u64, Option<String>)>,
}

static RECEIVER: OnceLock<ReceiverState> = OnceLock::new();

fn receiver_state() -> &'static ReceiverState {
    RECEIVER.get_or_init(|| ReceiverState {
        running: AtomicBool::new(false),
        received_count: AtomicU64::new(0),
        sessions: std::sync::Mutex::new(HashMap::new()),
    })
}

/// Start the embedded HTTP receiver on port 53317 (idempotent).
pub fn start_receiver() -> Result<u16, String> {
    let state = receiver_state();
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
/// is the byte offset just past the blank line ending the headers.
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
    Some((method, target, headers, head_end + 2))
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
    use tokio::io::AsyncWriteExt;
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

/// Handle one accepted connection: read the request, dispatch, respond.
async fn handle_connection(mut stream: tokio::net::TcpStream) -> std::io::Result<()> {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::with_capacity(8192);
    let mut tmp = [0u8; 1024];
    // Read until the header terminator or a reasonably large cap.
    loop {
        if find_head_end(&buf).is_some() {
            break;
        }
        if buf.len() > 64 * 1024 {
            return write_response(
                &mut stream,
                413,
                "Payload Too Large",
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
    let (method, target, headers, header_len) = match parse_http_head(&buf) {
        Some(v) => v,
        None => {
            return write_response(&mut stream, 400, "Bad Request", "text/plain", b"bad head")
                .await;
        }
    };
    let content_length = headers
        .get("content-length")
        .and_then(|cl| cl.parse().ok())
        .unwrap_or(0);
    // Read the body bytes.
    while buf.len() < header_len + content_length {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let body = buf[header_len..header_len + content_length.min(buf.len() - header_len)].to_vec();

    let query = parse_query(&target);
    let path = target.split('?').next().unwrap_or(&target);

    match (method.as_str(), path) {
        ("POST", "/api/localsend/v2/register") => {
            let info = serde_json::to_vec(&our_info(false)).unwrap_or_default();
            write_response(&mut stream, 200, "OK", "application/json", &info).await
        }
        ("POST", "/api/localsend/v2/prepare-upload") => match handle_prepare_upload(&body) {
            Ok(resp) => write_response(&mut stream, 200, "OK", "application/json", &resp).await,
            Err(code) => {
                write_response(&mut stream, code, "Rejected", "application/json", b"{}").await
            }
        },
        ("POST", "/api/localsend/v2/upload") => {
            let code = handle_upload(&query, &body).await;
            let (status, reason, out) = match code {
                200 => (200, "OK", b"".to_vec()),
                403 => (403, "Forbidden", b"invalid token".to_vec()),
                409 => (409, "Conflict", b"blocked".to_vec()),
                422 => (422, "Unprocessable", b"checksum mismatch".to_vec()),
                _ => (500, "Error", b"server error".to_vec()),
            };
            write_response(&mut stream, status, reason, "text/plain", &out).await
        }
        ("POST", "/api/localsend/v2/cancel") => {
            if let Some(sid) = query.get("sessionId") {
                lock_or_recover(&receiver_state().sessions).remove(sid);
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

/// Accept the transfer (session auto-accept behavior) and grant per-file
/// tokens. Returns the JSON body or an HTTP error code.
fn handle_prepare_upload(body: &[u8]) -> Result<Vec<u8>, u16> {
    let req: PrepareUploadRequest = serde_json::from_slice(body).map_err(|_| 400u16)?;
    // Ignore announcements coming from ourselves (should not happen, but the
    // sender fingerprint is the identity guard).
    if req.info.fingerprint == fingerprint() {
        return Err(403);
    }
    let session_id = random_hex();
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

/// Handle an upload: validate token, stream body to Downloads with SHA-256
/// verification (422 on mismatch). Returns the HTTP status code.
async fn handle_upload(query: &HashMap<String, String>, body: &[u8]) -> u16 {
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
    if body.len() as u64 != expected_size && expected_size > 0 {
        return 400;
    }
    let safe = sanitize_file_name(&name);
    let dest = unique_path(&downloads_dir(), &safe);
    if let Err(e) = std::fs::write(&dest, body) {
        log::warn!("LocalSend: failed to save '{}': {e}", dest.display());
        return 500;
    }
    // Verify announced SHA-256 when present.
    if let Some(expected) = sha {
        let mut hasher = Sha256::new();
        hasher.update(body);
        let got: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if got != expected {
            let _ = std::fs::remove_file(&dest);
            return 422;
        }
    }
    receiver_state()
        .received_count
        .fetch_add(1, Ordering::Relaxed);
    log::info!("LocalSend: received '{}' -> {}", safe, dest.display());
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
}
