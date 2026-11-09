//! Panel identity from the monitor's EDID, via `root\WMI` `WmiMonitorID`.
//!
//! Windows already parses EDID and exposes four useful fields per monitor:
//! `ManufacturerName` (3-letter PNP id), `ProductCodeID` (4 hex digits as
//! ASCII, e.g. `4503`), `SerialNumberID` and `UserFriendlyName`. Together with
//! the CCD monitor device path (`\\?\DISPLAY#EDO4503#4&…`) this identifies the
//! *panel*, not the adapter — the colour tab used to know only `\\.\DISPLAY1`.
//!
//! Everything here is read-only and unprivileged. When the query or the join
//! fails we return `None` and the UI shows “—”: a wrong panel id would be worse
//! than an honest blank, because the catalogue matches profiles against it.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One monitor as Windows reports it in `WmiMonitorID`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorIdentity {
    /// `DISPLAY\EDO4503\4&17c366c7&2&UID8388688_0`
    pub instance_name: String,
    /// Three-letter EDID manufacturer id (`EDO`, `SDC`, `GSM`).
    pub manufacturer: String,
    /// Product code exactly as EDID reports it (`4503`), already hex text.
    pub product_code: String,
    /// EDID serial string; may be `0` or empty on cheap panels.
    pub serial: String,
    /// Monitor name Windows shows in Settings; often empty on laptop panels.
    pub friendly_name: String,
    pub year: Option<u16>,
    pub week: Option<u8>,
}

impl MonitorIdentity {
    /// Human label used by the UI and the catalogue matcher: `EDO 0x4503`.
    pub fn panel_id(&self) -> String {
        format!("{} 0x{}", self.manufacturer, self.product_code)
    }

    /// `EDO4503` — the token Panelook and NotebookCheck indexes use.
    pub fn model_token(&self) -> String {
        format!("{}{}", self.manufacturer, self.product_code)
    }

    /// The best human name available: the friendly name, else the panel id.
    pub fn display_name(&self) -> String {
        if self.friendly_name.is_empty() {
            self.panel_id()
        } else {
            self.friendly_name.clone()
        }
    }
}

/// Normalise a monitor instance path so the WMI instance name and the CCD
/// `monitorDevicePath` compare equal.
///
/// WMI: `DISPLAY\EDO4503\4&17c366c7&2&UID8388688_0`
/// CCD: `\\?\DISPLAY#EDO4503#4&17c366c7&2&UID8388688_0#{e6f07b5f-…}`
pub fn normalize_instance_key(raw: &str) -> String {
    let mut s = raw.trim().to_ascii_lowercase();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        s = rest.to_string();
    }
    // Strip the interface GUID suffix CCD appends.
    if let Some(idx) = s.find("#{") {
        s.truncate(idx);
    }
    let mut s = s.replace('#', "\\");
    // WmiMonitorID appends the child-device index (`…&UID8388688_0`) while the
    // CCD monitor path omits it (`…&UID8388688`). Strip a numeric `_N` suffix so
    // both forms compare equal.
    if let Some((head, tail)) = s.rsplit_once('_') {
        if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
            s = head.to_string();
        }
    }
    s
}

/// Decode a `uint16[]` CIM property that carries ASCII codes (`EDO`, `4503`).
pub fn ascii_from_variant(value: &wmi::Variant) -> String {
    match value {
        wmi::Variant::Array(items) => {
            let mut out = String::new();
            for item in items {
                let code = match item {
                    wmi::Variant::UI2(v) => *v as u32,
                    wmi::Variant::UI1(v) => *v as u32,
                    wmi::Variant::I2(v) if *v >= 0 => *v as u32,
                    wmi::Variant::UI4(v) => *v,
                    _ => 0,
                };
                if code == 0 {
                    break;
                }
                if let Some(ch) = char::from_u32(code) {
                    out.push(ch);
                }
            }
            out.trim().to_string()
        }
        wmi::Variant::String(s) => s.trim().to_string(),
        _ => String::new(),
    }
}

/// Product code as the UI wants it: `4503`.
///
/// `WmiMonitorID.ProductCodeID` is a `uint16[]` of ASCII digits on Windows 11
/// (`'4','5','0','3'`). Some providers return the numeric value instead, so
/// both shapes are accepted; anything else yields an empty string.
pub fn product_code_from_variant(value: &wmi::Variant) -> String {
    let text = ascii_from_variant(value);
    if !text.is_empty() {
        let digits: String = text
            .chars()
            .filter(|c| c.is_ascii_hexdigit())
            .collect::<String>()
            .to_ascii_uppercase();
        if !digits.is_empty() {
            return digits;
        }
    }
    match value {
        wmi::Variant::UI2(v) => format!("{v:04X}"),
        wmi::Variant::UI4(v) => format!("{v:04X}"),
        _ => String::new(),
    }
}

