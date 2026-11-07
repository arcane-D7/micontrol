//! PC WiFi management via Windows netsh wlan.
//!
//! Provides network scanning, connection status, and connect/disconnect
//! functionality by parsing `netsh wlan` command output.

use crate::hw::errors::{HardwareError, HardwareResult};
use crate::util::xml;
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::time::{Duration, Instant};
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// A WiFi network (SSID) visible to the PC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiNetwork {
    pub ssid: String,
    pub signal: u32,      // 0-100 percentage
    pub security: String, // e.g. "WPA2-Personal", "Open"
    pub connected: bool,
}

/// Current WiFi connection status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiStatus {
    pub connected: bool,
    pub ssid: Option<String>,
    pub signal: Option<u32>,
    pub interface: Option<String>,
}

/// Scan for available WiFi networks using the native WlanAPI.
///
/// Uses wlanapi.dll instead of shelling out to `netsh wlan`. This is:
/// - Locale-independent (structured data, not parsed text)
/// - Faster (no arbitrary sleep — scan completes via notification)
/// - More accurate (raw signal quality, not parsed percentage)
#[cfg(windows)]
pub fn scan_networks() -> HardwareResult<Vec<WifiNetwork>> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::NetworkManagement::WiFi::{
        WlanCloseHandle, WlanEnumInterfaces, WlanGetAvailableNetworkList, WlanOpenHandle, WlanScan,
        WLAN_API_VERSION_2_0, WLAN_AVAILABLE_NETWORK_LIST, WLAN_INTERFACE_INFO_LIST,
    };

    unsafe {
        // 1. Open WLAN handle
        let mut handle = HANDLE::default();
        let mut negotiated_version = 0u32;
        let ret = WlanOpenHandle(
            WLAN_API_VERSION_2_0,
            None,
            &mut negotiated_version,
            &mut handle,
        );
        if ret != 0 {
            return Err(HardwareError::Wifi(format!(
                "WlanOpenHandle failed with error code {ret}"
            )));
        }

        // 2. Enumerate interfaces
        let mut iface_list_ptr: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        let ret = WlanEnumInterfaces(handle, None, &mut iface_list_ptr);
        if ret != 0 {
            WlanCloseHandle(handle, None);
            return Err(HardwareError::Wifi(format!(
                "WlanEnumInterfaces failed with error code {ret}"
            )));
        }
        let iface_list = &*iface_list_ptr;

        if iface_list.dwNumberOfItems == 0 {
            WlanCloseHandle(handle, None);
            return Ok(vec![]);
        }
        let guid = iface_list.InterfaceInfo[0].InterfaceGuid;

        // 3. Trigger scan (async, returns immediately)
        let ret = WlanScan(handle, &guid, None, None, None);
        if ret != 0 {
            WlanCloseHandle(handle, None);
            return Err(HardwareError::Wifi(format!(
                "WlanScan failed with error code {ret}"
            )));
        }

        // 4. Wait for scan completion (4s pragmatic — proper fix uses WlanRegisterNotification)
        std::thread::sleep(std::time::Duration::from_millis(4000));

        // 5. Get available network list (structured, locale-independent!)
        let mut network_list_ptr: *mut WLAN_AVAILABLE_NETWORK_LIST = std::ptr::null_mut();
        let ret = WlanGetAvailableNetworkList(handle, &guid, 0, None, &mut network_list_ptr);
        if ret != 0 {
            WlanCloseHandle(handle, None);
            return Err(HardwareError::Wifi(format!(
                "WlanGetAvailableNetworkList failed with error code {ret}"
            )));
        }
        let network_list = &*network_list_ptr;

        // 5b. Query current connection to determine which network is connected.
        // WLAN_AVAILABLE_NETWORK has a `bNetworkConnectable` flag but NOT a
        // "currently connected" flag, so we must compare SSIDs.
        let connected_ssid: Option<String> = {
            use windows::Win32::NetworkManagement::WiFi::{
                wlan_intf_opcode_current_connection, WlanQueryInterface, WLAN_CONNECTION_ATTRIBUTES,
            };
            let mut conn_data: *mut std::ffi::c_void = std::ptr::null_mut();
            let mut conn_size = 0u32;
            let conn_ret = WlanQueryInterface(
                handle,
                &guid,
                wlan_intf_opcode_current_connection,
                None,
                &mut conn_size,
                &mut conn_data,
                None,
            );
            if conn_ret == 0 && !conn_data.is_null() {
                let conn_attrs = &*(conn_data as *const WLAN_CONNECTION_ATTRIBUTES);
                let assoc = &conn_attrs.wlanAssociationAttributes;
                let ssid_len = assoc.dot11Ssid.uSSIDLength as usize;
                let ssid_bytes = &assoc.dot11Ssid.ucSSID[..ssid_len];
                let ssid_str = String::from_utf8_lossy(ssid_bytes).to_string();
                if ssid_str.is_empty() {
                    None
                } else {
                    Some(ssid_str)
                }
            } else {
                None
            }
        };

        // 6. Parse structured data into WifiNetwork
        // WLAN_AVAILABLE_NETWORK_LIST uses a C-style flexible array member:
        // the `Network` field is declared as [WLAN_AVAILABLE_NETWORK; 1] but
        // the actual allocation contains `dwNumberOfItems` entries. We must
        // use pointer arithmetic to access entries beyond index 0.
        let mut networks = Vec::new();
        let mut hidden_count = 0u32;
        let base = network_list.Network.as_ptr();
        for i in 0..network_list.dwNumberOfItems {
            let net = &*base.add(i as usize);

            // dot11Ssid: 4-byte length + 32-byte SSID buffer
            let ssid_len = net.dot11Ssid.uSSIDLength as usize;
            let ssid_bytes = &net.dot11Ssid.ucSSID[..ssid_len];
            let ssid = String::from_utf8_lossy(ssid_bytes).to_string();

            // wlanSignalQuality: 0-100 percentage
            let signal = net.wlanSignalQuality;

            // bSecurityEnabled: bool
            let security = if net.bSecurityEnabled.as_bool() {
                "WPA2-Personal"
            } else {
                "Open"
            };

            // Check if this is the currently connected network
            let is_connected = connected_ssid.as_ref().is_some_and(|s| s == &ssid);

            // Skip hidden networks (empty SSID) — they show as blank entries
            // and cause duplicate React key issues. We'll add a placeholder
            // hidden network entry at the end if any were found.
            if ssid.is_empty() {
                hidden_count += 1;
                continue;
            }

            networks.push(WifiNetwork {
                ssid,
                signal,
                security: security.to_string(),
                connected: is_connected,
            });
        }

        // Deduplicate by SSID — WlanAPI returns one entry per BSS, so the
        // same SSID from multiple APs (e.g., 2.4GHz + 5GHz) appears multiple
        // times. We keep the entry with the strongest signal and preserve
        // the connected flag.
        networks.sort_by_key(|n| std::cmp::Reverse(n.signal));
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut deduped: Vec<WifiNetwork> = Vec::with_capacity(networks.len());
        for net in networks {
            if seen.insert(net.ssid.clone()) {
                deduped.push(net);
            }
        }

        // Add a single placeholder for hidden networks if any were found
        if hidden_count > 0 {
            deduped.push(WifiNetwork {
                ssid: "<Hidden Network>".to_string(),
                signal: 0,
                security: "Unknown".to_string(),
                connected: connected_ssid.as_ref().is_some_and(|s| s.is_empty()),
            });
        }

        WlanCloseHandle(handle, None);
        Ok(deduped)
    }
}

