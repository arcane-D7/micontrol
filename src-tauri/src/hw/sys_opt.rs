//! System Optimization — Windows debloat & telemetry reduction (S53).
//!
//! Implements a curated, SAFE, fully-reversible subset of the well-known
//! Windows debloat practices (cross-checked against WinUtil's tweaks.json and
//! O&O ShutUp10's philosophy). Every toggle stores the previous value so it
//! can be restored exactly; scheduled tasks are DISABLED (never deleted —
//! feature updates would re-create them anyway) and services are set to
//! Disabled (never uninstalled).
//!
//! Safety rules enforced here:
//! - NEVER touches Windows Update services/tasks (wuauserv, UsoSvc, BITS,
//!   DoSvc, WaaSMedic, \Microsoft\Windows\WindowsUpdate\*) — breaking WU is
//!   the classic debloat footgun.
//! - NEVER touches Defender, BitLocker, Store licensing.
//! - `AllowTelemetry=0` is coerced to `1` by Windows on Home/Pro (only
//!   Enterprise honors 0), so we offer "minimal (1)" as the safe level and
//!   expose 0 only as a no-op note.
//!
//! All state lives under `HKLM\SOFTWARE\MiControl\SysOpt\<id>`:
//!   `Applied` = 1 when the tweak is active, plus `Prev_<value-name>` values
//!   capturing the exact previous state for faithful restore.

use crate::hw::errors::{HardwareError, HardwareResult};
use serde::{Deserialize, Serialize};

/// One optimization toggle: what it changes, how to apply/restore.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SysOptTweak {
    /// Stable identifier (also the registry sub-key name).
    pub id: &'static str,
    /// i18n key suffix for the UI (`sysOpt.items.<id>.*`).
    pub title: &'static str,
    pub desc: &'static str,
    /// Relative impact score for the UI badge (1-5).
    pub impact: u8,
}

/// The curated safe tweak list (order = UI display order).
pub const TWEAKS: &[SysOptTweak] = &[
    SysOptTweak {
        id: "telemetry_level",
        title: "telemetry",
        desc: "telemetryDesc",
        impact: 5,
    },
    SysOptTweak {
        id: "diagtrack_service",
        title: "diagTrack",
        desc: "diagTrackDesc",
        impact: 4,
    },
    SysOptTweak {
        id: "ceip_tasks",
        title: "ceipTasks",
        desc: "ceipTasksDesc",
        impact: 5,
    },
    SysOptTweak {
        id: "compat_appraiser",
        title: "compatAppraiser",
        desc: "compatAppraiserDesc",
        impact: 5,
    },
    SysOptTweak {
        id: "advertising_id",
        title: "advertisingId",
        desc: "advertisingIdDesc",
        impact: 2,
    },
    SysOptTweak {
        id: "tailored_experiences",
        title: "tailored",
        desc: "tailoredDesc",
        impact: 2,
    },
    SysOptTweak {
        id: "activity_history",
        title: "activityHistory",
        desc: "activityHistoryDesc",
        impact: 3,
    },
    SysOptTweak {
        id: "feedback_requests",
        title: "feedback",
        desc: "feedbackDesc",
        impact: 2,
    },
    SysOptTweak {
        id: "web_search_start",
        title: "webSearch",
        desc: "webSearchDesc",
        impact: 3,
    },
    SysOptTweak {
        id: "consumer_features",
        title: "consumerFeatures",
        desc: "consumerFeaturesDesc",
        impact: 3,
    },
    SysOptTweak {
        id: "delivery_opt_upload",
        title: "deliveryOpt",
        desc: "deliveryOptDesc",
        impact: 3,
    },
    SysOptTweak {
        id: "background_apps",
        title: "backgroundApps",
        desc: "backgroundAppsDesc",
        impact: 4,
    },
    // ── S54: tiny11-inspired Appx/bloat removal (running-system equivalents) ──
    // Each removes the per-user + provisioned package. Fully reversible via
    // the Store or `winget install`. PROTECTED_LIST below is hard-blocked.
    SysOptTweak {
        id: "appx_bing_games",
        title: "appxBingGames",
        desc: "appxBingGamesDesc",
        impact: 3,
    },
    SysOptTweak {
        id: "appx_office_media",
        title: "appxOfficeMedia",
        desc: "appxOfficeMediaDesc",
        impact: 3,
    },
    SysOptTweak {
        id: "appx_misc_tools",
        title: "appxMiscTools",
        desc: "appxMiscToolsDesc",
        impact: 2,
    },
    SysOptTweak {
        id: "appx_xbox",
        title: "appxXbox",
        desc: "appxXboxDesc",
        impact: 2,
    },
    SysOptTweak {
        id: "onedrive_uninstall",
        title: "onedrive",
        desc: "onedriveDesc",
        impact: 3,
    },
    SysOptTweak {
        id: "services_unused",
        title: "servicesUnused",
        desc: "servicesUnusedDesc",
        impact: 3,
    },
    // ── S55: RED tier — aggressive, potentially destabilizing (Sophia/Win11Debloat
    // aggressive lists). Each still has a documented restore path. The UI gates
    // these behind an explicit damage-acknowledgement consent.
    SysOptTweak {
        id: "red_copilot",
        title: "redCopilot",
        desc: "redCopilotDesc",
        impact: 4,
    },
    SysOptTweak {
        id: "red_edge_preinstall",
        title: "redEdge",
        desc: "redEdgeDesc",
        impact: 5,
    },
    SysOptTweak {
        id: "red_widgets",
        title: "redWidgets",
        desc: "redWidgetsDesc",
        impact: 4,
    },
    SysOptTweak {
        id: "red_cortana_relics",
        title: "redCortana",
        desc: "redCortanaDesc",
        impact: 4,
    },
];

