//! XML escaping and validation utilities.
//!
//! Used by the WiFi profile builder to prevent XML injection via
//! attacker-controlled SSIDs or passwords.

/// Escape XML metacharacters in a string.
///
/// Escapes:
/// - `&` → `&amp;`
/// - `<` → `&lt;`
/// - `>` → `&gt;`
/// - `"` → `&quot;`
/// - `'` → `&apos;`
///
/// This MUST be applied to any user-supplied value before interpolation
/// into an XML template.
pub fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Validate a WiFi SSID per IEEE 802.11 rules.
///
/// Returns `Ok(())` if valid, or `Err(message)` if:
/// - The SSID is empty
/// - The SSID exceeds 32 bytes (UTF-8 encoded)
/// - The SSID contains null bytes
pub fn validate_ssid(ssid: &str) -> Result<(), String> {
    if ssid.is_empty() {
        return Err("SSID cannot be empty".to_string());
    }
    let byte_len = ssid.len();
    if byte_len > 32 {
        return Err(format!(
            "SSID exceeds 32 bytes (got {byte_len} bytes)"
        ));
    }
    if ssid.contains('\0') {
        return Err("SSID cannot contain null bytes".to_string());
    }
    Ok(())
}

/// Validate a WPA2 passphrase per IEEE 802.11 rules.
///
/// Returns `Ok(())` if valid, or `Err(message)` if:
/// - The password is shorter than 8 characters
/// - The password is longer than 63 characters
pub fn validate_wpa2_passphrase(password: &str) -> Result<(), String> {
    let len = password.len();
    if len < 8 {
        return Err(format!(
            "WPA2 passphrase must be at least 8 characters (got {len})"
        ));
    }
    if len > 63 {
        return Err(format!(
            "WPA2 passphrase must be at most 63 characters (got {len})"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_all_metacharacters() {
        assert_eq!(escape_xml("&"), "&amp;");
        assert_eq!(escape_xml("<"), "&lt;");
        assert_eq!(escape_xml(">"), "&gt;");
        assert_eq!(escape_xml("\""), "&quot;");
        assert_eq!(escape_xml("'"), "&apos;");
    }

    #[test]
    fn test_escape_normal_text_unchanged() {
        assert_eq!(escape_xml("Hello World"), "Hello World");
        assert_eq!(escape_xml("MyNetwork123"), "MyNetwork123");
    }

    #[test]
    fn test_escape_mixed_content() {
        let input = "test</name><name>evil";
        let escaped = escape_xml(input);
        assert!(!escaped.contains("</name>"));
        assert!(escaped.contains("&lt;/name&gt;"));
    }

    #[test]
    fn test_escape_password_with_keymaterial_breakout() {
        let input = "</keyMaterial><x>";
        let escaped = escape_xml(input);
        assert!(!escaped.contains("</keyMaterial>"));
        assert!(escaped.contains("&lt;/keyMaterial&gt;"));
    }

    #[test]
    fn test_escape_ampersand_first() {
        // & must be escaped first to avoid double-escaping
        assert_eq!(escape_xml("&lt;"), "&amp;lt;");
    }

    #[test]
    fn test_validate_ssid_valid() {
        assert!(validate_ssid("MyNetwork").is_ok());
        assert!(validate_ssid("a").is_ok()); // 1 byte is valid
        assert!(validate_ssid(&"a".repeat(32)).is_ok()); // exactly 32 bytes
    }

    #[test]
    fn test_validate_ssid_empty_rejected() {
        assert!(validate_ssid("").is_err());
    }

    #[test]
    fn test_validate_ssid_too_long_rejected() {
        let long_ssid = "a".repeat(33);
        assert!(validate_ssid(&long_ssid).is_err());
    }

    #[test]
    fn test_validate_ssid_null_byte_rejected() {
        assert!(validate_ssid("test\0ssid").is_err());
    }

    #[test]
    fn test_validate_ssid_multibyte_byte_count() {
        // 2-byte UTF-8 chars: 16 chars = 32 bytes (valid)
        let ssid = "é".repeat(16);
        assert!(validate_ssid(&ssid).is_ok());
        // 17 chars = 34 bytes (invalid)
        let too_long = "é".repeat(17);
        assert!(validate_ssid(&too_long).is_err());
    }

    #[test]
    fn test_validate_passphrase_valid() {
        assert!(validate_wpa2_passphrase("12345678").is_ok()); // exactly 8
        assert!(validate_wpa2_passphrase(&"a".repeat(63)).is_ok()); // exactly 63
    }

    #[test]
    fn test_validate_passphrase_too_short_rejected() {
        assert!(validate_wpa2_passphrase("1234567").is_err()); // 7 chars
    }

    #[test]
    fn test_validate_passphrase_too_long_rejected() {
        let long = "a".repeat(64);
        assert!(validate_wpa2_passphrase(&long).is_err());
    }

    #[test]
    fn test_injection_ssid_produces_well_formed_xml() {
        let malicious_ssid = "test</name><name>evil";
        let escaped = escape_xml(malicious_ssid);
        // The escaped SSID should not contain any raw XML tags
        assert!(!escaped.contains("</name>"));
        assert!(!escaped.contains("<name>"));
    }
}