//! NFC pairing guidance + NDEF handshake payloads (MIOT-11).
//!
//! Windows/MiControl has no direct NFC controller on most laptops; Xiaomi
//! phones (e.g. the Xiaomi 14T) pair companions like **Phone Link** over the
//! network. This module:
//!
//! - Builds an **NDEF message** ([`NdefRecord`] plain-text record) carrying a
//!   pairing handshake payload — the payload is a compact JSON tag
//!   (URI + display name + fingerprint) that a phone app could read to auto
//!   launch the pairing flow.
//! - Hardcodes no secrets: `build_handshake` accepts explicit values, so the
//!   frontend can pass the current BLE/Link identity.
//! - Provides guidance (JSON status) and a deep-link to Microsoft Phone Link
//!   (`ms-phone-link:` scheme) when available — see [`NfcGuidance`].
//!
//! No OS NFC API is touched — all logic is pure encode/decode, so it cannot
//! interfere with other modules. The NDEF format here implements the classic
//! TNF_WELL_KNOWN / RTD_TEXT encoding (ISO/IEC 18092 NFC Data Exchange
//! Format): header byte + language length + language code + UTF-8 payload.

use serde::{Deserialize, Serialize};

/// NDEF well-known type name formats (TNF).
const TNF_WELL_KNOWN: u8 = 0x01;
/// RTD_TEXT "T" — free-form text.
const RTD_TEXT: &[u8] = b"T";
/// RTD_URI "U" — URI record.
const RTD_URI: &[u8] = b"U";
/// Language code used in encoded text records.
pub const DEFAULT_LANG: &str = "en";

/// A single NDEF record (encode only — enough for the handshake payload).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NdefRecord {
    pub tnf: u8,
    pub r#type: Vec<u8>,
    pub payload: Vec<u8>,
}

impl NdefRecord {
    /// Build an RTD_TEXT record for `text`.
    pub fn text(text: &str) -> Self {
        let mut payload = Vec::with_capacity(2 + DEFAULT_LANG.len() + text.len());
        payload.push((DEFAULT_LANG.len() as u8) & 0x3F); // status byte: lang len, no encoding flag
        payload.extend_from_slice(DEFAULT_LANG.as_bytes());
        payload.extend_from_slice(text.as_bytes());
        Self {
            tnf: TNF_WELL_KNOWN,
            r#type: RTD_TEXT.to_vec(),
            payload,
        }
    }

    /// Build an RTD_URI record.
    pub fn uri(uri: &str) -> Self {
        let mut payload = Vec::with_capacity(1 + uri.len());
        payload.push(0x00); // no URI prefix
        payload.extend_from_slice(uri.as_bytes());
        Self {
            tnf: TNF_WELL_KNOWN,
            r#type: RTD_URI.to_vec(),
            payload,
        }
    }

    /// Serialize this record as an NDEF field (header + type + payload).
    pub fn encode(&self) -> Vec<u8> {
        // Single-record message: MB+ME set, no ID.
        let mut out = Vec::new();
        let header = 0x80 | 0x40 | (self.tnf & 0x07); // MB|ME
        out.push(header);
        out.push(self.r#type.len() as u8);
        let payload_len = self.payload.len() as u16;
        if payload_len > 255 {
            out.push(0xFF);
            out.extend_from_slice(&payload_len.to_be_bytes());
        } else {
            out.push(payload_len as u8);
        }
        out.extend_from_slice(&self.r#type);
        out.extend_from_slice(&self.payload);
        out
    }

    /// Decode a *single-record* NDEF message (the inverse of [`encode`]).
    /// Returns `None` for multi-record, short/long-mismatched, or malformed
    /// input. Mirrors the encoding above so round-trip tests stay symmetric.
    pub fn decode_single(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 3 {
            return None;
        }
        let header = bytes[0];
        let tnf = header & 0x07;
        let is_mb = header & 0x80 != 0;
        let is_me = header & 0x40 != 0;
        if !(is_mb && is_me) {
            return None; // multi-record unsupported here
        }
        let type_len = bytes[1] as usize;
        // Short form: [header | type_len | payload_len | type | payload]
        // Long  form: [header | type_len | 0xFF | len(2) | type | payload]
        let short = bytes[2] != 0xFF;
        let (payload_len, type_start): (usize, usize) = if short {
            (bytes[2] as usize, 3usize)
        } else {
            if bytes.len() < 5 {
                return None;
            }
            let len = u16::from_be_bytes([bytes[3], bytes[4]]) as usize;
            (len, 5usize)
        };
        let type_end = type_start.checked_add(type_len)?;
        if type_end > bytes.len() {
            return None;
        }
        let r#type = bytes[type_start..type_end].to_vec();
        let payload_start = type_end;
        let end = payload_start.checked_add(payload_len)?;
        if end != bytes.len() {
            return None;
        }
        let payload = bytes[payload_start..end].to_vec();
        Some(Self {
            tnf,
            r#type,
            payload,
        })
    }
}

/// Pairing handshake payload written to an NDEF text tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NfcHandshake {
    /// Protocol/version tag, e.g. `micontrol-pair/v1`.
    pub schema: String,
    /// The URI to deep-link on the phone (e.g. Phone Link launcher).
    pub pair_uri: String,
    /// Human-readable PC name.
    pub display_name: String,
    /// Optional device fingerprint (matches KDE/LocalSend identity).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

impl NfcHandshake {
    /// Encode the handshake to a portable JSON string.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| format!("handshake encode: {e}"))
    }
}

/// Build the NDEF text record carrying `handshake` (RFC-friendly JSON text).
pub fn build_handshake_record(handshake: &NfcHandshake) -> Result<NdefRecord, String> {
    Ok(NdefRecord::text(&handshake.to_json()?))
}

/// Status/guidance surfaced to the UI (pure data — no OS calls).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NfcGuidance {
    /// Whether a phone-link compatible deep link is offered.
    pub link_available: bool,
    /// The Phone Link deep-link URI (empty when unavailable).
    pub link_uri: String,
    /// Plain-language pairing instructions (English).
    pub instructions: String,
    /// The NDEF text record bytes (base64) the frontend can show as QR/text.
    pub ndef_text: Option<String>,
}