#[cfg(not(windows))]
pub fn scan_networks() -> HardwareResult<Vec<WifiNetwork>> {
    Err(HardwareError::NotSupported(
        "WiFi scanning is Windows-only".into(),
    ))
}

/// Get current WiFi connection status using native WlanAPI.
///
/// Uses `WlanQueryInterface` with structured data instead of parsing
/// `netsh wlan show interfaces` text output. This is locale-independent.
///
/// Queries two opcodes:
/// 1. `wlan_intf_opcode_interface_state` — connected/disconnected enum
/// 2. `wlan_intf_opcode_current_connection` — SSID + signal quality (when connected)
#[cfg(windows)]
pub fn get_status() -> HardwareResult<WifiStatus> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::NetworkManagement::WiFi::{
        wlan_intf_opcode_current_connection, wlan_intf_opcode_interface_state, WlanCloseHandle,
        WlanEnumInterfaces, WlanOpenHandle, WlanQueryInterface, WLAN_API_VERSION_2_0,
        WLAN_CONNECTION_ATTRIBUTES, WLAN_INTERFACE_INFO_LIST,
    };

    unsafe {
        let mut handle = HANDLE::default();
        let mut negotiated = 0u32;
        let ret = WlanOpenHandle(WLAN_API_VERSION_2_0, None, &mut negotiated, &mut handle);
        if ret != 0 {
            return Err(HardwareError::Wifi(format!(
                "WlanOpenHandle failed with error code {ret}"
            )));
        }

        let mut iface_list_ptr: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        let ret = WlanEnumInterfaces(handle, None, &mut iface_list_ptr);
        if ret != 0 {
            WlanCloseHandle(handle, None);
            return Err(HardwareError::Wifi(format!(
                "WlanEnumInterfaces failed with error code {ret}"
            )));
        }
        let iface_list = &*iface_list_ptr;

        if iface_list.dwNumberOfItems == 0 {
            WlanCloseHandle(handle, None);
            return Ok(WifiStatus {
                connected: false,
                ssid: None,
                signal: None,
                interface: None,
            });
        }

        let guid = iface_list.InterfaceInfo[0].InterfaceGuid;
        let interface_name =
            String::from_utf16_lossy(&iface_list.InterfaceInfo[0].strInterfaceDescription)
                .trim_end_matches('\0')
                .to_string();

        // Query interface state (enum, not locale-dependent string)
        let mut state_data: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut state_size = 0u32;
        let ret = WlanQueryInterface(
            handle,
            &guid,
            wlan_intf_opcode_interface_state,
            None,
            &mut state_size,
            &mut state_data,
            None,
        );
        if ret != 0 {
            WlanCloseHandle(handle, None);
            return Err(HardwareError::Wifi(format!(
                "WlanQueryInterface(state) failed with error code {ret}"
            )));
        }

        // WLAN_INTERFACE_STATE: 0=not ready, 1=connected, 2=ad_hoc_network_formed,
        // 3=disconnecting, 4=disconnected, 5=associating, 6=discovering, 7=authenticating
        let state = *(state_data as *const u32);
        let connected = state == 1; // wlan_interface_state_connected

        // When connected, query the current connection to get SSID and signal
        let (ssid, signal) = if connected {
            let mut conn_data: *mut std::ffi::c_void = std::ptr::null_mut();
            let mut conn_size = 0u32;
            let conn_ret = WlanQueryInterface(
                handle,
                &guid,
                wlan_intf_opcode_current_connection,
                None,
                &mut conn_size,
                &mut conn_data,
                None,
            );

            if conn_ret == 0 && !conn_data.is_null() {
                let conn_attrs = &*(conn_data as *const WLAN_CONNECTION_ATTRIBUTES);
                let assoc = &conn_attrs.wlanAssociationAttributes;

                // Extract SSID from DOT11_SSID (4-byte length + 32-byte buffer)
                let ssid_len = assoc.dot11Ssid.uSSIDLength as usize;
                let ssid_bytes = &assoc.dot11Ssid.ucSSID[..ssid_len];
                let ssid_str = String::from_utf8_lossy(ssid_bytes).to_string();
                let ssid = if ssid_str.is_empty() {
                    None
                } else {
                    Some(ssid_str)
                };

                // wlanSignalQuality: 0-100 percentage
                let signal = assoc.wlanSignalQuality;

                (ssid, Some(signal))
            } else {
                log::warn!(
                    "[wifi] WlanQueryInterface(current_connection) failed with error code {conn_ret} — SSID unavailable"
                );
                (None, None)
            }
        } else {
            (None, None)
        };

        WlanCloseHandle(handle, None);

        Ok(WifiStatus {
            connected,
            ssid,
            signal,
            interface: Some(interface_name),
        })
    }
}