fn uint(value: &wmi::Variant) -> Option<u64> {
    match value {
        wmi::Variant::UI1(v) => Some(*v as u64),
        wmi::Variant::UI2(v) => Some(*v as u64),
        wmi::Variant::UI4(v) => Some(*v as u64),
        wmi::Variant::UI8(v) => Some(*v),
        wmi::Variant::I2(v) if *v >= 0 => Some(*v as u64),
        wmi::Variant::I4(v) if *v >= 0 => Some(*v as u64),
        _ => None,
    }
}

/// Pure row → identity conversion, so the parsing is testable without WMI.
pub fn monitors_from_rows(rows: &[HashMap<String, wmi::Variant>]) -> Vec<MonitorIdentity> {
    let mut out = Vec::new();
    for row in rows {
        if let Some(wmi::Variant::Bool(false)) = row.get("Active") {
            continue;
        }
        let Some(wmi::Variant::String(instance_name)) = row.get("InstanceName") else {
            continue;
        };
        let manufacturer = row
            .get("ManufacturerName")
            .map(ascii_from_variant)
            .unwrap_or_default()
            .to_ascii_uppercase();
        let product_code = row
            .get("ProductCodeID")
            .map(product_code_from_variant)
            .unwrap_or_default();
        if manufacturer.is_empty() || product_code.is_empty() {
            // Without both halves `panel_id()` would be a half-truth, and the
            // catalogue matches on them.
            continue;
        }
        out.push(MonitorIdentity {
            instance_name: instance_name.clone(),
            manufacturer,
            product_code,
            serial: row
                .get("SerialNumberID")
                .map(ascii_from_variant)
                .unwrap_or_default(),
            friendly_name: row
                .get("UserFriendlyName")
                .map(ascii_from_variant)
                .unwrap_or_default(),
            year: row
                .get("YearOfManufacture")
                .and_then(uint)
                .map(|v| v as u16),
            week: row.get("WeekOfManufacture").and_then(uint).map(|v| v as u8),
        });
    }
    out
}

