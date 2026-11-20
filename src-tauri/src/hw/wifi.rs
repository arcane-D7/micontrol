//! PC WiFi management via Windows netsh wlan.
//!
//! Provides network scanning, connection status, and connect/disconnect
//! functionality by parsing `netsh wlan` command output.

use crate::hw::errors::{HardwareError, HardwareResult};
use crate::util::process_spawn;
use crate::util::xml;
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::time::{Duration, Instant};

#[cfg(windows)]
use windows::core::GUID;
#[cfg(windows)]
use windows::Win32::Foundation::HANDLE;
#[cfg(windows)]
use windows::Win32::NetworkManagement::WiFi::{WlanCloseHandle, WlanFreeMemory};

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

/// How long Windows needs between `WlanScan` and a useful list.
///
/// `WlanScan` only queues the request; results arrive as beacons/probe
/// responses do. 4 s matches the refresh of the Windows network flyout.
///
/// The wait is `tokio::time::sleep` on purpose: `scan_networks` used to run on
/// the fixed blocking pool, where a 4 s `thread::sleep` kept one pool thread
/// (shared by every hardware query) busy doing nothing.
const SCAN_SETTLE: Duration = Duration::from_millis(4000);

// ── WlanAPI ownership guards ────────────────────────────────────────────────
//
// Every pointer the WLAN API returns (`WLAN_INTERFACE_INFO_LIST`,
// `WLAN_AVAILABLE_NETWORK_LIST`, `WlanQueryInterface` buffers) is caller-owned
// and must be released with `WlanFreeMemory`; `WlanOpenHandle` must be paired
// with `WlanCloseHandle`. Both used to be released by hand on the happy path
// only, so every early `return Err(..)` leaked. These guards make the release
// unconditional.

/// A `WlanOpenHandle` client handle, closed exactly once.
#[cfg(windows)]
struct WlanSession(HANDLE);

// SAFETY: the WLAN client handle is an opaque process-wide context. The API
// does not require calls from the creating thread and `WlanCloseHandle` may be
// called from another one. This guard is owned by one task at a time, so it is
// never used concurrently from two threads.
#[cfg(windows)]
unsafe impl Send for WlanSession {}

#[cfg(windows)]
impl WlanSession {
    fn open() -> HardwareResult<Self> {
        use windows::Win32::NetworkManagement::WiFi::{WlanOpenHandle, WLAN_API_VERSION_2_0};

        let mut handle = HANDLE::default();
        let mut negotiated_version = 0u32;
        // SAFETY: `handle` and `negotiated_version` are valid out-parameters.
        let ret = unsafe {
            WlanOpenHandle(
                WLAN_API_VERSION_2_0,
                None,
                &mut negotiated_version,
                &mut handle,
            )
        };
        if ret != 0 {
            return Err(HardwareError::Wifi(format!(
                "WlanOpenHandle failed with error code {ret}"
            )));
        }
        Ok(Self(handle))
    }

    fn raw(&self) -> HANDLE {
        self.0
    }
}

#[cfg(windows)]
impl Drop for WlanSession {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful `WlanOpenHandle` and is
        // closed here exactly once.
        let _ = unsafe { WlanCloseHandle(self.0, None) };
    }
}

/// A buffer allocated by the WLAN API, released with `WlanFreeMemory` on drop.
#[cfg(windows)]
struct WlanBuf<T> {
    ptr: *mut T,
}

#[cfg(windows)]
impl<T> WlanBuf<T> {
    /// # Safety
    /// `ptr` must be null or a live allocation returned by the WLAN API whose
    /// payload begins with a `T`.
    unsafe fn from_raw(ptr: *mut T) -> Option<Self> {
        (!ptr.is_null()).then_some(Self { ptr })
    }
}

#[cfg(windows)]
impl<T> std::ops::Deref for WlanBuf<T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: `ptr` is non-null and points at a live `T` (see `from_raw`);
        // the allocation outlives this borrow because the guard owns it.
        unsafe { &*self.ptr }
    }
}

#[cfg(windows)]
impl<T> Drop for WlanBuf<T> {
    fn drop(&mut self) {
        // SAFETY: allocated by the WLAN API and not freed before (see
        // `from_raw`).
        unsafe { WlanFreeMemory(self.ptr as *const std::ffi::c_void) };
    }
}

/// Coarse security type of a network, as advertised by the scan.
///
/// The variants drive both the label shown in the UI and the profile the
/// connect path writes, so what is displayed and what is attempted agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WifiSecurity {
    Open,
    Wpa2Personal,
    Wpa3Personal,
    /// 802.1X / EAP — needs credentials MiControl cannot supply.
    Enterprise,
    /// WPA/TKIP or WEP — legacy, not configurable here.
    LegacyWpa,
    Unknown,
}

