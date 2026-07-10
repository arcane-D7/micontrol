/**
 * Hardware-related type definitions.
 *
 * These types match the Rust structs in `src-tauri/src/hw/` and are used
 * across the frontend. Extracted from `src/hooks/useHardware.ts` (S28-003).
 */

export type PerformanceMode =
  | 'silence'
  | 'balance'
  | 'turbo'
  | 'smart'
  | 'long_battery'
  | 'decepticon'
  | 'smart_acceleration'
  | 'overdrive'
  | 'overdrive_high'
  | 'overdrive_max'
  | 'smart_adaptive';

export interface PerformanceResult {
  success: boolean;
  method: string;
  mode: PerformanceMode;
}

export interface SystemInfo {
  cpu_name: string;
  cpu_cores: number;
  cpu_threads: number;
  cpu_usage: number;
  gpu_name: string;
  gpu_usage: number;
  vram_used_mb: number;
  ram_total_gb: number;
  ram_used_gb: number;
  os_version: string;
}

export interface ProcessInfo {
  name: string;
  pid: number;
  cpu_percent: number;
  memory_mb: number;
}

export interface BatteryInfo {
  level: number;
  is_charging: boolean;
  is_plugged: boolean;
  health_percent: number;
  cycle_count: number;
  designed_capacity_mwh: number;
  full_capacity_mwh: number;
  manufacturer: string;
  device_name: string;
  serial_number: string;
  chemistry: string;
  temperature_celsius: number | null;
  time_remaining_minutes: number | null;
  /** Estimated minutes until fully charged. Null when not charging. */
  time_to_full_minutes: number | null;
  /** Positive = charge rate in mW; negative = discharge rate in mW; 0 = unknown */
  charge_rate_mw: number;
  /** Current battery voltage in millivolts. 0 = unavailable. */
  voltage_mv: number;
  /** AC adapter input power in milliwatts. Null when not plugged in or when the
   * ECRAM register offset for this laptop has not yet been confirmed. */
  ac_input_power_mw: number | null;
}

export interface AiBrightnessConfig {
  enabled: boolean;
  min_brightness: number; // 5-80
  max_brightness: number; // 20-100
  sensitivity: number; // 10-200
  smoothing: number; // 0-90
}

export interface DisplayInfo {
  brightness: number;
  hdr_enabled: boolean;
  refresh_rate_hz: number;
  /** All Hz values supported at the current resolution. */
  available_refresh_rates: number[];
  /** True when current Hz is the max — Windows 11 DRR activates automatically. */
  dynamic_refresh_rate_capable: boolean;
  /** Intel PSR2 DRRS — driver-level automatic 60↔120 Hz switching. */
  adaptive_refresh_rate: boolean;
  ai_brightness: boolean;
  ai_brightness_config: AiBrightnessConfig;
  /** Current ambient illuminance from the light sensor (lux). Null when unavailable. */
  ambient_lux: number | null;
}

export interface FanInfo {
  mode: 'auto' | 'fixed' | 'off';
  speed_rpm: number;
  speed_percent: number;
  gpu_temp_celsius: number;
  cpu_temp_celsius: number;
  /** System package power from RAPL (\\Power Meter(_Total)\\Power). Null for ~1.5 s after launch. */
  tdp_watts: number | null;
}

export interface AiPerfLogEntry {
  /** ISO-8601 timestamp */
  ts: string;
  /** "smart" | "smart_acceleration" */
  mode: string;
  cpu_temp: number;
  gpu_temp: number;
  tdp_watts: number | null;
  cpu_pct: number;
  gpu_pct: number;
  note?: string | null;
}

export interface TouchpadInfo {
  sensitivity: 'low' | 'medium' | 'high' | 'very_high';
  haptics_enabled: boolean;
  haptics_intensity: 'low' | 'medium' | 'high';
  gesture_screenshot: boolean;
  trackpad_repress: boolean;
  edge_slide: boolean;
}

export interface BiosInfo {
  version: string;
  release_date: string;
  manufacturer: string;
  serial_number: string;
}

export interface XiaomiDriverInfo {
  published_name: string;
  original_name: string;
  provider: string;
  version_string: string;
  class_name: string;
  signer: string;
}

export interface UpdateStatus {
  bios: BiosInfo;
  xiaomi_drivers: XiaomiDriverInfo[];
  last_xpm_scan: string | null;
  xpm_driver_cache: Record<string, string>;
  xpm_installed: boolean;
  xpm_version: string | null;
  xpm_path: string | null;
}

// ── Hardware Discovery (Phase 10) ────────────────────────────────────────────

export interface MissingDriver {
  name: string;
  description: string;
  bundled_inf: string | null;
}

/** Derived capability flags — what the hardware can actually do. */
export interface HardwareCapabilities {
  has_vhf_performance: boolean;
  has_touchpad_hid: boolean;
  has_touchscreen: boolean;
  has_stylus: boolean;
  has_igcl: boolean;
  has_iot_charging: boolean;
  has_mi_registry: boolean;
}

export interface HardwareProfile {
  discovered_at: number;
  device_model: string | null;
  vhf_device_path: string | null;
  touchpad_hid_path: string | null;
  touchscreen_hid_path: string | null;
  stylus_hid_path: string | null;
  iot_pipe_path: string | null;
  iot_service_name: string | null;
  igcl_dll_path: string | null;
  mi_registry_present: boolean;
  missing_drivers: MissingDriver[];
  capabilities: HardwareCapabilities;
}

export type IotRegionName = 'ERAM' | 'SMA2' | 'IOT_STATUS' | 'IOT_SENSORS';