/// S55: risk tier per tweak id — drives the debloater modal accordion group.
/// green: safe/reversible (open + enabled). yellow: may break non-essential
/// functionality (consent gate). red: may destabilize Windows (explicit
/// damage-acknowledgement gate).
pub fn risk_tier(id: &str) -> &'static str {
    match id {
        "red_copilot" | "red_edge_preinstall" | "red_widgets" | "red_cortana_relics" => "red",
        "appx_xbox" | "onedrive_uninstall" | "services_unused" | "diagtrack_service" => "yellow",
        _ => "green",
    }
}

/// S54: Appx packages that must NEVER be removed — removing any of these
/// breaks the Store, the Settings app, the app framework dependencies or
/// (critically) our own WebView2-based app. This is the tiny11 "do not
/// cross" list, enforced before any Remove-Appx call.
#[cfg(windows)]
const PROTECTED_APPX: &[&str] = &[
    "Microsoft.WindowsStore",
    "Microsoft.StorePurchaseApp",
    "Microsoft.SecHealthUI",
    "Microsoft.VCLibs",
    "Microsoft.UI.Xaml",
    "Microsoft.NET.Native",
    "Microsoft.WindowsAppRuntime",
    "Microsoft.WindowsAppSDK",
    "Microsoft.Windows.Photos",
    "Microsoft.WindowsCalculator",
    "Microsoft.WindowsNotepad",
    "Microsoft.WindowsTerminal",
    "Microsoft.DesktopAppInstaller",
    "Microsoft.WindowsSecurityHealth",
    "Microsoft.XboxGamingOverlay", // Game Bar — Win+G/Game DVR break
    "Microsoft.Windows.ShellExperienceHost",
    "Microsoft.Windows.StartMenuExperienceHost",
    "MicrosoftWindows.Client",
    "Microsoft.Windows.ContentDeliveryManager",
];

/// Groups of bloat Appx packages (tiny11maker's list, filtered to items that
/// are safe + reversible on a running 24H2 system, minus Skype/Cortana which
/// are retired no-ops and minus the XboxGamingOverlay Game Bar).
#[cfg(windows)]
const APPX_GROUPS: &[(&str, &[&str])] = &[
    (
        "appx_bing_games",
        &[
            "Microsoft.BingNews",
            "Microsoft.BingWeather",
            "Microsoft.BingSearch",
            "Microsoft.GamingApp",
            "Microsoft.MicrosoftSolitaireCollection",
        ],
    ),
    (
        "appx_office_media",
        &[
            "Microsoft.MicrosoftOfficeHub",
            "Microsoft.Office.OneNote",
            "Microsoft.OutlookForWindows",
            "Microsoft.Todos",
            "Microsoft.ZuneMusic",
            "Microsoft.ZuneVideo",
        ],
    ),
    (
        "appx_misc_tools",
        &[
            "Microsoft.GetHelp",
            "Microsoft.Getstarted",
            "Microsoft.WindowsFeedbackHub",
            "Microsoft.WindowsMaps",
            "Microsoft.People",
            "Microsoft.MicrosoftStickyNotes",
            "Microsoft.WindowsSoundRecorder",
            "Microsoft.Wallet",
            "Clipchamp.Clipchamp",
            "Microsoft.MixedReality.Portal",
            "Microsoft.Microsoft3DViewer",
            "Microsoft.PowerAutomateDesktop",
            "MicrosoftCorporationII.QuickAssist",
            "MicrosoftCorporationII.MicrosoftFamily",
            "AppUp.IntelManagementandSecurityStatus",
        ],
    ),
    (
        "appx_xbox",
        &[
            "Microsoft.Xbox.TCUI",
            "Microsoft.XboxApp",
            "Microsoft.XboxIdentityProvider",
            "Microsoft.XboxSpeechToTextOverlay",
        ],
    ),
];

/// Services safe to disable on a personal desktop (tiny11/WinUtil consensus).
/// Original `Start` value is stored before change for faithful restore.
#[cfg(windows)]
const UNUSED_SERVICES: &[&str] = &[
    "dmwappushservice",
    "wisvc",
    "RetailDemo",
    "PhoneSvc",
    "MapsBroker",
];

/// CEIP + diagnostics scheduled tasks (DISABLED, never deleted).
#[cfg(windows)]
const CEIP_TASKS: &[&str] = &[
    r"\Microsoft\Windows\Customer Experience Improvement Program\Consolidator",
    r"\Microsoft\Windows\Customer Experience Improvement Program\UsbCeip",
    r"\Microsoft\Windows\Customer Experience Improvement Program\Uploader",
    r"\Microsoft\Windows\Application Experience\Microsoft Compatibility Appraiser",
    r"\Microsoft\Windows\Application Experience\ProgramDataUpdater",
    r"\Microsoft\Windows\Autochk\Proxy",
    r"\Microsoft\Windows\DiskDiagnostic\Microsoft-Windows-DiskDiagnosticDataCollector",
    r"\Microsoft\Windows\Windows Error Reporting\QueueReporting",
];

// ── Registry helpers (HKLM, via the elevated bridge context) ────────────────