impl WifiSecurity {
    /// Label returned to (and echoed back by) the UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Wpa2Personal => "WPA2-Personal",
            Self::Wpa3Personal => "WPA3-Personal",
            Self::Enterprise => "Enterprise (802.1X)",
            Self::LegacyWpa => "WPA/TKIP (legacy)",
            Self::Unknown => "Unknown",
        }
    }

    /// Parse the label produced by [`Self::label`]. Anything unrecognised maps
    /// to `Unknown`, which makes the connect path fall back to the saved
    /// profile instead of guessing an authentication type.
    pub fn parse(label: &str) -> Self {
        let l = label.trim().to_ascii_lowercase();
        if l.starts_with("wpa3") && !l.contains("enterprise") {
            Self::Wpa3Personal
        } else if l.contains("enterprise") || l.contains("802.1x") {
            Self::Enterprise
        } else if l.starts_with("wpa2") {
            Self::Wpa2Personal
        } else if l.starts_with("wpa") {
            Self::LegacyWpa
        } else if l == "open" || l == "owe" {
            Self::Open
        } else {
            Self::Unknown
        }
    }
}

/// SSID bytes → String, clamping the declared length to the fixed array so a
/// malformed length cannot panic.
#[cfg(windows)]
fn ssid_of(ssid: &windows::Win32::NetworkManagement::WiFi::DOT11_SSID) -> String {
    let len = (ssid.uSSIDLength as usize).min(ssid.ucSSID.len());
    String::from_utf8_lossy(&ssid.ucSSID[..len]).to_string()
}

/// First wireless interface: GUID and description.
#[cfg(windows)]
fn first_interface(session: &WlanSession) -> HardwareResult<Option<(GUID, String)>> {
    use windows::Win32::NetworkManagement::WiFi::{WlanEnumInterfaces, WLAN_INTERFACE_INFO_LIST};

    let mut list_ptr: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
    // SAFETY: `list_ptr` is a valid out-pointer and the handle is open; the
    // returned allocation is owned by the guard below.
    let ret = unsafe { WlanEnumInterfaces(session.raw(), None, &mut list_ptr) };
    if ret != 0 {
        return Err(HardwareError::Wifi(format!(
            "WlanEnumInterfaces failed with error code {ret}"
        )));
    }
    // SAFETY: on success the API returns a live `WLAN_INTERFACE_INFO_LIST`.
    let list = unsafe { WlanBuf::from_raw(list_ptr) }
        .ok_or_else(|| HardwareError::Wifi("WlanEnumInterfaces returned a null list".into()))?;
    if list.dwNumberOfItems == 0 {
        return Ok(None);
    }
    let info = &list.InterfaceInfo[0];
    let name = String::from_utf16_lossy(&info.strInterfaceDescription)
        .trim_end_matches('\0')
        .to_string();
    Ok(Some((info.InterfaceGuid, name)))
}

/// SSID and signal quality of the current association, if there is one.
#[cfg(windows)]
fn current_connection(session: &WlanSession, guid: &GUID) -> Option<(String, u32)> {
    use windows::Win32::NetworkManagement::WiFi::{
        wlan_intf_opcode_current_connection, WlanQueryInterface, WLAN_CONNECTION_ATTRIBUTES,
    };

    let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
    let mut size = 0u32;
    // SAFETY: valid out-pointers; the returned blob is owned by the guard.
    let ret = unsafe {
        WlanQueryInterface(
            session.raw(),
            guid,
            wlan_intf_opcode_current_connection,
            None,
            &mut size,
            &mut data,
            None,
        )
    };
    if ret != 0 {
        return None;
    }
    // SAFETY: this opcode returns a `WLAN_CONNECTION_ATTRIBUTES`.
    let attrs = unsafe { WlanBuf::from_raw(data as *mut WLAN_CONNECTION_ATTRIBUTES) }?;
    let assoc = &attrs.wlanAssociationAttributes;
    let ssid = ssid_of(&assoc.dot11Ssid);
    (!ssid.is_empty()).then_some((ssid, assoc.wlanSignalQuality))
}

