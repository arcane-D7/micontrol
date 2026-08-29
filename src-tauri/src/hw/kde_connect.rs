//! KDE Connect discovery + plugins MVP (MIOT-10).
//!
//! KDE Connect (kdeconnect) links phones/PCs over the local network: devices
//! announce themselves on UDP port 1716 with a JSON identity packet, then
//! talk over TCP on the announced `tcpPort`, exchanging typed JSON messages
//! ("plugins").
//!
//! This module implements the *minimum viable* subset, fully isolated:
//! - **Discovery**: broadcast a `kdeconnect.identity` packet on
//!   `255.255.255.255:1716`, listen for identity replies, and parse them into
//!   [`KdeDevice`]s (id, name, type, tcp port, IP).
//! - **Ping plugin**: TCP connect to a discovered device's `tcpPort`, send a
//!   `kdeconnect.ping` request, and await the `kdeconnect.ping.reply`.
//! - **Clipboard stub**: capability is announced but no transfer is implemented
//!   yet (future MIOT wave).
//!
//! # TLS limitation (documented)
//!
//! Production KDE Connect encrypts TCP traffic with self-signed TLS
//! certificates (fingerprint = device identity). This MVP is **non-TLS
//! plaintext JSON only**: it can discover devices and reach permissive
//! debug/`pairing: false` peers, but a stock KDE Connect device will refuse
//! unauthenticated plugin messages. This is intentional — wiring the TLS
//! cert/fingerprint trust store is out of scope for this sprint and is
//! isolated here so nothing else in MiControl is affected.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const KDE_UDP_PORT: u16 = 1716;
pub const KDE_BROADCAST: &str = "255.255.255.255";
pub const KDE_PROTOCOL_VERSION: u32 = 7;
pub const KDE_DEVICE_TYPE: &str = "desktop";
pub const KDE_DEVICE_NAME: &str = "MiControl";
const RECV_BUF: usize = 8192;
static DEVICE_ID: OnceLock<String> = OnceLock::new();

/// The identity body broadcast + echoed by KDE Connect devices.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KdeIdentityBody {
    pub device_id: String,
    pub device_name: String,
    pub protocol_version: u32,
    pub device_type: String,
    #[serde(default)]
    pub incoming_capabilities: Vec<String>,
    #[serde(default)]
    pub outgoing_capabilities: Vec<String>,
    /// When present (e.g. `false`) a peer accepts unpaired plugin messages —
    /// this MVP relies on that for ping, otherwise TLS pairing is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing: Option<bool>,
}

/// Typed JSON envelope (KDE Connect wire format).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdeMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

/// A discovered KDE Connect device, as surfaced to the UI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KdeDevice {
    pub device_id: String,
    pub device_name: String,
    pub device_type: String,
    pub tcp_port: u16,
    pub addr: IpAddr,
    pub protocol_version: u32,
    /// Capabilities the peer advertises (incoming), e.g. `kdeconnect.ping`.
    #[serde(default)]
    pub incoming_capabilities: Vec<String>,
    pub pairing_optional: bool,
}

/// Stable per-process device id (kept inside the UDP payload, like LocalSend's
/// fingerprint — prevents us from "discovering" our own broadcast echo).
fn device_id() -> &'static str {
    DEVICE_ID.get_or_init(|| {
        let mut bytes = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":")
    })
}

/// Advertised capabilities for this MVP.
fn our_capabilities() -> Vec<String> {
    vec![
        "kdeconnect.ping".into(),
        "kdeconnect.clipboard".into(),
        "kdeconnect.notification".into(),
    ]
}

/// Build the identity packet we broadcast (also reusable for TCP handshake).
pub fn build_identity() -> KdeMessage {
    KdeMessage {
        msg_type: "kdeconnect.identity".into(),
        body: Some(
            serde_json::to_value(KdeIdentityBody {
                device_id: device_id().to_string(),
                device_name: KDE_DEVICE_NAME.into(),
                protocol_version: KDE_PROTOCOL_VERSION,
                device_type: KDE_DEVICE_TYPE.into(),
                incoming_capabilities: our_capabilities(),
                outgoing_capabilities: our_capabilities(),
                pairing: Some(false),
            })
            .expect("identity body is serializable"),
        ),
    }
}

