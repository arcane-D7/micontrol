//! Shared WMI field extraction utilities.
//!
//! Provides typed accessors for extracting values from
//! `HashMap<String, wmi::Variant>` query results, replacing repetitive
//! `match map.get(key) { Some(Variant::...) }` blocks across the hardware layer.

use std::collections::HashMap;

/// Extract a u32 from a WMI variant map.
pub fn extract_u32(map: &HashMap<String, wmi::Variant>, key: &str) -> Option<u32> {
    match map.get(key) {
        Some(wmi::Variant::UI1(v)) => Some(*v as u32),
        Some(wmi::Variant::UI2(v)) => Some(*v as u32),
        Some(wmi::Variant::UI4(v)) => Some(*v),
        Some(wmi::Variant::UI8(v)) => Some(*v as u32),
        Some(wmi::Variant::I1(v)) => Some(*v as u32),
        Some(wmi::Variant::I2(v)) => Some(*v as u32),
        Some(wmi::Variant::I4(v)) => Some(*v as u32),
        Some(wmi::Variant::I8(v)) => Some(*v as u32),
        Some(wmi::Variant::Bool(v)) => Some(*v as u32),
        _ => None,
    }
}

/// Extract an i32 from a WMI variant map.
///
/// Note: `UI4` values are cast via `as i32`, which preserves the bit pattern.
/// Values > `i32::MAX` will wrap to negative. For unsigned access, use
/// [`extract_u32`] instead.
pub fn extract_i32(map: &HashMap<String, wmi::Variant>, key: &str) -> Option<i32> {
    match map.get(key) {
        Some(wmi::Variant::I1(v)) => Some(*v as i32),
        Some(wmi::Variant::I2(v)) => Some(*v as i32),
        Some(wmi::Variant::I4(v)) => Some(*v),
        Some(wmi::Variant::I8(v)) => Some(*v as i32),
        Some(wmi::Variant::UI1(v)) => Some(*v as i32),
        Some(wmi::Variant::UI2(v)) => Some(*v as i32),
        // UI4 → i32: bit-pattern-preserving cast. Use extract_u32 for unsigned access.
        Some(wmi::Variant::UI4(v)) => Some(*v as i32),
        _ => None,
    }
}

/// Extract a u64 from a WMI variant map.
pub fn extract_u64(map: &HashMap<String, wmi::Variant>, key: &str) -> Option<u64> {
    match map.get(key) {
        Some(wmi::Variant::UI8(v)) => Some(*v),
        Some(wmi::Variant::UI4(v)) => Some(*v as u64),
        Some(wmi::Variant::UI2(v)) => Some(*v as u64),
        Some(wmi::Variant::UI1(v)) => Some(*v as u64),
        Some(wmi::Variant::I4(v)) => Some(*v as u64),
        Some(wmi::Variant::I8(v)) => Some(*v as u64),
        _ => None,
    }
}