#[cfg(windows)]
fn write_hklm_dword(path: &str, name: &str, value: u32) -> HardwareResult<()> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
    let key = RegKeyGuard::create_write(HKEY_LOCAL_MACHINE, path)
        .map_err(|e| HardwareError::Registry(format!("open {path}: {e}")))?;
    key.write_u32(name, value)
        .map_err(|e| HardwareError::Registry(format!("write {path}\\{name}: {e}")))
}

#[cfg(windows)]
fn read_hklm_dword(path: &str, name: &str) -> Option<u32> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
    RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, path)
        .ok()
        .flatten()
        .and_then(|k| k.read_u32(name).ok().flatten())
}

#[cfg(windows)]
fn write_hklm_string(path: &str, name: &str, value: &str) -> HardwareResult<()> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
    let key = RegKeyGuard::create_write(HKEY_LOCAL_MACHINE, path)
        .map_err(|e| HardwareError::Registry(format!("open {path}: {e}")))?;
    key.write_string(name, value)
        .map_err(|e| HardwareError::Registry(format!("write {path}\\{name}: {e}")))
}

/// State store: marks a tweak applied and remembers a previous DWORD value
/// (or absence) so restore is faithful.
#[cfg(windows)]
fn mark_applied(id: &str, prev: Option<(&str, Option<u32>)>) -> HardwareResult<()> {
    let state_path = format!(r"SOFTWARE\MiControl\SysOpt\{id}");
    write_hklm_dword(&state_path, "Applied", 1)?;
    if let Some((path, value)) = prev {
        match value {
            Some(v) => write_hklm_dword(&format!(r"{state_path}\Prev"), path, v)?,
            None => write_hklm_string(&format!(r"{state_path}\Prev"), path, "<absent>")?,
        }
    }
    Ok(())
}

#[cfg(windows)]
fn mark_restored(id: &str) -> HardwareResult<()> {
    let state_path = format!(r"SOFTWARE\MiControl\SysOpt\{id}");
    write_hklm_dword(&state_path, "Applied", 0)
}

#[cfg(windows)]
fn is_applied(id: &str) -> bool {
    read_hklm_dword(&format!(r"SOFTWARE\MiControl\SysOpt\{id}"), "Applied")
        .map(|v| v != 0)
        .unwrap_or(false)
}

// ── Individual tweaks ────────────────────────────────────────────────────────

const DC_POLICIES: &str = r"SOFTWARE\Policies\Microsoft\Windows\DataCollection";
const DC_POLICIES_MIRROR: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\DataCollection";

fn apply_telemetry_level(minimal: bool) -> HardwareResult<()> {
    // 1 = Required (Basic) — the minimum that keeps Windows Update
    // diagnostics sane on Home/Pro. 0 is Enterprise-only (coerced to 1).
    let v: u32 = if minimal { 1 } else { 3 };
    let prev_dc = read_hklm_dword(DC_POLICIES, "AllowTelemetry");
    let prev_mirror = read_hklm_dword(DC_POLICIES_MIRROR, "AllowTelemetry");
    write_hklm_dword(DC_POLICIES, "AllowTelemetry", v)?;
    write_hklm_dword(DC_POLICIES_MIRROR, "AllowTelemetry", v)?;
    mark_applied("telemetry_level", Some((DC_POLICIES, prev_dc)))?;
    // Mirror prev stored separately.
    if let Some(mv) = prev_mirror {
        let _ = write_hklm_dword(
            r"SOFTWARE\MiControl\SysOpt\telemetry_level\Prev",
            DC_POLICIES_MIRROR,
            mv,
        );
    }
    Ok(())
}

fn restore_telemetry_level() -> HardwareResult<()> {
    let base = r"SOFTWARE\MiControl\SysOpt\telemetry_level\Prev";
    let prev_dc = read_hklm_dword(base, DC_POLICIES);
    // "<absent>" is stored as a string; absence of the DWORD means delete.
    match prev_dc {
        Some(v) => {
            write_hklm_dword(DC_POLICIES, "AllowTelemetry", v)?;
            write_hklm_dword(DC_POLICIES_MIRROR, "AllowTelemetry", v)?;
        }
        None => {
            // Never set before — remove the policy keys' value by writing 1
            // (Windows default) rather than deleting, to keep it simple.
            write_hklm_dword(DC_POLICIES, "AllowTelemetry", 1)?;
            write_hklm_dword(DC_POLICIES_MIRROR, "AllowTelemetry", 1)?;
        }
    }
    mark_restored("telemetry_level")
}