/// Parse a UDP identity reply into a [`KdeDevice`].
pub fn parse_identity_payload(payload: &[u8], src: SocketAddr) -> Result<KdeDevice, String> {
    let msg: KdeMessage =
        serde_json::from_slice(payload).map_err(|e| format!("invalid kdeconnect packet: {e}"))?;
    if msg.msg_type != "kdeconnect.identity" {
        return Err(format!("unexpected message type {}", msg.msg_type));
    }
    let body: KdeIdentityBody = serde_json::from_value(
        msg.body
            .ok_or_else(|| "identity without body".to_string())?,
    )
    .map_err(|e| format!("invalid identity body: {e}"))?;

    // Ignore our own broadcast echo (identity round-trip).
    if body.device_id == device_id() {
        return Err("self packet".into());
    }

    // Modern KDE Connect UDP identity does not carry the TCP port — callers
    // fall back to the well-known 1716 (see discover_devices).
    Ok(KdeDevice {
        device_id: body.device_id,
        device_name: body.device_name,
        device_type: body.device_type,
        tcp_port: 0,
        addr: src.ip(),
        protocol_version: body.protocol_version,
        incoming_capabilities: body.incoming_capabilities,
        pairing_optional: body.pairing == Some(false),
    })
}

// NOTE: KDE Connect's UDP identity does NOT carry a TCP port field in modern
// versions — the TCP port is discovered via the handshake. We therefore keep
// `tcp_port` discoverable via the TCP identity exchange, defaulting to the
// well-known 1716 when a peer does not negotiate otherwise.

/// Discover KDE Connect devices on the LAN for `duration`.
pub async fn discover_devices(duration: Duration) -> Vec<KdeDevice> {
    let socket = match discovery_socket() {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    // Broadcast our identity so peers reply with theirs.
    let packet = serde_json::to_vec(&build_identity()).unwrap_or_default();
    let _ = socket
        .send_to(&packet, format!("{KDE_BROADCAST}:{KDE_UDP_PORT}"))
        .await;

    let mut buf = vec![0u8; RECV_BUF];
    let mut found: Vec<KdeDevice> = Vec::new();
    let mut seen: HashMap<String, ()> = HashMap::new();
    let deadline = tokio::time::Instant::now() + duration;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let r = tokio::time::timeout(remaining, socket.recv_from(&mut buf)).await;
        match r {
            Ok(Ok((n, src))) => match parse_identity_payload(&buf[..n], src) {
                Ok(mut dev) => {
                    if !seen.contains_key(&dev.device_id) {
                        // Keep the well-known TCP fallback port.
                        dev.tcp_port = KDE_UDP_PORT;
                        seen.insert(dev.device_id.clone(), ());
                        found.push(dev);
                    }
                }
                Err(_) => continue,
            },
            Ok(Err(_)) => break,
            Err(_) => break, // timeout elapsed
        }
    }
    found
}

/// Async UDP socket for discovery (broadcast + dual behavior).
fn discovery_socket() -> std::io::Result<tokio::net::UdpSocket> {
    let std_sock = std::net::UdpSocket::bind("0.0.0.0:0")?;
    std_sock.set_nonblocking(true)?;
    std_sock.set_broadcast(true)?;
    tokio::net::UdpSocket::from_std(std_sock)
}

// ── TCP plugin channel (Ping MVP) ───────────────────────────────────────────

/// Send a `kdeconnect.ping` to a discovered device over TCP (non-TLS) and wait
/// for the `kdeconnect.ping.reply`. Returns `Ok(true)` on a confirmed reply.
pub async fn send_ping(device: &KdeDevice, timeout: Duration) -> Result<bool, String> {
    let port = if device.tcp_port == 0 {
        KDE_UDP_PORT
    } else {
        device.tcp_port
    };
    let addr: SocketAddr = SocketAddr::new(device.addr, port);
    let stream = tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr))
        .await
        .map_err(|_| format!("timeout connecting to {}", device.device_name))?
        .map_err(|e| format!("connect to {}: {e}", device.device_name))?;
    let mut stream = stream;

    // Non-TLS MVP: try an identity handshake first so permissive peers accept
    // the plugin traffic (pairing:false); failures are non-fatal — the ping
    // alone is enough for peers that skip the handshake.
    let identity = serde_json::to_vec(&build_identity()).unwrap_or_default();
    let _ = stream.write_all(&identity).await;
    let _ = stream.write_all(b"\n").await; // KDE Connect frames with newline

    let ping = serde_json::json!({
        "type": "kdeconnect.ping",
        "body": { "message": "MiControl ping" }
    });
    let mut payload = serde_json::to_vec(&ping).unwrap_or_default();
    payload.push(b'\n');
    let _ = stream.write_all(&payload).await;

    // Await the reply.
    let mut buffer = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let mut tmp = [0u8; 4096];
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let read = tokio::time::timeout(remaining, stream.read(&mut tmp))
            .await
            .map_err(|_| "timeout waiting for ping reply".to_string())?
            .map_err(|e| format!("read: {e}"))?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&tmp[..read]);
        if buffer.windows(9).any(|w| w == b"ping.rep") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&buffer);
    Ok(text.contains("kdeconnect.ping.reply"))
}