#[cfg(not(windows))]
pub fn get_status() -> HardwareResult<WifiStatus> {
    Err(HardwareError::NotSupported(
        "WiFi status is Windows-only".into(),
    ))
}

/// How long `connect` waits for the OS to confirm the association.
const ASSOCIATION_TIMEOUT: Duration = Duration::from_secs(15);
/// Poll interval while waiting for the association.
const ASSOCIATION_POLL: Duration = Duration::from_millis(400);

/// Build a WLAN profile XML for `ssid`.
///
/// `Some(password)` produces a WPA2-PSK/AES profile; `None` produces an open
/// (`open`/`none`) profile. The profile name is the SSID — that is the key
/// both `netsh wlan connect name=` and `WlanConnect` address.
///
/// Only WPA2-PSK is generated on purpose: the security type of the target is
/// not known here (the UI does not pass it), and guessing WPA3/SAE or 802.1X
/// would produce a profile that cannot associate. When the generated profile
/// is rejected, the error carries the WLAN reason code instead of a silent
/// failure.
fn build_profile_xml(ssid: &str, password: Option<&str>) -> String {
    let escaped_ssid = xml::escape_xml(ssid);
    let security = match password {
        Some(pwd) => format!(
            r#"<security>
            <authEncryption>
                <authentication>WPA2PSK</authentication>
                <encryption>AES</encryption>
                <useOneX>false</useOneX>
            </authEncryption>
            <sharedKey>
                <keyType>passPhrase</keyType>
                <protected>false</protected>
                <keyMaterial>{}</keyMaterial>
            </sharedKey>
        </security>"#,
            xml::escape_xml(pwd)
        ),
        None => r#"<security>
            <authEncryption>
                <authentication>open</authentication>
                <encryption>none</encryption>
                <useOneX>false</useOneX>
            </authEncryption>
        </security>"#
            .to_string(),
    };

    format!(
        r#"<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
    <name>{escaped_ssid}</name>
    <SSIDConfig>
        <SSID>
            <name>{escaped_ssid}</name>
        </SSID>
    </SSIDConfig>
    <connectionType>ESS</connectionType>
    <connectionMode>auto</connectionMode>
    <MSM>
        {security}
    </MSM>
</WLANProfile>"#
    )
}