/// Extract a String from a WMI variant map.
pub fn extract_string(map: &HashMap<String, wmi::Variant>, key: &str) -> Option<String> {
    match map.get(key) {
        Some(wmi::Variant::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Extract a bool from a WMI variant map.
pub fn extract_bool(map: &HashMap<String, wmi::Variant>, key: &str) -> Option<bool> {
    match map.get(key) {
        Some(wmi::Variant::Bool(v)) => Some(*v),
        _ => None,
    }
}

/// Extract a u32 or return a default value.
pub fn extract_u32_or(map: &HashMap<String, wmi::Variant>, key: &str, default: u32) -> u32 {
    extract_u32(map, key).unwrap_or(default)
}

/// Extract a String or return a default value.
pub fn extract_string_or(map: &HashMap<String, wmi::Variant>, key: &str, default: &str) -> String {
    extract_string(map, key).unwrap_or_else(|| default.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> HashMap<String, wmi::Variant> {
        HashMap::new()
    }

    // ── extract_u32 ──────────────────────────────────────────────────────────

    #[test]
    fn extract_u32_from_ui4() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::UI4(42));
        assert_eq!(extract_u32(&m, "v"), Some(42));
    }

    #[test]
    fn extract_u32_from_ui1() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::UI1(255));
        assert_eq!(extract_u32(&m, "v"), Some(255));
    }

    #[test]
    fn extract_u32_from_ui2() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::UI2(65535));
        assert_eq!(extract_u32(&m, "v"), Some(65535));
    }

    #[test]
    fn extract_u32_from_ui8() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::UI8(100_000));
        assert_eq!(extract_u32(&m, "v"), Some(100_000));
    }

    #[test]
    fn extract_u32_from_i4() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::I4(-7));
        assert_eq!(extract_u32(&m, "v"), Some((-7i32) as u32));
    }

    #[test]
    fn extract_u32_from_bool() {
        let mut m = map();
        m.insert("t".to_string(), wmi::Variant::Bool(true));
        m.insert("f".to_string(), wmi::Variant::Bool(false));
        assert_eq!(extract_u32(&m, "t"), Some(1));
        assert_eq!(extract_u32(&m, "f"), Some(0));
    }

    #[test]
    fn extract_u32_missing_key() {
        let m = map();
        assert_eq!(extract_u32(&m, "nope"), None);
    }

    #[test]
    fn extract_u32_wrong_type() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::String("42".to_string()));
        assert_eq!(extract_u32(&m, "v"), None);
    }

    // ── extract_i32 ──────────────────────────────────────────────────────────

    #[test]
    fn extract_i32_from_i4() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::I4(-100));
        assert_eq!(extract_i32(&m, "v"), Some(-100));
    }

    #[test]
    fn extract_i32_from_ui4() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::UI4(7));
        assert_eq!(extract_i32(&m, "v"), Some(7));
    }

    #[test]
    fn extract_i32_from_i2() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::I2(-1));
        assert_eq!(extract_i32(&m, "v"), Some(-1));
    }

    #[test]
    fn extract_i32_missing_key() {
        let m = map();
        assert_eq!(extract_i32(&m, "x"), None);
    }

    #[test]
    fn extract_i32_wrong_type() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::Bool(true));
        assert_eq!(extract_i32(&m, "v"), None);
    }

    // ── extract_u64 ──────────────────────────────────────────────────────────

    #[test]
    fn extract_u64_from_ui8() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::UI8(u64::MAX / 2));
        assert_eq!(extract_u64(&m, "v"), Some(u64::MAX / 2));
    }

    #[test]
    fn extract_u64_from_ui4() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::UI4(400_000));
        assert_eq!(extract_u64(&m, "v"), Some(400_000));
    }

    #[test]
    fn extract_u64_from_i4() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::I4(99));
        assert_eq!(extract_u64(&m, "v"), Some(99));
    }

    #[test]
    fn extract_u64_missing_key() {
        let m = map();
        assert_eq!(extract_u64(&m, "k"), None);
    }

    #[test]
    fn extract_u64_wrong_type() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::String("1".to_string()));
        assert_eq!(extract_u64(&m, "v"), None);
    }

    // ── extract_string ───────────────────────────────────────────────────────

    #[test]
    fn extract_string_present() {
        let mut m = map();
        m.insert(
            "name".to_string(),
            wmi::Variant::String("Intel".to_string()),
        );
        assert_eq!(extract_string(&m, "name"), Some("Intel".to_string()));
    }

    #[test]
    fn extract_string_missing_key() {
        let m = map();
        assert_eq!(extract_string(&m, "name"), None);
    }

    #[test]
    fn extract_string_wrong_type() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::UI4(0));
        assert_eq!(extract_string(&m, "v"), None);
    }

    // ── extract_bool ─────────────────────────────────────────────────────────

    #[test]
    fn extract_bool_true() {
        let mut m = map();
        m.insert("online".to_string(), wmi::Variant::Bool(true));
        assert_eq!(extract_bool(&m, "online"), Some(true));
    }

    #[test]
    fn extract_bool_false() {
        let mut m = map();
        m.insert("online".to_string(), wmi::Variant::Bool(false));
        assert_eq!(extract_bool(&m, "online"), Some(false));
    }

    #[test]
    fn extract_bool_missing_key() {
        let m = map();
        assert_eq!(extract_bool(&m, "online"), None);
    }

    #[test]
    fn extract_bool_wrong_type() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::UI4(1));
        assert_eq!(extract_bool(&m, "v"), None);
    }

    // ── extract_u32_or ────────────────────────────────────────────────────────

    #[test]
    fn extract_u32_or_present() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::UI4(10));
        assert_eq!(extract_u32_or(&m, "v", 99), 10);
    }

    #[test]
    fn extract_u32_or_missing() {
        let m = map();
        assert_eq!(extract_u32_or(&m, "v", 99), 99);
    }

    #[test]
    fn extract_u32_or_wrong_type() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::String("x".to_string()));
        assert_eq!(extract_u32_or(&m, "v", 99), 99);
    }

    // ── extract_string_or ─────────────────────────────────────────────────────

    #[test]
    fn extract_string_or_present() {
        let mut m = map();
        m.insert("mfr".to_string(), wmi::Variant::String("COSMX".to_string()));
        assert_eq!(extract_string_or(&m, "mfr", "default"), "COSMX");
    }

    #[test]
    fn extract_string_or_missing() {
        let m = map();
        assert_eq!(extract_string_or(&m, "mfr", "default"), "default");
    }

    #[test]
    fn extract_string_or_wrong_type() {
        let mut m = map();
        m.insert("v".to_string(), wmi::Variant::UI4(0));
        assert_eq!(extract_string_or(&m, "v", "fallback"), "fallback");
    }
}