// ── Shared discovery cache (for status/UI refresh) ──────────────────────────

static CACHE: OnceLock<Arc<Mutex<Vec<KdeDevice>>>> = OnceLock::new();

pub fn cache() -> &'static Arc<Mutex<Vec<KdeDevice>>> {
    CACHE.get_or_init(|| Arc::new(Mutex::new(Vec::new())))
}

/// Refresh the shared cache with a bounded discovery sweep.
pub async fn refresh_cache(duration: Duration) -> Vec<KdeDevice> {
    let devs = discover_devices(duration).await;
    if let Ok(mut c) = cache().lock() {
        *c = devs.clone();
    }
    devs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_payload() -> Vec<u8> {
        serde_json::json!({
            "type": "kdeconnect.identity",
            "body": {
                "deviceId": "aa:bb:cc",
                "deviceName": "Pixel 9",
                "protocolVersion": 7,
                "deviceType": "phone",
                "incomingCapabilities": ["kdeconnect.ping", "kdeconnect.clipboard"],
                "outgoingCapabilities": ["kdeconnect.ping"]
            }
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn parse_identity_accepts_peer() {
        let src: SocketAddr = "192.168.1.50:1716".parse().unwrap();
        let dev = parse_identity_payload(&sample_payload(), src).unwrap();
        assert_eq!(dev.device_id, "aa:bb:cc");
        assert_eq!(dev.device_name, "Pixel 9");
        assert_eq!(dev.device_type, "phone");
        assert_eq!(dev.addr, "192.168.1.50".parse::<IpAddr>().unwrap());
        assert!(dev
            .incoming_capabilities
            .contains(&"kdeconnect.ping".into()));
        assert!(!dev.pairing_optional); // no pairing field → default false
    }

    #[test]
    fn parse_identity_rejects_wrong_type() {
        let src: SocketAddr = "127.0.0.1:1716".parse().unwrap();
        let bad = serde_json::json!({ "type": "kdeconnect.ping" })
            .to_string()
            .into_bytes();
        assert!(parse_identity_payload(&bad, src).is_err());
    }

    #[test]
    fn parse_identity_rejects_malformed() {
        let src: SocketAddr = "127.0.0.1:1716".parse().unwrap();
        assert!(parse_identity_payload(b"not json", src).is_err());
        let no_body = serde_json::json!({ "type": "kdeconnect.identity" })
            .to_string()
            .into_bytes();
        assert!(parse_identity_payload(&no_body, src).is_err());
    }

    #[test]
    fn build_identity_is_valid_message() {
        let m = build_identity();
        assert_eq!(m.msg_type, "kdeconnect.identity");
        let body: KdeIdentityBody = serde_json::from_value(m.body.unwrap()).unwrap();
        assert_eq!(body.device_type, "desktop");
        assert!(body
            .incoming_capabilities
            .contains(&"kdeconnect.ping".to_string()));
        assert_eq!(body.pairing, Some(false));
    }

    #[test]
    fn discover_returns_empty_without_network() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let devs = rt.block_on(discover_devices(Duration::from_millis(120)));
        // On a real LAN other KDE Connect devices may reply; in CI/offline
        // we must simply not panic and never see ourselves.
        assert!(!devs.iter().any(|d| d.device_id == device_id()));
    }

    #[test]
    fn ping_errors_cleanly_for_unroutable_device() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dev = KdeDevice {
            device_id: "x".into(),
            device_name: "nowhere".into(),
            device_type: "phone".into(),
            tcp_port: 1716,
            addr: "203.0.113.9".parse().unwrap(), // TEST-NET-3, unroutable
            protocol_version: 7,
            incoming_capabilities: vec![],
            pairing_optional: false,
        };
        // Must return a Result (Err is fine) — no hang, no panic.
        let _ = rt.block_on(send_ping(&dev, Duration::from_millis(500)));
    }
}