/// Install a profile from memory (no temporary file, so the passphrase never
/// reaches the disk). Per-user profile (`WLAN_PROFILE_USER`) — same scope as
/// `netsh wlan add profile` for a non-elevated caller.
#[cfg(windows)]
fn set_profile_in_memory(profile_xml: &str, overwrite: bool) -> HardwareResult<()> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{BOOL, HANDLE};
    use windows::Win32::NetworkManagement::WiFi::{
        WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory, WlanOpenHandle, WlanSetProfile,
        WLAN_API_VERSION_2_0, WLAN_INTERFACE_INFO_LIST, WLAN_PROFILE_USER,
    };

    let wide: Vec<u16> = profile_xml
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: every handle allocated here is released before returning; the
    // wide buffer outlives the WlanSetProfile call; the reason-code pointer is
    // a valid stack slot.
    unsafe {
        let mut handle = HANDLE::default();
        let mut negotiated = 0u32;
        let ret = WlanOpenHandle(WLAN_API_VERSION_2_0, None, &mut negotiated, &mut handle);
        if ret != 0 {
            return Err(HardwareError::Wifi(format!(
                "WlanOpenHandle failed with error code {ret}"
            )));
        }

        let mut list_ptr: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        let ret = WlanEnumInterfaces(handle, None, &mut list_ptr);
        if ret != 0 || list_ptr.is_null() {
            WlanCloseHandle(handle, None);
            return Err(HardwareError::Wifi(format!(
                "WlanEnumInterfaces failed with error code {ret}"
            )));
        }
        let iface_count = (*list_ptr).dwNumberOfItems;
        let guid = if iface_count > 0 {
            Some((*list_ptr).InterfaceInfo[0].InterfaceGuid)
        } else {
            None
        };
        WlanFreeMemory(list_ptr as *const std::ffi::c_void);

        let Some(guid) = guid else {
            WlanCloseHandle(handle, None);
            return Err(HardwareError::Wifi(
                "no wireless interface to add the profile to".into(),
            ));
        };

        let mut reason_code = 0u32;
        let ret = WlanSetProfile(
            handle,
            &guid,
            WLAN_PROFILE_USER,
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            BOOL::from(overwrite),
            None,
            &mut reason_code,
        );
        WlanCloseHandle(handle, None);

        if ret != 0 {
            return Err(HardwareError::Wifi(format!(
                "WlanSetProfile failed (error {ret}, reason {reason_code})"
            )));
        }
        log::info!("[wifi] profile installed in memory (overwrite={overwrite})");
        Ok(())
    }
}

