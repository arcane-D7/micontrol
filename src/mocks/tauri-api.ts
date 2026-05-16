/**
 * Mock implementation of @tauri-apps/api/core for browser-only dev mode.
 * Activated via `npm run dev:mock` (vite.config.mock.ts aliases this module).
 */

import type {
  AiBrightnessConfig,
  BatteryInfo,
  BiosInfo,
  HardwareCapabilities,
  HardwareProfile,
  PerformanceMode,
  SystemInfo,
  UpdateStatus,
  XiaomiDriverInfo,
} from "../hooks/useHardware";

// ── Persisted mock state (simulates registry / IPC) ─────────────────────────

let _performanceMode: PerformanceMode = "balance";
let _chargingThreshold = 80;
let _brightness = 72;
let _hdr = false;
let _aiBrightness = true;
let _aiBrightnessConfig: AiBrightnessConfig = {
  enabled: true,
  min_brightness: 10,
  max_brightness: 100,
  sensitivity: 100,
  smoothing: 30,
};
let _fanMode: "auto" | "fixed" | "off" = "auto";
let _fanSpeedPercent = 0;
let _touchpadSensitivity: "low" | "medium" | "high" = "high";
let _touchpadHaptics = true;
let _autostart = false;

// ── Mock data ────────────────────────────────────────────────────────────────

const SYSTEM_INFO: SystemInfo = {
  cpu_name: "12th Gen Intel Core i7-12700H",
  cpu_cores: 14,
  cpu_threads: 20,
  cpu_usage: 23.4,
  gpu_name: "NVIDIA GeForce RTX 3060 Laptop GPU",
  ram_total_gb: 16,
  ram_used_gb: 8.3,
  os_version: "Windows 11 Home 23H2 (26100.3915)",
};

const BATTERY_INFO: BatteryInfo = {
  level: 78,
  is_charging: false,
  is_plugged: true,
  health_percent: 94,
  cycle_count: 412,
  designed_capacity_mah: 70000,
  full_capacity_mah: 65800,
  manufacturer: "Xiaomi",
  device_name: "LP-L9N80V",
  temperature_celsius: 32.5,
  time_remaining_minutes: 187,
};

const HARDWARE_CAPABILITIES: HardwareCapabilities = {
  has_vhf_performance: true,
  has_touchpad_hid: true,
  has_touchscreen: false,
  has_stylus: false,
  has_igcl: true,
  has_iot_charging: true,
  has_mi_registry: true,
};

const HARDWARE_PROFILE: HardwareProfile = {
  discovered_at: Math.floor(Date.now() / 1000) - 60 * 60 * 2,  // Unix seconds (2 h ago)
  device_model: "Xiaomi Mi Notebook Pro X 15",
  vhf_device_path: "\\\\?\\HID#{0CC99493-EB87-54F5-BB10-C0D5EA4A4F4C}#5&3e2b1f9c&0&0000#{0CC99493-EB87-54F5-BB10-C0D5EA4A4F4C}",
  touchpad_hid_path: "\\\\?\\HID#VID_2575&PID_0204&Col04#7&1a2b3c4d&0&0003#{4d1e55b2-f16f-11cf-88cb-001111000030}",
  touchscreen_hid_path: null,
  stylus_hid_path: null,
  iot_pipe_path: "\\\\.\\pipe\\LOCAL\\IoTService_IPC_Broker",
  iot_service_name: "IoTSvc",
  igcl_dll_path: "C:\\Windows\\System32\\ControlLib.dll",
  mi_registry_present: true,
  missing_drivers: [],
  capabilities: HARDWARE_CAPABILITIES,
};

const BIOS_INFO: BiosInfo = {
  version: "XMACM100P0106",
  release_date: "20240815000000.000000+000",
  manufacturer: "Xiaomi",
  serial_number: "XMKB2305A0000BX",
};

const XIAOMI_DRIVERS: XiaomiDriverInfo[] = [
  {
    published_name: "oem71.inf",
    original_name: "virtualcontrolhid.inf",
    provider: "Xiaomi Inc.",
    version_string: "12/12/2023 1.2.0.8",
    class_name: "HIDClass",
    signer: "Microsoft Windows Hardware Compatibility Publisher",
  },
  {
    published_name: "oem169.inf",
    original_name: "iotdriver.inf",
    provider: "Xiaomi Inc.",
    version_string: "11/30/2023 2.0.1.12",
    class_name: "System",
    signer: "Microsoft Windows Hardware Compatibility Publisher",
  },
];

const UPDATE_STATUS: UpdateStatus = {
  bios: BIOS_INFO,
  xiaomi_drivers: XIAOMI_DRIVERS,
  last_xpm_scan: new Date(Date.now() - 1000 * 60 * 30).toISOString(),
  xpm_driver_cache: {
    VirtualControlHID: "1.2.0.8",
    IoTDriver: "2.0.1.12",
  },
  xpm_installed: true,
  xpm_version: "5.8.0.57",
  xpm_path: "C:\\Program Files\\MI\\XiaomiPCManager\\5.8.0.57",
};

// ── Simulated sensor drift for the overview ──────────────────────────────────

let _tick = 0;
function drift(base: number, amp: number): number {
  _tick++;
  return base + Math.sin(_tick * 0.2) * amp;
}