/// Map the advertised authentication algorithm to our coarse security type.
///
/// Windows reports the BSS's *default* algorithm, so a WPA2/WPA3 transition
/// network usually shows up as WPA2-Personal — transition APs accept a WPA2
/// client, which is what the generated profile asks for. Value 8 is documented
/// for both `DOT11_AUTH_ALGO_WPA3` and `DOT11_AUTH_ALGO_WPA3_ENT_192`, so a
/// 192-bit enterprise network cannot be told apart here; connecting to it ends
/// in the honest enterprise/profile error rather than a wrong-password loop.
#[cfg(windows)]
fn security_from_auth(
    net: &windows::Win32::NetworkManagement::WiFi::WLAN_AVAILABLE_NETWORK,
) -> WifiSecurity {
    if !net.bSecurityEnabled.as_bool() {
        return WifiSecurity::Open;
    }
    match net.dot11DefaultAuthAlgorithm.0 {
        // 802.11 open, WPA-none, OWE (enhanced open: no passphrase).
        1 | 5 | 10 => WifiSecurity::Open,
        // WEP shared key, WPA, WPA-PSK (TKIP).
        2..=4 => WifiSecurity::LegacyWpa,
        6 | 11 => WifiSecurity::Enterprise,
        7 => WifiSecurity::Wpa2Personal,
        8 | 9 => WifiSecurity::Wpa3Personal,
        _ => WifiSecurity::Unknown,
    }
}

/// Open the WLAN client, pick an interface and queue a scan.
///
/// Every call here returns immediately (the radio works asynchronously), so
/// this is safe to run on the blocking pool; the settle wait happens outside
/// it.
#[cfg(windows)]
fn begin_scan() -> HardwareResult<Option<(WlanSession, GUID)>> {
    use windows::Win32::NetworkManagement::WiFi::WlanScan;

    let session = WlanSession::open()?;
    let Some((guid, _name)) = first_interface(&session)? else {
        return Ok(None);
    };
    // SAFETY: the handle is open and `guid` identifies one of its interfaces.
    let ret = unsafe { WlanScan(session.raw(), &guid, None, None, None) };
    if ret != 0 {
        return Err(HardwareError::Wifi(format!(
            "WlanScan failed with error code {ret}"
        )));
    }
    Ok(Some((session, guid)))
}

/// Read the scan results for `guid` and normalise them for the UI.
#[cfg(windows)]
fn collect_networks(session: &WlanSession, guid: &GUID) -> HardwareResult<Vec<WifiNetwork>> {
    use windows::Win32::NetworkManagement::WiFi::{
        WlanGetAvailableNetworkList, WLAN_AVAILABLE_NETWORK_LIST,
    };

    // `WLAN_AVAILABLE_NETWORK` has no "currently connected" flag, so the
    // associated SSID has to be queried separately.
    let connected_ssid = current_connection(session, guid).map(|(ssid, _)| ssid);

    let mut list_ptr: *mut WLAN_AVAILABLE_NETWORK_LIST = std::ptr::null_mut();
    // SAFETY: valid out-pointer; `guid` is one of the handle's interfaces.
    let ret = unsafe { WlanGetAvailableNetworkList(session.raw(), guid, 0, None, &mut list_ptr) };
    if ret != 0 {
        return Err(HardwareError::Wifi(format!(
            "WlanGetAvailableNetworkList failed with error code {ret}"
        )));
    }
    // SAFETY: live allocation, freed by the guard.
    let list = unsafe { WlanBuf::from_raw(list_ptr) }.ok_or_else(|| {
        HardwareError::Wifi("WlanGetAvailableNetworkList returned a null list".into())
    })?;

    let mut networks: Vec<WifiNetwork> = Vec::with_capacity(list.dwNumberOfItems as usize);
    let mut hidden_count = 0u32;
    // `Network` is a C flexible array member declared as length 1 but holding
    // `dwNumberOfItems` entries; pointer arithmetic reaches the rest.
    let base = list.Network.as_ptr();
    for i in 0..list.dwNumberOfItems as usize {
        // SAFETY: `i < dwNumberOfItems`, the real length of the allocation.
        let net = unsafe { &*base.add(i) };
        let ssid = ssid_of(&net.dot11Ssid);
        if ssid.is_empty() {
            // Hidden networks show as blank entries and break React keys; one
            // placeholder is appended at the end instead.
            hidden_count += 1;
            continue;
        }
        let connected = connected_ssid.as_deref() == Some(ssid.as_str());
        networks.push(WifiNetwork {
            ssid,
            signal: net.wlanSignalQuality,
            security: security_from_auth(net).label().to_string(),
            connected,
        });
    }

    // WlanAPI returns one entry per BSS, so the same SSID from several APs
    // (2.4 GHz + 5 GHz) repeats: keep the strongest and preserve the flag.
    networks.sort_by_key(|n| std::cmp::Reverse(n.signal));
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut deduped: Vec<WifiNetwork> = Vec::with_capacity(networks.len());
    for net in networks {
        if seen.insert(net.ssid.clone()) {
            deduped.push(net);
        }
    }

    if hidden_count > 0 {
        deduped.push(WifiNetwork {
            ssid: "<Hidden Network>".to_string(),
            signal: 0,
            security: WifiSecurity::Unknown.label().to_string(),
            connected: false,
        });
    }
    Ok(deduped)
}