/// Ask the WLAN service to associate with the profile named `ssid`.
///
/// `netsh` is kept for this one call (it needs no elevation and returns as
/// soon as the request is queued); the caller verifies the real association.
fn request_association(ssid: &str) -> HardwareResult<()> {
    let mut cmd = Command::new("netsh");
    // One `name=<ssid>` token: Command quotes it, so SSIDs with spaces or '&'
    // stay intact (the process is spawned directly, never through cmd.exe, so
    // '&' is not reinterpreted).
    cmd.args(["wlan", "connect"]).arg(format!("name={ssid}"));
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd
        .output()
        .map_err(|e| HardwareError::Wifi(format!("Failed to connect: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(HardwareError::Wifi(format!(
            "Failed to connect to '{ssid}': {}",
            if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                stderr.trim()
            }
        )));
    }
    Ok(())
}

/// Does the reported SSID denote the network we asked for?
fn ssid_eq(reported: Option<&str>, wanted: &str) -> bool {
    match reported {
        Some(s) => s == wanted,
        None => false,
    }
}

/// Same as `ssid_eq`, but tolerates surrounding whitespace differences (some
/// callers trim the SSID). Logged when it changes the outcome.
fn ssid_eq_loose(reported: Option<&str>, wanted: &str) -> bool {
    ssid_eq(reported, wanted)
        || reported.is_some_and(|s| s.trim() == wanted.trim() && !s.trim().is_empty())
}

/// Poll until the OS reports "connected to `ssid`", or the timeout expires.
///
/// This is what turns `wifi_connect` from "the request was accepted" into
/// "the association happened": `netsh wlan connect` returns before the
/// 802.11 association completes, so the old code could toast "Connected"
/// while the radio was still failing authentication.
fn wait_for_association_with<F>(
    ssid: &str,
    timeout: Duration,
    interval: Duration,
    mut poll: F,
) -> HardwareResult<()>
where
    F: FnMut() -> HardwareResult<WifiStatus>,
{
    let started = Instant::now();
    let mut last: Option<WifiStatus> = None;

    loop {
        match poll() {
            Ok(status) => {
                if status.connected && ssid_eq(status.ssid.as_deref(), ssid) {
                    log::info!("[wifi] associated with '{ssid}'");
                    return Ok(());
                }
                if status.connected && ssid_eq_loose(status.ssid.as_deref(), ssid) {
                    log::warn!(
                        "[wifi] associated with '{ssid}' (matched after trimming whitespace)"
                    );
                    return Ok(());
                }
                last = Some(status);
            }
            Err(e) => log::debug!("[wifi] status poll failed during connect: {e}"),
        }

        if started.elapsed() >= timeout {
            break;
        }
        std::thread::sleep(interval);
    }

    let detail = match last {
        Some(st) if st.connected => format!(
            "the adapter is still associated with '{}'",
            st.ssid.unwrap_or_default()
        ),
        _ => "the adapter is not associated".to_string(),
    };
    Err(HardwareError::Wifi(format!(
        "Association to '{ssid}' was not confirmed within {} s ({detail}). The password or the security type may be wrong, or the access point is out of range.",
        timeout.as_secs()
    )))
}