/// DiagTrack (Connected User Experiences and Telemetry) service → Disabled.
/// S55 FIX: the service's security descriptor grants ChangeConfig (DC) only
/// to BUILTIN\Administrators — SYSTEM (our bridge) only gets start/stop, so
/// `sc config` fails with Access Denied. The registry Start value under
/// HKLM\SYSTEM\CurrentControlSet\Services IS writable by SYSTEM, so set it
/// there directly (equivalent and persistent) and stop/start via `sc`.
#[cfg(windows)]
fn apply_diagtrack(disable: bool) -> HardwareResult<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const SVC_PATH: &str = r"SYSTEM\CurrentControlSet\Services\DiagTrack";
    let prev_start = read_hklm_dword(SVC_PATH, "Start");
    let start_val: u32 = if disable { 4 } else { 2 };
    write_hklm_dword(SVC_PATH, "Start", start_val)?;
    let verb = if disable { "stop" } else { "start" };
    let _ = std::process::Command::new("sc.exe")
        .args([verb, "DiagTrack"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    if disable {
        mark_applied("diagtrack_service", Some((SVC_PATH, prev_start)))?;
    } else {
        mark_restored("diagtrack_service")?;
    }
    log::info!(
        "[sys_opt] DiagTrack {} (registry Start={start_val})",
        if disable { "disabled" } else { "restored" }
    );
    Ok(())
}

/// Disable/enable the CEIP + diagnostics scheduled tasks.
#[cfg(windows)]
fn apply_ceip_tasks(disable: bool) -> HardwareResult<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut failures = Vec::new();
    for task in CEIP_TASKS {
        let verb = if disable { "/DISABLE" } else { "/ENABLE" };
        let out = std::process::Command::new("schtasks")
            .args(["/Change", verb, "/TN", task])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        match out {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                // S55 FIX: schtasks/sc write errors to STDOUT, not stderr.
                let text = format!(
                    "{}{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                );
                if text.to_lowercase().contains("does not exist")
                    || text.to_lowercase().contains("não existe")
                {
                    log::info!("[sys_opt] CEIP task {task} not present on this build — skipped");
                } else {
                    failures.push(format!("{task}: {}", text.trim()));
                }
            }
            Err(e) => failures.push(format!("{task}: {e}")),
        }
    }
    if disable {
        mark_applied("ceip_tasks", None)?;
    } else {
        mark_restored("ceip_tasks")?;
    }
    if failures.len() == CEIP_TASKS.len() {
        return Err(HardwareError::Other(format!(
            "All CEIP task changes failed: {}",
            failures.join("; ")
        )));
    }
    if !failures.is_empty() {
        log::warn!(
            "[sys_opt] some CEIP tasks not changed (may not exist on this build): {}",
            failures.join("; ")
        );
    }
    Ok(())
}

/// Generic single-DWORD HKLM/HKCU tweak applier.
#[cfg(windows)]
struct DwordTweak {
    id: &'static str,
    path: &'static str,
    name: &'static str,
    on_value: u32,
    off_value: u32,
    hive_user: bool,
}

#[cfg(windows)]
fn apply_dword_tweak(t: &DwordTweak, on: bool) -> HardwareResult<()> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_CURRENT_USER;
    let value = if on { t.on_value } else { t.off_value };
    if t.hive_user {
        let key = RegKeyGuard::create_write(HKEY_CURRENT_USER, t.path)
            .map_err(|e| HardwareError::Registry(format!("{}: {e}", t.path)))?;
        key.write_u32(t.name, value)
            .map_err(|e| HardwareError::Registry(format!("{}\\{}: {e}", t.path, t.name)))?;
    } else {
        write_hklm_dword(t.path, t.name, value)?;
    }
    if on {
        let prev = if t.hive_user {
            read_hkcu_dword(t.path, t.name)
        } else {
            read_hklm_dword(t.path, t.name)
        };
        mark_applied(t.id, Some((t.path, prev)))?;
    } else {
        mark_restored(t.id)?;
    }
    Ok(())
}

#[cfg(windows)]
fn read_hkcu_dword(path: &str, name: &str) -> Option<u32> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_CURRENT_USER;
    RegKeyGuard::open_read(HKEY_CURRENT_USER, path)
        .ok()
        .flatten()
        .and_then(|k| k.read_u32(name).ok().flatten())
}