export interface EramMap {
  misc0: number;
  misc1: number;
  control_flags_1b: number;
  ai_limit_enabled: boolean;
  long_battery_limit_enabled: boolean;
  cpu_temp_c: number;
  fan_rpm: number;
  fan2_rpm: number;
  cpu_power_w: number;
  smart_mode_type: number;
  smart_mode_data: number;
  smart_mode_profile: string | null;
  qfan_mode: number;
  perf_profile: number;
  tdp_w: number;
  ac_flags: number;
  ac_connected: boolean;
  ac_adapter_w: number;
  battery_current_ma: number;
  battery_capacity_mah: number;
  battery_voltage_mv: number;
  charge_threshold_pct: number;
  battery_temp_c: number;
  display_brightness_level: number;
  keyboard_backlight_level: number;
  raw_hex: string;
}

export interface AudioVolumeResult {
  success: boolean;
  volume: number;
  muted: boolean;
}

// ── IoTService IPC types ───────────────────────────────────────────────────

/** IoT device bind status (whether a Xiaomi account is linked). */
export interface BindStatusInfo {
  bound: boolean;
  uid: number | null;
}

/** WiFi connection status from the IoT device. */
export interface WiFiStatusInfo {
  wifi_status: number;
  ssid: string | null;
}

/** A provisioned WiFi network item on the IoT device. */
export interface WiFiItemInfo {
  ssid: string;
  connected: boolean;
  enabled: boolean;
}

/**
 * Composite IoT device info returned by `get_iot_device_info`.
 *
 * Each field is independently queried; `null` means the query failed or the
 * pipe was unavailable.
 */
export interface IotDeviceInfo {
  pipe_available: boolean;
  model: string | null;
  fw_version: string | null;
  bind_status: BindStatusInfo | null;
  device_id: number | null;
  device_status: string | null;
  wifi_status: WiFiStatusInfo | null;
  wifi_network_count: number | null;
}

/**
 * Consolidated WiFi list returned by `get_iot_wifi_list`.
 *
 * Combines connection status, count, and all provisioned networks in one call.
 */
export interface IotWifiList {
  status: WiFiStatusInfo | null;
  count: number;
  networks: WiFiItemInfo[];
}

/** Power event types monitored by IoTService. */
export type PowerEventType =
  | 'ac_dc_source_change'
  | 'battery_percentage_change'
  | 'monitor_power_change'
  | 'power_saving_change'
  | 'power_scheme_change'
  | 'away_mode_change'
  | 'lid_switch_change'
  | 'console_display_change'
  | 'user_presence_change';

/** A power event with optional context fields. */
export interface PowerEvent {
  event_type: PowerEventType;
  ac_online?: boolean;
  battery_percent?: number;
  monitor_on?: boolean;
  battery_saver_on?: boolean;
  power_scheme?: string;
  away_mode?: boolean;
  lid_open?: boolean;
  display_on?: boolean;
  user_present?: boolean;
}

/** Laptop lifecycle status values. */
export type LaptopStatus = 'win_ready' | 'suspending' | 'shutting';

/**
 * Unified IoT event for `iot_notify_event`.
 *
 * Tagged union — the `kind` field determines which variant is present.
 */
export type IotEvent =
  | { kind: 'power'; event: PowerEvent }
  | { kind: 'ec'; event_func: number; event_value: number }
  | { kind: 'laptop_status'; status: LaptopStatus };

// ── WMAA / WMI MiInterface types (elevated bridge) ──────────────────────────
//
// These types match the Rust structs in `src-tauri/src/hw/wmi_ec.rs`.
// All WMAA commands require admin privileges and are dispatched through
// the elevated bridge.

/** Raw WMAA response buffer (30 bytes from WMI output). */
export interface WmaaResponse {
  sger: number;
  futr: number;
  frd0: number;
  frd1: number;
  frd2: number;
  frd3: number;
  raw: number[];
}

/** EC performance mode IDs (FUN3 for FUN2=0x0800 write commands). */
export type EcPerformanceMode =
  'Performance' | 'Balanced' | 'Quiet' | 'SuperQuiet' | 'UltraPerformance' | 'Extreme';

/** Numeric performance mode values for `wmi_ec_set_performance_mode`. */
export const EC_PERF_MODE = {
  Performance: 5,
  Balanced: 6,
  Quiet: 7,
  SuperQuiet: 8,
  UltraPerformance: 9,
  Extreme: 10,
} as const;

/** Sensor data read from the EC via WMI in a single call. */
export interface EcSensorData {
  battery_health: number;
  adapter_power: number;
  mi_usage_type: number;
  wmid_type: number;
  lid_open_type: number;
  removable_type: number;
  current_mode: number;
}

// ── HQWmiCommonInterface (BIOS control via WMI) ────────────────────────────
//
// These types match the Rust structs in `src-tauri/src/hw/hq_wmi.rs`.
// All methods take a String `req` parameter and return a String `ret`.

/** Response from an HQWmiCommonInterface method call. */
export interface HqWmiResponse {
  /** The method that was called. */
  method: string;
  /** The request string sent to the method. */
  req: string;
  /** The return string from the method. */
  ret: string;
  /** Whether the call succeeded. */
  success: boolean;
}

// ── Thermal Zone (ACPI temperature) ────────────────────────────────────────
//
// These types match the Rust structs in `src-tauri/src/hw/thermal.rs`.

/** Thermal zone information from ACPI. */
export interface ThermalZoneInfo {
  /** ACPI instance name (e.g., "ACPI\ThermalZone\TZ00_0"). */
  instance_name: string;
  /** Current temperature in Celsius. */
  current_temp_celsius: number;
  /** Critical trip point in Celsius (system shutdown threshold). */
  critical_trip_celsius: number | null;
  /** Whether this thermal zone is active. */
  active: boolean;
}