/// Connect to a WiFi network and confirm the association.
///
/// The SSID and password are validated and XML-escaped before being
/// interpolated into the WLAN profile template to prevent XML injection.
pub fn connect(ssid: &str, password: Option<&str>) -> HardwareResult<()> {
    // Validate the SSID before any XML construction.
    xml::validate_ssid(ssid).map_err(|e| HardwareError::Wifi(format!("Invalid SSID: {e}")))?;

    // Validate the password if provided.
    if let Some(pwd) = password {
        xml::validate_wpa2_passphrase(pwd)
            .map_err(|e| HardwareError::Wifi(format!("Invalid WPA2 passphrase: {e}")))?;
    }

    // Fast path: already associated to this network.
    if let Ok(status) = get_status() {
        if status.connected && ssid_eq(status.ssid.as_deref(), ssid) {
            log::info!("[wifi] connect '{ssid}': already associated");
            return Ok(());
        }
    }

    log::info!(
        "[wifi] connect '{ssid}' (password supplied: {})",
        password.is_some()
    );

    // 1. Materialise the profile — in memory. The passphrase used to be
    //    written in clear text to `%TEMP%\micontrol_wifi_<rand>.xml`; it never
    //    touches the disk now.
    let profile_xml = build_profile_xml(ssid, password);
    #[cfg(windows)]
    let profile_err = {
        // A password means "(re)configure this profile"; no password must
        // never overwrite a saved profile with an open one (that would
        // downgrade a secured network), so it is only installed if absent.
        set_profile_in_memory(&profile_xml, password.is_some()).err()
    };
    #[cfg(not(windows))]
    let profile_err: Option<HardwareError> = Some(HardwareError::NotSupported(
        "WiFi connect is Windows-only".into(),
    ));

    // 2. Ask for the association. If no password was supplied and the profile
    //    did not exist, the open profile installed above is the one used.
    if let Err(e) = request_association(ssid) {
        if password.is_some() {
            return Err(e);
        }
        // Surfaces the profile reason when the connect itself failed for the
        // same cause (e.g. WPA3-only AP).
        if let Some(profile_err) = profile_err {
            return Err(HardwareError::Wifi(format!("{e} (profile: {profile_err})")));
        }
        return Err(e);
    }

    // 3. Success means "the OS says we are associated to that SSID".
    wait_for_association_with(ssid, ASSOCIATION_TIMEOUT, ASSOCIATION_POLL, get_status)
}

/// Disconnect from current WiFi network.
pub fn disconnect() -> HardwareResult<()> {
    let mut cmd = Command::new("netsh");
    cmd.args(["wlan", "disconnect"]);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd
        .output()
        .map_err(|e| HardwareError::Wifi(format!("Failed to disconnect: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(HardwareError::Wifi(format!(
            "Failed to disconnect: {stderr}"
        )));
    }

    Ok(())
}

/// Parse netsh wlan show networks output.
#[allow(dead_code)]
fn parse_scan_output(output: &str) -> HardwareResult<Vec<WifiNetwork>> {
    let mut networks: Vec<WifiNetwork> = Vec::new();
    let mut current_ssid: Option<String> = None;
    let mut current_signal: u32 = 0;
    let mut current_security: String = String::new();

    for line in output.lines() {
        let line = line.trim();
        if line.starts_with("SSID ") && !line.contains("BSSID") {
            // Save previous network
            if let Some(ssid) = current_ssid.take() {
                networks.push(WifiNetwork {
                    ssid,
                    signal: current_signal,
                    security: current_security.clone(),
                    connected: false,
                });
            }
            // Start new network
            // netsh output format: "SSID 1 : MyNetwork" or "SSID 2 :"
            // We need to extract just the SSID name after the colon
            let ssid = if let Some(colon_pos) = line.find(':') {
                line[colon_pos + 1..].trim().to_string()
            } else {
                String::new()
            };
            if !ssid.is_empty() {
                current_ssid = Some(ssid);
                current_signal = 0;
                current_security = String::new();
            }
        } else if line.starts_with("Signal") {
            if let Some(pct) = line.split(':').nth(1) {
                current_signal = pct.trim().trim_matches('%').parse().unwrap_or(0);
            }
        } else if line.starts_with("Authentication") {
            if let Some(auth) = line.split(':').nth(1) {
                current_security = auth.trim().to_string();
            }
        }
    }

    // Don't forget the last network
    if let Some(ssid) = current_ssid {
        networks.push(WifiNetwork {
            ssid,
            signal: current_signal,
            security: current_security,
            connected: false,
        });
    }

    Ok(networks)
}