/// Scan for available WiFi networks using the native WlanAPI.
///
/// Uses wlanapi.dll instead of shelling out to `netsh wlan`: locale-independent
/// (structured data, not parsed text), accurate (raw signal quality and the
/// real authentication algorithm), and the settle time is an async wait so the
/// shared blocking pool is not held while the radio works.
#[cfg(windows)]
pub async fn scan_networks() -> HardwareResult<Vec<WifiNetwork>> {
    let Some((session, guid)) = crate::util::blocking::run_blocking(begin_scan).await? else {
        return Ok(Vec::new());
    };
    tokio::time::sleep(SCAN_SETTLE).await;
    crate::util::blocking::run_blocking(move || collect_networks(&session, &guid)).await
}

#[cfg(not(windows))]
pub async fn scan_networks() -> HardwareResult<Vec<WifiNetwork>> {
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
///
/// Every buffer returned by the API is owned by a `WlanBuf` guard, so it is
/// released on the error paths too (the previous version leaked all of them).
#[cfg(windows)]
pub fn get_status() -> HardwareResult<WifiStatus> {
    use windows::Win32::NetworkManagement::WiFi::{
        wlan_intf_opcode_interface_state, WlanQueryInterface,
    };

    let session = WlanSession::open()?;
    let Some((guid, interface_name)) = first_interface(&session)? else {
        return Ok(WifiStatus {
            connected: false,
            ssid: None,
            signal: None,
            interface: None,
        });
    };

    // Query interface state (enum, not locale-dependent string).
    let mut state_data: *mut std::ffi::c_void = std::ptr::null_mut();
    let mut state_size = 0u32;
    // SAFETY: valid out-pointers; the returned buffer is owned by the guard.
    let ret = unsafe {
        WlanQueryInterface(
            session.raw(),
            &guid,
            wlan_intf_opcode_interface_state,
            None,
            &mut state_size,
            &mut state_data,
            None,
        )
    };
    if ret != 0 {
        return Err(HardwareError::Wifi(format!(
            "WlanQueryInterface(state) failed with error code {ret}"
        )));
    }
    // SAFETY: the interface_state opcode returns a `WLAN_INTERFACE_STATE`
    // (an enum with a u32 representation).
    let state_buf = unsafe { WlanBuf::from_raw(state_data as *mut u32) }
        .ok_or_else(|| HardwareError::Wifi("WlanQueryInterface(state) returned null".into()))?;
    // 0=not ready, 1=connected, 2=ad_hoc_network_formed, 3=disconnecting,
    // 4=disconnected, 5=associating, 6=discovering, 7=authenticating.
    let connected = *state_buf == 1; // wlan_interface_state_connected
    drop(state_buf);

    let (ssid, signal) = if connected {
        match current_connection(&session, &guid) {
            Some((ssid, signal)) => (Some(ssid), Some(signal)),
            None => {
                log::warn!(
                    "[wifi] WlanQueryInterface(current_connection) returned no SSID — SSID unavailable"
                );
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    Ok(WifiStatus {
        connected,
        ssid,
        signal,
        interface: Some(interface_name),
    })
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
/// Budget for the `netsh wlan connect` request itself. It returns as soon as
/// the request is queued; if it has not exited in 15 s the WLAN service is
/// wedged and the tree must die instead of hanging the caller.
const NETSH_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Budget for `netsh wlan disconnect` (a local radio call, sub-second).
const NETSH_DISCONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Authentication/encryption written into the WLAN profile.
///
/// These are the only three MiControl can provision. 802.1X/EAP and legacy
/// WPA/TKIP are refused with an explicit error instead of being approximated by
/// a WPA2 profile that then fails to associate for a reason the user cannot
/// see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProfileAuth {
    Open,
    Wpa2Psk,
    Wpa3Sae,
}

impl ProfileAuth {
    fn name(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Wpa2Psk => "WPA2-PSK",
            Self::Wpa3Sae => "WPA3-SAE",
        }
    }
}

/// What the connect path should do with the profile before associating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProfilePlan {
    /// Install a profile with this auth (`overwrite` replaces an existing one).
    Install { auth: ProfileAuth, overwrite: bool },
    /// Leave the saved profile alone and only ask for the association.
    UseSavedProfile,
}

/// Decide the profile plan, or return the honest reason the request cannot be
/// honoured.
///
/// Pure (no radio, no registry) so every branch is unit-testable.
fn plan_profile(security: WifiSecurity, password: Option<&str>) -> HardwareResult<ProfilePlan> {
    match (security, password) {
        (WifiSecurity::Enterprise, _) => Err(HardwareError::Wifi(
            "This network uses 802.1X/Enterprise authentication, which MiControl cannot configure \
             (EAP methods and certificates are not supported). Sign in through Windows Settings › \
             Network & internet › Wi-Fi › Manage known networks."
                .into(),
        )),
        (WifiSecurity::LegacyWpa, _) => Err(HardwareError::Wifi(
            "This network advertises legacy WPA/TKIP or WEP, which MiControl does not configure. \
             Connect once through Windows Settings for the OS to create the profile."
                .into(),
        )),
        (WifiSecurity::Open, Some(_)) => Err(HardwareError::Wifi(
            "This network is reported as open — clear the password to connect.".into(),
        )),
        (WifiSecurity::Open, None) => Ok(ProfilePlan::Install {
            auth: ProfileAuth::Open,
            overwrite: false,
        }),
        // WPA3-SAE gets its own authentication element; anything else with a
        // passphrase is WPA2-PSK (the `Unknown` case is the historical
        // behaviour for callers that do not pass the scan result).
        (
            WifiSecurity::Wpa2Personal | WifiSecurity::Wpa3Personal | WifiSecurity::Unknown,
            Some(_),
        ) => {
            let auth = if security == WifiSecurity::Wpa3Personal {
                ProfileAuth::Wpa3Sae
            } else {
                ProfileAuth::Wpa2Psk
            };
            Ok(ProfilePlan::Install {
                auth,
                overwrite: true,
            })
        }
        // Secured, but no passphrase: use whatever profile Windows already has
        // (this is the "connect to a saved network" path). Never write an open
        // profile for a secured SSID — that would be a security downgrade.
        (WifiSecurity::Wpa2Personal | WifiSecurity::Wpa3Personal, None) => {
            Ok(ProfilePlan::UseSavedProfile)
        }
        (WifiSecurity::Unknown, None) => Ok(ProfilePlan::UseSavedProfile),
    }
}

/// Build a WLAN profile XML for `ssid`.
///
/// `auth` selects the `<authentication>`/`<encryption>` pair, and
/// `password` fills `<keyMaterial>` for the two PSK/SAE variants. The profile
/// name is the SSID — that is the key both `netsh wlan connect name=` and
/// `WlanConnect` address.
fn build_profile_xml(ssid: &str, password: Option<&str>, auth: ProfileAuth) -> String {
    let escaped_ssid = xml::escape_xml(ssid);
    let security = match auth {
        ProfileAuth::Open => r#"<security>
            <authEncryption>
                <authentication>open</authentication>
                <encryption>none</encryption>
                <useOneX>false</useOneX>
            </authEncryption>
        </security>"#
            .to_string(),
        ProfileAuth::Wpa2Psk | ProfileAuth::Wpa3Sae => {
            let (authentication, encryption) = match auth {
                ProfileAuth::Wpa3Sae => ("WPA3SAE", "AES"),
                _ => ("WPA2PSK", "AES"),
            };
            format!(
                r#"<security>
            <authEncryption>
                <authentication>{authentication}</authentication>
                <encryption>{encryption}</encryption>
                <useOneX>false</useOneX>
            </authEncryption>
            <sharedKey>
                <keyType>passPhrase</keyType>
                <protected>false</protected>
                <keyMaterial>{}</keyMaterial>
            </sharedKey>
        </security>"#,
                xml::escape_xml(password.unwrap_or(""))
            )
        }
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

/// Render a WLAN reason code with the OS's own (localised) text.
///
/// `WlanReasonCodeToString` writes into a caller-provided buffer, so nothing
/// here needs `WlanFreeMemory`.
#[cfg(windows)]
fn reason_code_text(code: u32) -> String {
    use windows::Win32::NetworkManagement::WiFi::WlanReasonCodeToString;

    let buf = [0u16; 512];
    // SAFETY: `buf` is a valid buffer whose length the binding passes to the
    // API, which only writes into it. windows-rs declares the out-buffer as
    // `&[u16]` (the C prototype is `PWSTR`), hence the shared borrow.
    let ret = unsafe { WlanReasonCodeToString(code, &buf, None) };
    if ret != 0 {
        return format!("reason {code}");
    }
    let text = String::from_utf16_lossy(&buf);
    let text = text.trim_end_matches('\0').trim();
    if text.is_empty() {
        format!("reason {code}")
    } else {
        format!("{text} (reason {code})")
    }
}

/// Install a profile from memory (no temporary file, so the passphrase never
/// reaches the disk). Per-user profile (`WLAN_PROFILE_USER`) — same scope as
/// `netsh wlan add profile` for a non-elevated caller.
///
/// On rejection the error carries the WLAN reason code *and* its text, so the
/// UI can say why (e.g. a WPA3-SAE profile on a driver that does not support
/// it) instead of a generic failure.
#[cfg(windows)]
fn set_profile_in_memory(profile_xml: &str, overwrite: bool) -> HardwareResult<()> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::NetworkManagement::WiFi::{WlanSetProfile, WLAN_PROFILE_USER};

    /// `ERROR_ALREADY_EXISTS`: the profile is already installed and `overwrite`
    /// was false — the saved one is what we want anyway.
    const ERROR_ALREADY_EXISTS: u32 = 183;

    let wide: Vec<u16> = profile_xml
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let session = WlanSession::open()?;
    let Some((guid, _name)) = first_interface(&session)? else {
        return Err(HardwareError::Wifi(
            "no wireless interface to add the profile to".into(),
        ));
    };

    let mut reason_code = 0u32;
    // SAFETY: the handle is open, `wide` is a null-terminated UTF-16 string
    // that outlives the call, and `reason_code` is a valid stack slot.
    let ret = unsafe {
        WlanSetProfile(
            session.raw(),
            &guid,
            WLAN_PROFILE_USER,
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            BOOL::from(overwrite),
            None,
            &mut reason_code,
        )
    };

    if ret == ERROR_ALREADY_EXISTS {
        log::info!("[wifi] profile already installed — keeping it (overwrite=false)");
        return Ok(());
    }
    if ret != 0 {
        return Err(HardwareError::Wifi(format!(
            "WlanSetProfile failed (error {ret}, {})",
            reason_code_text(reason_code)
        )));
    }
    log::info!("[wifi] profile installed in memory (overwrite={overwrite})");
    Ok(())
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
    // run_capture applies CREATE_NO_WINDOW and kills the tree on timeout.
    let output = process_spawn::run_capture(&mut cmd, NETSH_CONNECT_TIMEOUT)
        .map_err(|e| HardwareError::Wifi(format!("Failed to connect: {e}")))?;

    if !output.success() {
        let stderr = output.stderr_str();
        let stdout = output.stdout_str();
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
/// `security` is the label the scan reported for this SSID (see
/// [`WifiSecurity::label`]). It decides which profile is written:
///
/// - `WPA2-Personal` → `WPA2PSK`/AES,
/// - `WPA3-Personal` → `WPA3SAE`/AES,
/// - `Open` → `open`/`none`,
/// - `Enterprise (802.1X)` / `WPA/TKIP (legacy)` → refused with an explicit
///   error, because MiControl cannot supply EAP credentials or TKIP keys.
///
/// `None` keeps the historical behaviour (WPA2-PSK when a passphrase is given,
/// otherwise the saved profile) for callers that do not have a scan result.
///
/// The SSID and password are validated and XML-escaped before being
/// interpolated into the WLAN profile template to prevent XML injection.
pub fn connect(ssid: &str, password: Option<&str>, security: Option<&str>) -> HardwareResult<()> {
    // Validate the SSID before any XML construction.
    xml::validate_ssid(ssid).map_err(|e| HardwareError::Wifi(format!("Invalid SSID: {e}")))?;

    let security = security
        .map(WifiSecurity::parse)
        .unwrap_or(WifiSecurity::Unknown);

    // Validate the password if provided.
    if let Some(pwd) = password {
        xml::validate_wpa2_passphrase(pwd)
            .map_err(|e| HardwareError::Wifi(format!("Invalid WPA2 passphrase: {e}")))?;
    }

    // Refuse what cannot be provisioned *before* touching the radio, so the
    // user gets the real reason instead of a 15 s timeout.
    let plan = plan_profile(security, password)?;

    // Fast path: already associated to this network.
    if let Ok(status) = get_status() {
        if status.connected && ssid_eq(status.ssid.as_deref(), ssid) {
            log::info!("[wifi] connect '{ssid}': already associated");
            return Ok(());
        }
    }

    log::info!(
        "[wifi] connect '{ssid}' (security: {}, password supplied: {}, plan: {})",
        security.label(),
        password.is_some(),
        match plan {
            ProfilePlan::Install { auth, overwrite } =>
                format!("install {} (overwrite={overwrite})", auth.name()),
            ProfilePlan::UseSavedProfile => "use saved profile".to_string(),
        }
    );

    // 1. Materialise the profile — in memory. The passphrase used to be
    //    written in clear text to `%TEMP%\micontrol_wifi_<rand>.xml`; it never
    //    touches the disk now.
    #[cfg(windows)]
    if let ProfilePlan::Install { auth, overwrite } = plan {
        let profile_xml = build_profile_xml(ssid, password, auth);
        set_profile_in_memory(&profile_xml, overwrite)?;
    }
    #[cfg(not(windows))]
    {
        let _ = plan;
        return Err(HardwareError::NotSupported(
            "WiFi connect is Windows-only".into(),
        ));
    }

    // 2. Ask for the association. With `UseSavedProfile` the profile Windows
    //    already has is the one used; if there is none, netsh says so.
    #[cfg(windows)]
    request_association(ssid)?;

    // 3. Success means "the OS says we are associated to that SSID".
    #[cfg(windows)]
    {
        wait_for_association_with(ssid, ASSOCIATION_TIMEOUT, ASSOCIATION_POLL, get_status).map_err(
            |e| match e {
                // Name the security type we tried: a WPA3-only AP with a WPA2
                // profile, or a wrong passphrase, both end here.
                HardwareError::Wifi(msg) if security != WifiSecurity::Unknown => {
                    HardwareError::Wifi(format!("{msg} Security type used: {}.", security.label()))
                }
                other => other,
            },
        )
    }
    #[cfg(not(windows))]
    unreachable!("non-Windows returned above")
}

/// Disconnect from current WiFi network.
pub fn disconnect() -> HardwareResult<()> {
    let mut cmd = Command::new("netsh");
    cmd.args(["wlan", "disconnect"]);
    // run_capture applies CREATE_NO_WINDOW and kills the tree on timeout.
    let output = process_spawn::run_capture(&mut cmd, NETSH_DISCONNECT_TIMEOUT)
        .map_err(|e| HardwareError::Wifi(format!("Failed to disconnect: {e}")))?;

    if !output.success() {
        let stderr = output.stderr_str();
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

        let profile_xml = build_profile_xml(ssid, Some(pwd), ProfileAuth::Wpa2Psk);

        // The profile XML should not contain any raw injection tags
        assert!(!profile_xml.contains("</name><name>evil"));
        assert!(!profile_xml.contains("</keyMaterial><x>"));
        // It should contain the escaped versions
        assert!(profile_xml.contains("&lt;/name&gt;"));
        assert!(profile_xml.contains("&lt;/keyMaterial&gt;"));
    }

    #[test]
    fn profile_xml_matches_the_requested_security_type() {
        let wpa2 = build_profile_xml(
            "Home",
            Some("correct horse battery staple"),
            ProfileAuth::Wpa2Psk,
        );
        assert!(wpa2.contains("<authentication>WPA2PSK</authentication>"));
        assert!(wpa2.contains("<encryption>AES</encryption>"));
        assert!(wpa2.contains("<keyMaterial>correct horse battery staple</keyMaterial>"));
        assert!(wpa2.contains("<name>Home</name>"));

        // WPA3-SAE must not be approximated by a WPA2 profile: the whole point
        // is that the profile matches what the AP advertises.
        let wpa3 = build_profile_xml(
            "Home",
            Some("correct horse battery staple"),
            ProfileAuth::Wpa3Sae,
        );
        assert!(wpa3.contains("<authentication>WPA3SAE</authentication>"));
        assert!(wpa3.contains("<encryption>AES</encryption>"));
        assert!(wpa3.contains("<keyMaterial>correct horse battery staple</keyMaterial>"));
        assert!(!wpa3.contains("WPA2PSK"));

        let open = build_profile_xml("Cafe", None, ProfileAuth::Open);
        assert!(open.contains("<authentication>open</authentication>"));
        assert!(open.contains("<encryption>none</encryption>"));
        assert!(!open.contains("keyMaterial"));
    }

    #[test]
    fn security_labels_round_trip_through_the_ui() {
        for sec in [
            WifiSecurity::Open,
            WifiSecurity::Wpa2Personal,
            WifiSecurity::Wpa3Personal,
            WifiSecurity::Enterprise,
            WifiSecurity::LegacyWpa,
            WifiSecurity::Unknown,
        ] {
            assert_eq!(WifiSecurity::parse(sec.label()), sec, "{sec:?}");
        }
        // Labels from older builds / the mock data must not be misread as a
        // PSK type.
        assert_eq!(WifiSecurity::parse("WPA2"), WifiSecurity::Wpa2Personal);
        assert_eq!(
            WifiSecurity::parse("WPA2-Enterprise"),
            WifiSecurity::Enterprise
        );
        assert_eq!(WifiSecurity::parse(""), WifiSecurity::Unknown);
    }

    #[test]
    fn plan_refuses_what_it_cannot_provision() {
        let enterprise = plan_profile(WifiSecurity::Enterprise, Some("hunter2hunter2"))
            .expect_err("802.1X must be refused, not approximated");
        let msg = format!("{enterprise}");
        assert!(msg.contains("802.1X"), "{msg}");
        assert!(msg.contains("Windows Settings"), "{msg}");

        let legacy = plan_profile(WifiSecurity::LegacyWpa, Some("hunter2hunter2"))
            .expect_err("legacy WPA/TKIP must be refused");
        assert!(format!("{legacy}").contains("legacy"));

        let open_with_pwd = plan_profile(WifiSecurity::Open, Some("hunter2hunter2"))
            .expect_err("a password on an open network is a contradiction");
        assert!(format!("{open_with_pwd}").contains("open"));
    }

    #[test]
    fn plan_builds_the_profile_the_scan_advertised() {
        let wpa2 = plan_profile(WifiSecurity::Wpa2Personal, Some("hunter2hunter2")).unwrap();
        assert_eq!(
            wpa2,
            ProfilePlan::Install {
                auth: ProfileAuth::Wpa2Psk,
                overwrite: true
            }
        );
        let wpa3 = plan_profile(WifiSecurity::Wpa3Personal, Some("hunter2hunter2")).unwrap();
        assert_eq!(
            wpa3,
            ProfilePlan::Install {
                auth: ProfileAuth::Wpa3Sae,
                overwrite: true
            }
        );
        let open = plan_profile(WifiSecurity::Open, None).unwrap();
        assert_eq!(
            open,
            ProfilePlan::Install {
                auth: ProfileAuth::Open,
                overwrite: false
            }
        );
        // Secured without a passphrase = "connect to the saved profile"; never
        // install an open profile for a secured SSID.
        assert_eq!(
            plan_profile(WifiSecurity::Wpa2Personal, None).unwrap(),
            ProfilePlan::UseSavedProfile
        );
        assert_eq!(
            plan_profile(WifiSecurity::Wpa3Personal, None).unwrap(),
            ProfilePlan::UseSavedProfile
        );
        // Unknown security with a passphrase keeps the historical WPA2 default.
        assert_eq!(
            plan_profile(WifiSecurity::Unknown, Some("hunter2hunter2")).unwrap(),
            ProfilePlan::Install {
                auth: ProfileAuth::Wpa2Psk,
                overwrite: true
            }
        );
    }

    /// The WLAN handle crosses from `begin_scan` to `collect_networks` through
    /// the blocking pool, so it must be `Send`; the async scan future is
    /// spawned by Tauri and cannot hold a non-`Send` guard across the wait.
    #[cfg(windows)]
    #[test]
    fn wlan_session_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<WlanSession>();
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

    /// The netsh child waits are bounded: a child that sleeps past the budget
    /// must return `SpawnError::Timeout` and the whole tree must be dead
    /// (no orphan process left behind).
    #[test]
    fn netsh_child_wait_is_bounded_and_kills_tree() {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "ping -n 10 127.0.0.1"]);
        let started = Instant::now();
        let err = process_spawn::run_capture(&mut cmd, Duration::from_millis(500))
            .expect_err("sleeping child must hit the timeout");
        assert!(
            matches!(err, process_spawn::SpawnError::Timeout { .. }),
            "expected Timeout, got {err:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "timeout path took too long: {:?}",
            started.elapsed()
        );

        // No orphan ping.exe may survive the tree kill.
        std::thread::sleep(Duration::from_millis(300));
        let mut probe = Command::new("tasklist");
        probe.args(["/FI", "IMAGENAME eq ping.exe", "/NH"]);
        let out = process_spawn::run_capture(&mut probe, Duration::from_secs(15))
            .expect("tasklist probe failed");
        assert!(
            !out.stdout_str().contains("ping.exe"),
            "ping.exe survived the tree kill: {}",
            out.stdout_str()
        );
    }
}