/// Build pairing guidance for the current device.
///
/// `pair_uri` is optional — when provided the guidance includes a
/// `ms-phone-link:` deep link (Phone Link app on Windows 11).
pub fn guidance(
    display_name: &str,
    fingerprint: Option<&str>,
    pair_uri: Option<&str>,
) -> NfcGuidance {
    let handshake = NfcHandshake {
        schema: "micontrol-pair/v1".into(),
        pair_uri: pair_uri.unwrap_or("").to_string(),
        display_name: display_name.to_string(),
        fingerprint: fingerprint.map(|s| s.to_string()),
    };
    // Only emit an NDEF handshake when a pairing URI is actually offered —
    // an empty deep link is useless on a tag.
    let ndef_text = if handshake.pair_uri.is_empty() {
        None
    } else {
        build_handshake_record(&handshake)
            .map(|r| base64_encode(&r.encode()))
            .ok()
    };
    let link_uri = format!("ms-phone-link:pairing?pc={}", url_encode(display_name));
    NfcGuidance {
        link_available: pair_uri.is_some() && !pair_uri.unwrap_or_default().is_empty(),
        // Keep the well-known scheme regardless; UI decides when to show.
        link_uri,
        instructions: "Hold the back of your Xiaomi phone near the NFC antenna \
on the laptop (usually palm-rest or top cover) to read the pairing tag. \
If nothing happens, enable NFC in phone Settings → Connection & sharing \
→ NFC, then retry."
            .to_string(),
        ndef_text,
    }
}

/// Minimal base64 helper (avoid pulling the base64 crate just for one field —
/// keeps format consistent: no padding for short strings is fine for UI).
fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn url_encode(s: &str) -> String {
    // Minimal URL encoding for the PC name in the deep link.
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ndef_text_record_roundtrip() {
        let rec = NdefRecord::text("Hello MiControl");
        let encoded = rec.encode();
        let decoded = NdefRecord::decode_single(&encoded).expect("decode");
        assert_eq!(decoded, rec);
        assert_eq!(decoded.tnf, TNF_WELL_KNOWN);
        assert_eq!(decoded.r#type, RTD_TEXT);
        // Payload: status byte + lang + text.
        let text = String::from_utf8(decoded.payload[3..].to_vec()).unwrap();
        assert_eq!(text, "Hello MiControl");
    }

    #[test]
    fn ndef_uri_record_roundtrip() {
        let rec = NdefRecord::uri("https://example.com/pair");
        let decoded = NdefRecord::decode_single(&rec.encode()).unwrap();
        assert_eq!(decoded, rec);
    }

    #[test]
    fn ndef_long_payload_roundtrip() {
        // >255 byte payload exercises the long form.
        let text = "x".repeat(400);
        let rec = NdefRecord::text(&text);
        let decoded = NdefRecord::decode_single(&rec.encode()).unwrap();
        assert_eq!(decoded, rec);
    }

    #[test]
    fn ndef_rejects_malformed() {
        assert!(NdefRecord::decode_single(b"").is_none());
        assert!(NdefRecord::decode_single(b"\x80").is_none());
        // Header claims MB|ME but type len runs past buffer.
        let mut bad = vec![0xC1u8, 0x05, 0x00, b'T'];
        assert!(NdefRecord::decode_single(&bad).is_none());
        bad.truncate(2);
        assert!(NdefRecord::decode_single(&bad).is_none());
    }

    #[test]
    fn handshake_roundtrip() {
        let hs = NfcHandshake {
            schema: "micontrol-pair/v1".into(),
            pair_uri: "https://aka.ms/phone-link".into(),
            display_name: "MiBookPro14".into(),
            fingerprint: Some("aa:bb:cc:dd".into()),
        };
        let rec = build_handshake_record(&hs).unwrap();
        let decoded_text = String::from_utf8(rec.payload[3..].to_vec()).unwrap();
        let back: NfcHandshake = serde_json::from_str(&decoded_text).unwrap();
        assert_eq!(back, hs);
    }

    #[test]
    fn guidance_never_panics() {
        let g = guidance("MiBook Pro", Some("fp"), Some("https://aka.ms/phone-link"));
        assert!(g.link_available);
        assert!(g.link_uri.starts_with("ms-phone-link:"));
        assert!(g.ndef_text.is_some());
        assert!(!g.instructions.is_empty());

        let g2 = guidance("MiBook Pro", None, None);
        assert!(!g2.link_available);
        assert_eq!(g2.ndef_text, None);
    }
}