/// Parse netsh wlan show interfaces output.
#[allow(dead_code)]
fn parse_interface_output(output: &str) -> HardwareResult<WifiStatus> {
    let mut ssid: Option<String> = None;
    let mut signal: Option<u32> = None;
    let mut interface: Option<String> = None;
    let mut state: Option<String> = None;

    for line in output.lines() {
        let line = line.trim();
        if line.starts_with("Name") && line.contains(':') {
            interface = line.split(':').nth(1).map(|s| s.trim().to_string());
        } else if line.starts_with("SSID") && line.contains(':') {
            ssid = line.split(':').nth(1).map(|s| s.trim().to_string());
        } else if line.starts_with("State") && line.contains(':') {
            state = line.split(':').nth(1).map(|s| s.trim().to_string());
        } else if line.starts_with("Signal") && line.contains(':') {
            if let Some(sig_str) = line.split(':').nth(1) {
                signal = sig_str.trim().trim_matches('%').parse().ok();
            }
        }
    }

    let connected = state.as_deref() == Some("connected");

    Ok(WifiStatus {
        connected,
        ssid: if connected { ssid } else { None },
        signal,
        interface,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::xml;

    #[test]
    fn test_get_status_does_not_panic() {
        // This test just verifies the command runs without panicking
        let _ = get_status();
    }

    // NOTE: there used to be a `test_scan_networks_does_not_panic` here. It
    // called `scan_networks()`, which issues a real `WlanScan` on the radio
    // and blocks for 4 s — a unit test must not touch the airwaves. The
    // parsing/FFI paths it covered are exercised by `get_status` (read-only)
    // and by the profile/association tests below.

    #[test]
    fn test_parse_empty_scan() {
        let result = parse_scan_output("");
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_parse_empty_interface() {
        let result = parse_interface_output("");
        assert!(result.is_ok());
    }

    #[test]
    fn test_ssid_injection_prevented() {
        // An SSID that tries to break out of the <name> element
        let malicious = "test</name><name>evil";
        let escaped = xml::escape_xml(malicious);
        // The escaped version should not contain raw XML tags
        assert!(!escaped.contains("</name>"));
        assert!(!escaped.contains("<name>"));
    }

    #[test]
    fn test_password_injection_prevented() {
        // A password that tries to break out of <keyMaterial>
        let malicious = "</keyMaterial><x>";
        let escaped = xml::escape_xml(malicious);
        assert!(!escaped.contains("</keyMaterial>"));
        assert!(escaped.contains("&lt;/keyMaterial&gt;"));
    }

    #[test]
    fn test_oversized_ssid_rejected() {
        let long_ssid = "a".repeat(33);
        assert!(xml::validate_ssid(&long_ssid).is_err());
    }

    #[test]
    fn test_short_password_rejected() {
        assert!(xml::validate_wpa2_passphrase("short").is_err());
    }

    #[test]
    fn test_valid_ssid_accepted() {
        assert!(xml::validate_ssid("MyHomeNetwork").is_ok());
    }

    #[test]
    fn test_valid_password_accepted() {
        assert!(xml::validate_wpa2_passphrase("correct horse battery staple").is_ok());
    }

    #[test]
    fn test_profile_xml_well_formed_after_escape() {
        // Build the profile XML the same way connect() does, with a malicious SSID
        let ssid = "test</name><name>evil";
        let pwd = "password</keyMaterial><x>";
        // validate_ssid should pass (it's 23 bytes, under 32)
        assert!(xml::validate_ssid(ssid).is_ok());
        // validate_wpa2_passphrase should pass (it's > 8 chars)
        assert!(xml::validate_wpa2_passphrase(pwd).is_ok());

        let profile_xml = build_profile_xml(ssid, Some(pwd));

        // The profile XML should not contain any raw injection tags
        assert!(!profile_xml.contains("</name><name>evil"));
        assert!(!profile_xml.contains("</keyMaterial><x>"));
        // It should contain the escaped versions
        assert!(profile_xml.contains("&lt;/name&gt;"));
        assert!(profile_xml.contains("&lt;/keyMaterial&gt;"));
    }

    #[test]
    fn profile_xml_uses_wpa2psk_only_when_a_password_is_given() {
        let secured = build_profile_xml("Home", Some("correct horse battery staple"));
        assert!(secured.contains("<authentication>WPA2PSK</authentication>"));
        assert!(secured.contains("<encryption>AES</encryption>"));
        assert!(secured.contains("<keyMaterial>correct horse battery staple</keyMaterial>"));
        assert!(secured.contains("<name>Home</name>"));

        let open = build_profile_xml("Cafe", None);
        assert!(open.contains("<authentication>open</authentication>"));
        assert!(open.contains("<encryption>none</encryption>"));
        assert!(!open.contains("keyMaterial"));
    }

    /// The SSID actually in use on this machine has a trailing space: the exact
    /// comparison must accept it and the loose fallback must only bridge
    /// whitespace differences.
    #[test]
    fn ssid_comparison_is_exact_then_whitespace_tolerant() {
        assert!(ssid_eq(Some("Ramtula & Freitas "), "Ramtula & Freitas "));
        assert!(!ssid_eq(Some("Ramtula & Freitas "), "Ramtula & Freitas"));
        assert!(ssid_eq_loose(
            Some("Ramtula & Freitas "),
            "Ramtula & Freitas"
        ));
        assert!(!ssid_eq(None, "Ramtula & Freitas"));
        assert!(!ssid_eq(Some("Other"), "Ramtula & Freitas"));
        // Whitespace-only SSIDs are degenerate: they still compare exactly
        // (the strings are equal), but the loose fallback must never turn an
        // empty SSID into a match.
        assert!(ssid_eq_loose(Some("   "), "   "));
        assert!(!ssid_eq_loose(Some(""), "Home"));
        assert!(!ssid_eq(Some(""), "Home"));
    }

    fn status(connected: bool, ssid: Option<&str>) -> WifiStatus {
        WifiStatus {
            connected,
            ssid: ssid.map(str::to_string),
            signal: connected.then_some(70),
            interface: Some("test".into()),
        }
    }

    /// netsh returns before the 802.11 association completes. Success must be
    /// reported only once the OS confirms the target SSID.
    #[test]
    fn association_wait_succeeds_only_after_the_target_ssid_is_reported() {
        let mut polls = 0;
        let result = wait_for_association_with(
            "Home",
            Duration::from_secs(1),
            Duration::from_millis(1),
            || {
                polls += 1;
                Ok(match polls {
                    1 => status(false, None),
                    2 => status(true, Some("Neighbour")),
                    _ => status(true, Some("Home")),
                })
            },
        );
        assert!(result.is_ok());
        assert_eq!(polls, 3, "must not short-circuit before the SSID matches");
    }

    /// Staying on another network must fail — the old code toasted success.
    #[test]
    fn association_wait_fails_when_another_ssid_remains() {
        let err = wait_for_association_with(
            "Home",
            Duration::from_millis(30),
            Duration::from_millis(1),
            || Ok(status(true, Some("Neighbour"))),
        )
        .expect_err("associated to the wrong SSID must not be reported as success");
        let msg = format!("{err}");
        assert!(msg.contains("Home"), "error should name the target: {msg}");
        assert!(
            msg.contains("Neighbour"),
            "error should say what we are still associated with: {msg}"
        );
    }

    #[test]
    fn association_wait_fails_when_never_connected() {
        let err = wait_for_association_with(
            "Home",
            Duration::from_millis(20),
            Duration::from_millis(1),
            || Ok(status(false, None)),
        )
        .expect_err("disconnected must not be reported as success");
        assert!(format!("{err}").contains("not associated"));
    }

    /// A failing status poll is not a success either.
    #[test]
    fn association_wait_fails_when_status_poll_errors() {
        let err = wait_for_association_with(
            "Home",
            Duration::from_millis(20),
            Duration::from_millis(1),
            || Err(crate::hw::errors::HardwareError::Wifi("boom".into())),
        )
        .expect_err("status errors must not be reported as success");
        assert!(format!("{err}").contains("not confirmed"));
    }
}