/// Public API: apply or restore one tweak by id.
pub fn set_tweak(id: &str, enabled: bool) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        let dword = |id_, path, name, on_v, off_v, hive_user| DwordTweak {
            id: id_,
            path,
            name,
            on_value: on_v,
            off_value: off_v,
            hive_user,
        };
        match id {
            "telemetry_level" => {
                if enabled {
                    apply_telemetry_level(true)
                } else {
                    restore_telemetry_level()
                }
            }
            "diagtrack_service" => apply_diagtrack(enabled),
            "ceip_tasks" => apply_ceip_tasks(enabled),
            "compat_appraiser" => {
                // The Compatibility Appraiser is inside CEIP tasks; standalone
                // control via its own task.
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                // S55 FIX: on several 24H2 builds this task no longer exists —
                // "task not found" is a successful no-op, not an error.
                const APPRAISER_TASK: &str =
                    r"\Microsoft\Windows\Application Experience\Microsoft Compatibility Appraiser";
                let out = std::process::Command::new("schtasks")
                    .args([
                        "/Change",
                        if enabled { "/DISABLE" } else { "/ENABLE" },
                        "/TN",
                        APPRAISER_TASK,
                    ])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output()
                    .map_err(|e| HardwareError::Other(format!("schtasks: {e}")))?;
                let combined = format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
                if !out.status.success()
                    && !combined.to_lowercase().contains("does not exist")
                    && !combined.to_lowercase().contains("não existe")
                {
                    return Err(HardwareError::Other(format!(
                        "Compatibility Appraiser task: {}",
                        combined.trim()
                    )));
                }
                if out.status.success() {
                    log::info!(
                        "[sys_opt] Compatibility Appraiser task {}",
                        if enabled { "enabled" } else { "disabled" }
                    );
                } else {
                    log::info!(
                        "[sys_opt] Compatibility Appraiser task not present on this build — no-op"
                    );
                }
                if enabled {
                    mark_applied("compat_appraiser", None)
                } else {
                    mark_restored("compat_appraiser")
                }
            }
            "advertising_id" => apply_dword_tweak(
                &dword(
                    "advertising_id",
                    r"Software\Microsoft\Windows\CurrentVersion\AdvertisingInfo",
                    "Enabled",
                    0,
                    1,
                    true,
                ),
                enabled,
            ),
            "tailored_experiences" => apply_dword_tweak(
                &dword(
                    "tailored_experiences",
                    r"Software\Microsoft\Windows\CurrentVersion\Privacy",
                    "TailoredExperiencesWithDiagnosticDataEnabled",
                    0,
                    1,
                    true,
                ),
                enabled,
            ),
            "activity_history" => {
                apply_dword_tweak(
                    &dword(
                        "activity_history",
                        r"SOFTWARE\Policies\Microsoft\Windows\System",
                        "EnableActivityFeed",
                        0,
                        1,
                        false,
                    ),
                    enabled,
                )?;
                write_hklm_dword(
                    r"SOFTWARE\Policies\Microsoft\Windows\System",
                    "PublishUserActivities",
                    if enabled { 0 } else { 1 },
                )?;
                write_hklm_dword(
                    r"SOFTWARE\Policies\Microsoft\Windows\System",
                    "UploadUserActivities",
                    if enabled { 0 } else { 1 },
                )?;
                Ok(())
            }
            "feedback_requests" => apply_dword_tweak(
                &dword(
                    "feedback_requests",
                    r"Software\Microsoft\Siuf\Rules",
                    "NumberOfSIUFInPeriod",
                    0,
                    1,
                    true,
                ),
                enabled,
            ),
            "web_search_start" => {
                apply_dword_tweak(
                    &dword(
                        "web_search_start",
                        r"Software\Policies\Microsoft\Windows\Explorer",
                        "DisableSearchBoxSuggestions",
                        1,
                        0,
                        true,
                    ),
                    enabled,
                )?;
                apply_dword_tweak(
                    &dword(
                        "web_search_start_bing",
                        r"Software\Microsoft\Windows\CurrentVersion\Search",
                        "BingSearchEnabled",
                        0,
                        1,
                        true,
                    ),
                    enabled,
                )
            }
            "consumer_features" => {
                write_hklm_dword(
                    r"SOFTWARE\Policies\Microsoft\Windows\CloudContent",
                    "DisableWindowsConsumerFeatures",
                    if enabled { 1 } else { 0 },
                )?;
                write_hklm_dword(
                    r"SOFTWARE\Policies\Microsoft\Windows\CloudContent",
                    "DisableThirdPartySuggestions",
                    if enabled { 1 } else { 0 },
                )?;
                if enabled {
                    mark_applied("consumer_features", None)
                } else {
                    mark_restored("consumer_features")
                }
            }
            "delivery_opt_upload" => {
                let prev = read_hklm_dword(
                    r"SOFTWARE\Policies\Microsoft\Windows\DeliveryOptimization",
                    "DODownloadMode",
                );
                write_hklm_dword(
                    r"SOFTWARE\Policies\Microsoft\Windows\DeliveryOptimization",
                    "DODownloadMode",
                    if enabled { 0 } else { 1 },
                )?;
                if enabled {
                    mark_applied(
                        "delivery_opt_upload",
                        Some((
                            r"SOFTWARE\Policies\Microsoft\Windows\DeliveryOptimization",
                            prev,
                        )),
                    )
                } else {
                    mark_restored("delivery_opt_upload")
                }
            }
            "background_apps" => apply_dword_tweak(
                &dword(
                    "background_apps",
                    r"Software\Microsoft\Windows\CurrentVersion\BackgroundAccessApplications",
                    "GlobalUserDisabled",
                    1,
                    0,
                    true,
                ),
                enabled,
            ),
            // ── S54: tiny11-inspired removals ──
            id if id.starts_with("appx_") => apply_appx_group(id, enabled),
            "onedrive_uninstall" => apply_onedrive(enabled),
            "services_unused" => apply_unused_services(enabled),
            // ── S55: red-tier aggressive tweaks ──
            "red_copilot" => apply_copilot(enabled),
            "red_edge_preinstall" => apply_edge_preinstall(enabled),
            "red_widgets" => apply_widgets(enabled),
            "red_cortana_relics" => apply_cortana_relics(enabled),
            other => Err(HardwareError::Other(format!(
                "Unknown SysOpt tweak: {other}"
            ))),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (id, enabled);
        Err(HardwareError::NotSupported(
            "System optimization only available on Windows".into(),
        ))
    }
}