// ── invoke mock ──────────────────────────────────────────────────────────────

type InvokeArgs = Record<string, unknown>;

export async function invoke<T>(command: string, args?: InvokeArgs): Promise<T> {
  // Simulate realistic async latency
  await new Promise((r) => setTimeout(r, 30 + Math.random() * 60));

  switch (command) {
    // ── System info ────────────────────────────────────────────────────────
    case "get_system_info":
      return {
        ...SYSTEM_INFO,
        cpu_usage: Math.round(drift(23, 18) * 10) / 10,
        ram_used_gb: Math.round(drift(8.3, 0.8) * 10) / 10,
      } as T;

    // ── Battery ────────────────────────────────────────────────────────────
    case "get_battery_info":
      return {
        ...BATTERY_INFO,
        level: Math.max(5, Math.min(100, BATTERY_INFO.level)),
      } as T;

    // ── Display ────────────────────────────────────────────────────────────
    case "get_display_info":
      return {
        brightness: _brightness,
        hdr_enabled: _hdr,
        refresh_rate_hz: 120,
        ai_brightness: _aiBrightness,
        ai_brightness_config: { ..._aiBrightnessConfig },
      } as T;

    case "set_brightness":
      _brightness = (args?.level as number) ?? _brightness;
      return undefined as T;

    case "set_hdr":
      _hdr = (args?.enabled as boolean) ?? _hdr;
      return undefined as T;

    case "set_ai_brightness":
      _aiBrightness = (args?.enabled as boolean) ?? _aiBrightness;
      _aiBrightnessConfig = { ..._aiBrightnessConfig, enabled: _aiBrightness };
      return undefined as T;

    case "get_ai_brightness_config":
      return { ..._aiBrightnessConfig } as T;

    case "set_ai_brightness_config": {
      const cfg = args?.config as AiBrightnessConfig;
      if (cfg) {
        _aiBrightnessConfig = { ...cfg };
        _aiBrightness = cfg.enabled;
      }
      return undefined as T;
    }

    // ── Fan ────────────────────────────────────────────────────────────────
    case "get_fan_info":
      return {
        mode: _fanMode,
        speed_rpm: _fanMode === "off" ? 0 : Math.round(drift(_fanMode === "fixed" ? _fanSpeedPercent * 55 : 2100, 180)),
        speed_percent: _fanSpeedPercent,
        gpu_temp_celsius: Math.round(drift(52, 6) * 10) / 10,
      } as T;

    case "set_fan_mode":
      _fanMode = (args?.mode as "auto" | "fixed" | "off") ?? _fanMode;
      _fanSpeedPercent = (args?.speed_percent as number) ?? _fanSpeedPercent;
      return undefined as T;

    // ── Touchpad ───────────────────────────────────────────────────────────
    case "get_touchpad_info":
      return {
        sensitivity: _touchpadSensitivity,
        haptics_enabled: _touchpadHaptics,
      } as T;

    case "set_touchpad_sensitivity":
      _touchpadSensitivity = (args?.sensitivity as "low" | "medium" | "high") ?? _touchpadSensitivity;
      return undefined as T;

    case "set_touchpad_haptics":
      _touchpadHaptics = (args?.enabled as boolean) ?? _touchpadHaptics;
      return undefined as T;

    // ── Performance ────────────────────────────────────────────────────────
    case "get_performance_mode":
      return _performanceMode as T;

    case "set_performance_mode":
      _performanceMode = (args?.mode as PerformanceMode) ?? _performanceMode;
      return { mode: _performanceMode, hw_value: 1 } as T;

    // ── Charging ───────────────────────────────────────────────────────────
    case "get_charging_threshold":
      return _chargingThreshold as T;

    case "set_charging_threshold":
      _chargingThreshold = (args?.threshold as number) ?? _chargingThreshold;
      return undefined as T;

    // ── Startup ────────────────────────────────────────────────────────────
    case "get_autostart":
      return _autostart as T;

    case "set_autostart":
      _autostart = (args?.enabled as boolean) ?? _autostart;
      return undefined as T;

    // ── Updates ────────────────────────────────────────────────────────────
    case "get_update_status":
      return UPDATE_STATUS as T;

    case "trigger_driver_scan":
      return UPDATE_STATUS as T;

    // ── Hardware discovery ─────────────────────────────────────────────────
    case "get_hardware_profile":
      return HARDWARE_PROFILE as T;

    case "run_hardware_discovery":
      await new Promise((r) => setTimeout(r, 1200)); // simulate discovery time
      return HARDWARE_PROFILE as T;

    case "install_driver":
      await new Promise((r) => setTimeout(r, 800));
      return "Driver installed successfully (mock)" as T;

    // ── Tray ───────────────────────────────────────────────────────────────
    case "open_main_window":
      return undefined as T;

    default:
      console.warn(`[mock] Unknown command: ${command}`, args);
      return undefined as T;
  }
}

// Re-export the rest of the tauri core API as no-ops so imports don't break
export const transformCallback = () => 0;
export const convertFileSrc = (p: string) => p;
export const Channel = class {};
export const PluginListener = class {};
export const addPluginListener = async () => ({ unregister: async () => {} });
export const checkPermissions = async () => ({});
export const requestPermissions = async () => ({});