/// Read every panel Windows knows about. Never fails: an unavailable WMI
/// provider yields an empty list and the UI degrades to “—”.
pub fn read_monitors() -> Vec<MonitorIdentity> {
    #[cfg(windows)]
    {
        let query = "SELECT InstanceName, ManufacturerName, ProductCodeID, \
                     SerialNumberID, UserFriendlyName, YearOfManufacture, \
                     WeekOfManufacture, Active FROM WmiMonitorID";
        match crate::hw::wmi_cache::with_wmi(|wmi| {
            wmi.raw_query::<HashMap<String, wmi::Variant>>(query)
                .map_err(|e| anyhow::anyhow!("{e}"))
        }) {
            Ok(rows) => {
                let monitors = monitors_from_rows(&rows);
                log::debug!("[edid] {} monitor(s) with EDID identity", monitors.len());
                monitors
            }
            Err(e) => {
                log::info!("[edid] WmiMonitorID unavailable — panel identity degraded: {e}");
                Vec::new()
            }
        }
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Pick the monitor that belongs to a GDI display.
///
/// `monitor_path` is the CCD `monitorDevicePath` of that display. When it is
/// missing or does not match, a single-monitor machine still resolves — but a
/// multi-monitor machine deliberately returns `None` rather than guessing.
pub fn monitor_for_display<'a>(
    monitor_path: Option<&str>,
    monitors: &'a [MonitorIdentity],
    attached_displays: usize,
) -> Option<&'a MonitorIdentity> {
    if let Some(path) = monitor_path {
        let key = normalize_instance_key(path);
        if let Some(found) = monitors
            .iter()
            .find(|m| normalize_instance_key(&m.instance_name) == key)
        {
            return Some(found);
        }
    }
    if monitors.len() == 1 && attached_displays <= 1 {
        return monitors.first();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ascii_variant(text: &str) -> wmi::Variant {
        wmi::Variant::Array(text.encode_utf16().map(|c| wmi::Variant::UI2(c)).collect())
    }

    fn row(instance: &str, mfg: &str, code: &str, name: &str) -> HashMap<String, wmi::Variant> {
        let mut map = HashMap::new();
        map.insert(
            "InstanceName".to_string(),
            wmi::Variant::String(instance.to_string()),
        );
        map.insert("ManufacturerName".to_string(), ascii_variant(mfg));
        map.insert("ProductCodeID".to_string(), ascii_variant(code));
        map.insert("SerialNumberID".to_string(), ascii_variant("0"));
        map.insert("UserFriendlyName".to_string(), ascii_variant(name));
        map.insert("YearOfManufacture".to_string(), wmi::Variant::UI2(2025));
        map.insert("WeekOfManufacture".to_string(), wmi::Variant::UI1(36));
        map
    }

    #[test]
    fn parses_the_real_edo4503_row_shape() {
        // Exactly the shape Windows 11 returns: ProductCodeID is ASCII digits.
        let rows = vec![row(
            r"DISPLAY\EDO4503\4&17c366c7&2&UID8388688_0",
            "EDO",
            "4503",
            "",
        )];
        let monitors = monitors_from_rows(&rows);
        assert_eq!(monitors.len(), 1);
        assert_eq!(monitors[0].panel_id(), "EDO 0x4503");
        assert_eq!(monitors[0].model_token(), "EDO4503");
        assert_eq!(monitors[0].display_name(), "EDO 0x4503");
        assert_eq!(monitors[0].year, Some(2025));
    }

    #[test]
    fn keeps_the_friendly_name_when_windows_has_one() {
        let rows = vec![row("DISPLAY\\GSM774A\\x_0", "GSM", "774A", "LG ULTRAGEAR")];
        let monitors = monitors_from_rows(&rows);
        assert_eq!(monitors[0].panel_id(), "GSM 0x774A");
        assert_eq!(monitors[0].display_name(), "LG ULTRAGEAR");
    }

    #[test]
    fn numeric_product_codes_are_formatted_as_hex() {
        let mut r = row("DISPLAY\\SDC\\x_0", "SDC", "", "");
        r.insert("ProductCodeID".to_string(), wmi::Variant::UI2(0x4188));
        let monitors = monitors_from_rows(&[r]);
        assert_eq!(monitors[0].product_code, "4188");
        assert_eq!(monitors[0].panel_id(), "SDC 0x4188");
    }

    #[test]
    fn drops_rows_without_a_manufacturer_or_product_code() {
        let rows = vec![row("DISPLAY\\X\\x_0", "", "4503", "")];
        assert!(monitors_from_rows(&rows).is_empty());
    }

    #[test]
    fn skips_inactive_monitors() {
        let mut r = row("DISPLAY\\EDO4503\\x_0", "EDO", "4503", "");
        r.insert("Active".to_string(), wmi::Variant::Bool(false));
        assert!(monitors_from_rows(&[r]).is_empty());
    }

    #[test]
    fn joins_the_wmi_instance_to_the_ccd_device_path() {
        let monitors = monitors_from_rows(&[row(
            r"DISPLAY\EDO4503\4&17c366c7&2&UID8388688_0",
            "EDO",
            "4503",
            "",
        )]);
        let path =
            r"\\?\DISPLAY#EDO4503#4&17c366c7&2&UID8388688_0#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}";
        let found = monitor_for_display(Some(path), &monitors, 3).expect("panel joined by path");
        assert_eq!(found.panel_id(), "EDO 0x4503");
    }

    #[test]
    fn never_guesses_a_panel_on_a_multi_monitor_machine() {
        let monitors = monitors_from_rows(&[row("DISPLAY\\EDO4503\\x_0", "EDO", "4503", "")]);
        assert!(monitor_for_display(None, &monitors, 3).is_none());
        assert!(monitor_for_display(Some(r"\\?\DISPLAY#GSM774A#y_0#{}"), &monitors, 3).is_none());
        // ... but a single attached display makes the lone monitor unambiguous.
        assert!(monitor_for_display(None, &monitors, 1).is_some());
    }

    #[test]
    fn normalises_case_separators_and_the_interface_suffix() {
        // CCD keeps the monitor path; WMI appends the child index `_0`. Both
        // must normalise to the same key.
        assert_eq!(
            normalize_instance_key(r"\\?\DISPLAY#EDO4503#4&17C366C7&2&UID8388688_0#{guid}"),
            r"display\edo4503\4&17c366c7&2&uid8388688"
        );
        assert_eq!(
            normalize_instance_key(r"\\?\DISPLAY#EDO4503#4&17C366C7&2&UID8388688#{guid}"),
            r"display\edo4503\4&17c366c7&2&uid8388688"
        );
        assert_eq!(
            normalize_instance_key(r"DISPLAY\EDO4503\4&17c366c7&2&UID8388688_0"),
            r"display\edo4503\4&17c366c7&2&uid8388688"
        );
    }

    #[test]
    fn live_edid_read_does_not_panic_and_is_well_formed() {
        let monitors = read_monitors();
        for m in &monitors {
            println!(
                "edid: {} name={:?} serial={:?}",
                m.panel_id(),
                m.friendly_name,
                m.serial
            );
            assert_eq!(m.manufacturer.len(), 3, "EDID manufacturer ids are 3 chars");
            assert!(!m.product_code.is_empty());
        }
    }

    /// Live join check: every CCD monitor path must normalise to a WMI instance
    /// on this machine. If Windows ever changes either format, this test is the
    /// one that says so before the UI silently degrades to "—".
    #[cfg(windows)]
    #[test]
    fn live_ccd_monitor_paths_join_to_edid_instances() {
        let monitors = read_monitors();
        let paths = crate::hw::display::monitor_device_paths();
        for (gdi, path) in &paths {
            println!("ccd: {gdi} -> {path}");
        }
        for m in &monitors {
            println!("wmi: {}", m.instance_name);
        }
        if paths.is_empty() || monitors.is_empty() {
            println!("skipped: no CCD paths or no EDID monitors on this machine");
            return;
        }
        let joined = paths
            .values()
            .filter(|path| {
                monitors.iter().any(|m| {
                    normalize_instance_key(&m.instance_name) == normalize_instance_key(path)
                })
            })
            .count();
        assert!(
            joined > 0,
            "no CCD monitor path matched a WmiMonitorID instance"
        );
    }
}