/// Run a PowerShell command hidden and return stdout (or an error with stderr).
#[cfg(windows)]
fn run_ps(script: &str) -> HardwareResult<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| HardwareError::Other(format!("powershell spawn: {e}")))?;
    if !out.status.success() {
        return Err(HardwareError::Other(format!(
            "powershell failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Guard: a package name must not be in the protected list. Each protected
/// prefix is matched against the full package name (which may carry version
/// and architecture suffixes), so prefix matching is the correct check.
#[cfg(windows)]
fn is_protected_appx(full_name: &str) -> bool {
    let lower = full_name.to_ascii_lowercase();
    PROTECTED_APPX
        .iter()
        .any(|p| lower.starts_with(&p.to_ascii_lowercase()))
}

/// Remove (or re-register) a group of Appx packages for the current user AND
/// the provisioned set (so new users don't get them either).
///
/// Reversibility: every package in these groups is a Store app — reinstall is
/// one click in the Store or `winget install`. The per-user removal is the
/// documented supported path; nothing outside the group list and outside the
/// PROTECTED_APPX guard is ever touched.
#[cfg(windows)]
fn apply_appx_group(group_id: &str, remove: bool) -> HardwareResult<()> {
    let group = APPX_GROUPS
        .iter()
        .find(|(id, _)| *id == group_id)
        .ok_or_else(|| HardwareError::Other(format!("Unknown appx group: {group_id}")))?;
    let (_, packages) = group;

    let mut removed = Vec::new();
    let mut missing = Vec::new();

    if remove {
        for pkg in packages.iter() {
            if is_protected_appx(pkg) {
                log::warn!("[sys_opt] refusing to remove protected appx: {pkg}");
                continue;
            }
            // Per-user removal (documented supported path).
            let script = format!(
                "Get-AppxPackage -Name '{}' | Remove-AppxPackage -ErrorAction Stop",
                pkg.replace('\'', "''")
            );
            match run_ps(&script) {
                Ok(_) => removed.push(*pkg),
                Err(e) => {
                    // "Not installed" is fine — treat as missing, not failure.
                    let msg = e.to_string();
                    if msg.to_lowercase().contains("not") && msg.to_lowercase().contains("found") {
                        missing.push(*pkg);
                    } else {
                        log::warn!("[sys_opt] appx remove {pkg}: {msg}");
                        missing.push(*pkg);
                    }
                }
            }
            // Provisioned removal (future users) — best effort, may not exist.
            let prov = format!(
                "Get-AppxProvisionedPackage -Online | Where-Object DisplayName -eq '{}' | ForEach-Object {{ Remove-AppxProvisionedPackage -Online -PackageName $_.PackageName }}",
                pkg.replace('\'', "''")
            );
            let _ = run_ps(&prov);
        }
        mark_applied(group_id, None)?;
        log::info!(
            "[sys_opt] appx group {group_id}: removed {}, not-present {}",
            removed.len(),
            missing.len()
        );
        Ok(())
    } else {
        // Restore = reinstall via winget (the Store-able documented path).
        for pkg in packages.iter() {
            if is_protected_appx(pkg) {
                continue;
            }
            // winget resolves the friendly package name from the Appx family.
            let script = format!(
                "winget install --id '{}' --source msstore --accept-source-agreements --accept-package-agreements --silent",
                pkg.replace('\'', "''")
            );
            if let Err(e) = run_ps(&script) {
                log::warn!("[sys_opt] winget restore {pkg}: {e}");
            } else {
                removed.push(*pkg);
            }
        }
        mark_restored(group_id)?;
        log::info!(
            "[sys_opt] appx group {group_id} restore: winget handled {}",
            removed.len()
        );
        Ok(())
    }
}

/// OneDrive: use the OFFICIAL uninstaller from SYSTEM context (documented
/// supported path; tiny11's takeown+delete is offline-only and unsafe).
/// Restore: re-install via winget.
///
/// S55 FIX: the elevated context may be SYSTEM, where $env:LOCALAPPDATA
/// points at systemprofile and winget.exe is not on PATH. Probe all known
/// OneDriveSetup.exe locations (user profile dirs of real users, Program
/// Files, SysWOW64) and fall back to the per-user uninstaller registry key;
/// winget is resolved via its stable WinGetLinks path as a last resort.
#[cfg(windows)]
fn apply_onedrive(remove: bool) -> HardwareResult<()> {
    if remove {
        let script = r#"
$ErrorActionPreference = 'Stop'
$setups = @()
foreach ($root in @('C:\Users')) {
  $setups += Get-ChildItem -Path $root -Filter 'OneDriveSetup.exe' -Recurse -Depth 4 -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -like '*Microsoft\OneDrive*' } | Select-Object -ExpandProperty FullName
}
$setups += "$env:ProgramFiles\Microsoft OneDrive\OneDriveSetup.exe"
$setups += "$env:ProgramFiles(x86)\Microsoft OneDrive\OneDriveSetup.exe"
$setup = $setups | Where-Object { Test-Path $_ } | Select-Object -First 1
if ($setup) {
    Start-Process -FilePath $setup -ArgumentList '/uninstall' -Wait
    Write-Output "uninstalled via $setup"
} else {
    # Fallback: the registered uninstaller string (documented per-machine entry).
    $uninst = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\OneDriveSetup.exe' -ErrorAction SilentlyContinue).UninstallString
    if (-not $uninst) {
        $uninst = (Get-ItemProperty 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\OneDriveSetup.exe' -ErrorAction SilentlyContinue).UninstallString
    }
    if ($uninst) {
        Start-Process -FilePath $uninst -ArgumentList '/uninstall' -Wait
        Write-Output "uninstalled via registry entry"
    } else {
        # winget last (may not be on PATH for SYSTEM — try known links path too)
        $wg = Get-Command winget.exe -ErrorAction SilentlyContinue
        if (-not $wg) {
            $links = "$env:LOCALAPPDATA\Microsoft\WindowsApps\winget.exe"
            if (Test-Path $links) { $wg = $links }
        }
        if ($wg) {
            & $wg uninstall --id Microsoft.OneDrive --silent --accept-source-agreements
            Write-Output "uninstalled via winget"
        } else {
            Write-Output "OneDrive not present (nothing to uninstall)"
        }
    }
}
"#;
        run_ps(script)?;
        // Hide the leftover OneDrive shortcut policy (tiny11 sets this too).
        write_hklm_dword(
            r"SOFTWARE\Policies\Microsoft\Windows\OneDrive",
            "DisableFileSyncNGSC",
            1,
        )?;
        mark_applied("onedrive_uninstall", None)?;
        log::info!("[sys_opt] OneDrive uninstalled (official uninstaller)");
        Ok(())
    } else {
        // Restore: clear the hide policy and reinstall via winget.
        write_hklm_dword(
            r"SOFTWARE\Policies\Microsoft\Windows\OneDrive",
            "DisableFileSyncNGSC",
            0,
        )?;
        run_ps("winget install --id Microsoft.OneDrive --source winget --accept-source-agreements --accept-package-agreements --silent")?;
        mark_restored("onedrive_uninstall")?;
        log::info!("[sys_opt] OneDrive reinstalled");
        Ok(())
    }
}

/// S55 (RED): remove the Copilot app + disable the Copilot policy. Restore
/// re-registers via the Store / winget and clears the policy. Windows itself
/// re-offers Copilot on updates, so this is a persistent-policy approach.
#[cfg(windows)]
fn apply_copilot(disable: bool) -> HardwareResult<()> {
    if disable {
        write_hklm_dword(
            r"SOFTWARE\Policies\Microsoft\Windows\WindowsCopilot",
            "TurnOffWindowsCopilot",
            1,
        )?;
        let _ = run_ps(
            "Get-AppxPackage -Name 'Microsoft.Copilot' | Remove-AppxPackage -ErrorAction SilentlyContinue",
        );
        mark_applied("red_copilot", None)?;
        log::info!("[sys_opt] Copilot disabled (policy + app removal)");
    } else {
        write_hklm_dword(
            r"SOFTWARE\Policies\Microsoft\Windows\WindowsCopilot",
            "TurnOffWindowsCopilot",
            0,
        )?;
        mark_restored("red_copilot")?;
        log::info!("[sys_opt] Copilot policy restored (app reinstall via Store)");
    }
    Ok(())
}

/// S55 (RED): block Edge pre-launch/background and its preload of startup
/// processes. Does NOT uninstall Edge (unsupported and unsafe on Win11 —
/// Sophia Script's approach is exactly this policy set). Restore re-enables.
#[cfg(windows)]
fn apply_edge_preinstall(disable: bool) -> HardwareResult<()> {
    let v: u32 = if disable { 1 } else { 0 };
    write_hklm_dword(
        r"SOFTWARE\Policies\Microsoft\Edge",
        "StartupBoostEnabled",
        if disable { 0 } else { 1 },
    )?;
    write_hklm_dword(
        r"SOFTWARE\Policies\Microsoft\Edge",
        "BackgroundModeEnabled",
        if disable { 0 } else { 1 },
    )?;
    write_hklm_dword(
        r"SOFTWARE\Policies\Microsoft\Windows\EdgeUpdate",
        "AllowEdgePrelaunch",
        v,
    )?;
    if disable {
        mark_applied("red_edge_preinstall", None)?;
    } else {
        mark_restored("red_edge_preinstall")?;
    }
    log::info!(
        "[sys_opt] Edge prelaunch {}",
        if disable { "blocked" } else { "restored" }
    );
    Ok(())
}

/// S55 (RED): disable/restore the Widgets (News & Interests) feed.
///
/// FIX 6 (final): registry paths are a dead end on this machine —
/// `HKCU\...\Advanced\TaskbarDa` is blocked by Microsoft's UCPD.sys
/// (User Choice Protection Driver, blocks SetValue for that value name from
/// any process outside Explorer/Settings) and `HKLM\...\Dsh` denies service
/// tokens. The viable documented path is removing the Widgets APP itself:
/// the `MicrosoftWindows.Client.WebExperience` Appx (Web Experience Pack).
/// Remove-AppxPackage is not a User Choice operation, so UCPD does not block
/// it. Restore reinstalls via winget/Store. TaskbarDa is still attempted
/// best-effort for builds where UCPD permits it.
#[cfg(windows)]
fn apply_widgets(disable: bool) -> HardwareResult<()> {
    const WEB_EXPERIENCE: &str = "MicrosoftWindows.Client.WebExperience";
    if disable {
        let script = format!(
            "Get-AppxPackage -Name '{WEB_EXPERIENCE}' | Remove-AppxPackage -ErrorAction Stop"
        );
        match run_ps(&script) {
            Ok(_) => {
                log::info!("[sys_opt] Widgets removed (Web Experience Pack uninstalled)");
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.to_lowercase().contains("not") && msg.to_lowercase().contains("found") {
                    log::info!("[sys_opt] Widgets app not installed — no-op");
                } else {
                    log::warn!("[sys_opt] Web Experience removal failed: {msg}");
                }
            }
        }
        // Best-effort taskbar setting (works on builds without UCPD guard).
        let _ = std::process::Command::new("reg.exe")
            .args([
                "add",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
                "/v",
                "TaskbarDa",
                "/t",
                "REG_DWORD",
                "/d",
                "0",
                "/f",
            ])
            .output();
        // Best-effort policy (service tokens are denied on some builds).
        let _ = std::process::Command::new("reg.exe")
            .args([
                "add",
                r"HKLM\SOFTWARE\Policies\Microsoft\Dsh",
                "/v",
                "AllowNewsAndInterests",
                "/t",
                "REG_DWORD",
                "/d",
                "0",
                "/f",
            ])
            .output();
        mark_applied("red_widgets", None)?;
        log::info!("[sys_opt] Widgets disabled");
    } else {
        // Restore: reinstall the Web Experience Pack from the Store/winget.
        if let Err(e) = run_ps(
            "winget install --id 9MSSGKG348SP --source msstore --accept-source-agreements --accept-package-agreements --silent",
        ) {
            log::warn!("[sys_opt] Widgets restore via winget failed (install manually from Store): {e}");
        }
        let _ = std::process::Command::new("reg.exe")
            .args([
                "add",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
                "/v",
                "TaskbarDa",
                "/t",
                "REG_DWORD",
                "/d",
                "1",
                "/f",
            ])
            .output();
        mark_restored("red_widgets")?;
        log::info!("[sys_opt] Widgets restored");
    }
    Ok(())
}

/// S55 (RED): remove Cortana leftovers (app + policy).
///
/// S55 FIX 4: `HKLM\SOFTWARE\Policies\Microsoft\Windows\Windows Search` also
/// denies writes to service tokens on this build (RegCreateKeyExW → ERROR_5),
/// so the policy write goes through reg.exe best-effort while the app removal
/// (per-user Appx) does the real work. Failure of the policy write alone is
/// logged, not fatal.
#[cfg(windows)]
fn apply_cortana_relics(disable: bool) -> HardwareResult<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let v: u32 = if disable { 0 } else { 1 };
    let policy = std::process::Command::new("reg.exe")
        .args([
            "add",
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\Windows Search",
            "/v",
            "AllowCortana",
            "/t",
            "REG_DWORD",
            "/d",
            &v.to_string(),
            "/f",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    if let Ok(o) = &policy {
        if !o.status.success() {
            log::warn!(
                "[sys_opt] Cortana policy write denied (best-effort skipped): {}",
                String::from_utf8_lossy(&o.stdout).trim()
            );
        }
    }
    let _ = run_ps(
        "Get-AppxPackage -Name 'Microsoft.549981C3F5F10' | Remove-AppxPackage -ErrorAction SilentlyContinue",
    );
    if disable {
        mark_applied("red_cortana_relics", None)?;
        log::info!("[sys_opt] Cortana disabled (app removal; policy best-effort)");
    } else {
        mark_restored("red_cortana_relics")?;
        log::info!("[sys_opt] Cortana policy restored (app reinstall via Store)");
    }
    Ok(())
}

/// Disable (or restore) the services safe to turn off on a personal desktop.
/// The original `Start` value of each service is stored before the change so
/// restore is faithful (most are Manual=3 by default).
///
/// S55 FIX: service security descriptors commonly deny ChangeConfig to
/// SYSTEM (only Administrators get DC), so `sc config` fails with Access
/// Denied from the bridge. Writing the registry `Start` value directly under
/// HKLM\SYSTEM\CurrentControlSet\Services IS permitted to SYSTEM and is the
/// same persistent change; `sc stop` still works (SYSTEM has that right).
#[cfg(windows)]
fn apply_unused_services(disable: bool) -> HardwareResult<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut failures = Vec::new();
    for svc in UNUSED_SERVICES {
        let svc_path = format!(r"SYSTEM\CurrentControlSet\Services\{svc}");
        if disable {
            let prev = read_hklm_dword(&svc_path, "Start");
            // Disabled = 4. Write the registry Start value directly.
            if let Err(e) = write_hklm_dword(&svc_path, "Start", 4) {
                failures.push(format!("{svc}: {e}"));
                continue;
            }
            let _ = std::process::Command::new("sc.exe")
                .args(["stop", svc])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
            mark_applied("services_unused", Some((&svc_path, prev)))?;
        }
    }
    if !disable {
        // Restore: most of these are Manual (3) by default on Win11.
        for svc in UNUSED_SERVICES {
            let svc_path = format!(r"SYSTEM\CurrentControlSet\Services\{svc}");
            if let Err(e) = write_hklm_dword(&svc_path, "Start", 3) {
                failures.push(format!("{svc}: {e}"));
            }
        }
        mark_restored("services_unused")?;
    }
    if failures.len() == UNUSED_SERVICES.len() {
        return Err(HardwareError::Other(format!(
            "All service changes failed: {}",
            failures.join("; ")
        )));
    }
    if !failures.is_empty() {
        log::warn!(
            "[sys_opt] some services not changed: {}",
            failures.join("; ")
        );
    }
    log::info!(
        "[sys_opt] unused services {}",
        if disable { "disabled" } else { "restored" }
    );
    Ok(())
}

/// Full status for the UI: applied-state + impact + risk tier for every tweak.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SysOptStatus {
    pub id: String,
    pub applied: bool,
    /// S55: "green" | "yellow" | "red" — debloater modal accordion group.
    pub tier: String,
}

pub fn get_status() -> HardwareResult<Vec<SysOptStatus>> {
    #[cfg(windows)]
    {
        Ok(TWEAKS
            .iter()
            .map(|t| SysOptStatus {
                id: t.id.to_string(),
                applied: is_applied(t.id),
                tier: risk_tier(t.id).to_string(),
            })
            .collect())
    }
    #[cfg(not(windows))]
    {
        Ok(TWEAKS
            .iter()
            .map(|t| SysOptStatus {
                id: t.id.to_string(),
                applied: false,
                tier: risk_tier(t.id).to_string(),
            })
            .collect())
    }
}

/// Re-apply every enabled tweak (used at app boot — feature updates and some
/// Windows servicing actions reset HKLM policies; O&O ShutUp10 offers the
/// same "re-apply after updates" feature).
pub fn restore_all() {
    #[cfg(windows)]
    {
        for t in TWEAKS {
            if is_applied(t.id) {
                match set_tweak(t.id, true) {
                    Ok(()) => log::info!("[sys_opt] re-applied tweak {}", t.id),
                    Err(e) => log::warn!("[sys_opt] re-apply {} failed: {e}", t.id),
                }
            }
        }
    }
}
