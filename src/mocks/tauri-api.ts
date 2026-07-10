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
} from '../types/hardware';

// ── Persisted mock state (simulates registry / IPC) ─────────────────────────

let _performanceMode: PerformanceMode = 'balance';
let _chargingThreshold = 80;
let _brightness = 72;
let _hdr = false;
let _adaptiveRefreshRate = true;
let _aiBrightness = true;
let _aiBrightnessConfig: AiBrightnessConfig = {
  enabled: true,
  min_brightness: 10,
  max_brightness: 100,
  sensitivity: 100,
  smoothing: 30,
};
let _fanMode: 'auto' | 'fixed' | 'off' = 'auto';
let _fanSpeedPercent = 0;
let _touchpadSensitivity: 'low' | 'medium' | 'high' = 'high';
let _touchpadHaptics = true;
let _touchpadHapticsIntensity: 'low' | 'medium' | 'high' = 'medium';
let _touchpadGestureScreenshot = false;
let _touchpadRepress = false;
let _touchpadEdgeSlide = false;
let _autostart = false;

// ── Mock data ────────────────────────────────────────────────────────────────

const SYSTEM_INFO: SystemInfo = {
  cpu_name: 'Intel Core Ultra X7 358H',
  cpu_cores: 16,
  cpu_threads: 16,
  cpu_usage: 18.5,
  gpu_name: 'Intel Arc B390 Graphics',
  gpu_usage: 9.2,
  vram_used_mb: 1024,
  ram_total_gb: 32,
  ram_used_gb: 14.6,
  os_version: 'Windows 11 Home 24H2 (26100.3323)',
};

const BATTERY_INFO: BatteryInfo = {
  level: 78,
  is_charging: false,
  is_plugged: true,
  health_percent: 98,
  cycle_count: 42,
  designed_capacity_mwh: 80000,
  full_capacity_mwh: 78400,
  manufacturer: 'Xiaomi',
  device_name: 'X14Pro2024',
  serial_number: 'BATT-X14P-001234',
  chemistry: 'Li-ion',
  temperature_celsius: 32.5,
  time_remaining_minutes: 187,
  time_to_full_minutes: null,
  charge_rate_mw: 0,
  ac_input_power_mw: null,
  voltage_mv: 15600,
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
  discovered_at: Math.floor(Date.now() / 1000) - 60 * 60 * 2, // Unix seconds (2 h ago)
  device_model: 'Xiaomi Notebook Pro 14 2026',
  vhf_device_path:
    '\\\\?\\HID#{0CC99493-EB87-54F5-BB10-C0D5EA4A4F4C}#5&3e2b1f9c&0&0000#{0CC99493-EB87-54F5-BB10-C0D5EA4A4F4C}',
  touchpad_hid_path:
    '\\\\?\\HID#VID_2575&PID_0204&Col04#7&1a2b3c4d&0&0003#{4d1e55b2-f16f-11cf-88cb-001111000030}',
  touchscreen_hid_path: null,
  stylus_hid_path: null,
  iot_pipe_path: '\\\\.\\pipe\\LOCAL\\IoTService_IPC_Broker',
  iot_service_name: 'IoTSvc',
  igcl_dll_path: 'C:\\Windows\\System32\\ControlLib.dll',
  mi_registry_present: true,
  missing_drivers: [],
  capabilities: HARDWARE_CAPABILITIES,
};

const BIOS_INFO: BiosInfo = {
  version: 'XMACM100P0106',
  release_date: '20240815000000.000000+000',
  manufacturer: 'Xiaomi',
  serial_number: 'XMKB2305A0000BX',
};

const XIAOMI_DRIVERS: XiaomiDriverInfo[] = [
  {
    published_name: 'oem71.inf',
    original_name: 'virtualcontrolhid.inf',
    provider: 'Xiaomi Inc.',
    version_string: '12/12/2023 1.2.0.8',
    class_name: 'HIDClass',
    signer: 'Microsoft Windows Hardware Compatibility Publisher',
  },
  {
    published_name: 'oem169.inf',
    original_name: 'iotdriver.inf',
    provider: 'Xiaomi Inc.',
    version_string: '11/30/2023 2.0.1.12',
    class_name: 'System',
    signer: 'Microsoft Windows Hardware Compatibility Publisher',
  },
];

const UPDATE_STATUS: UpdateStatus = {
  bios: BIOS_INFO,
  xiaomi_drivers: XIAOMI_DRIVERS,
  last_xpm_scan: new Date(Date.now() - 1000 * 60 * 30).toISOString(),
  xpm_driver_cache: {
    VirtualControlHID: '1.2.0.8',
    IoTDriver: '2.0.1.12',
  },
  xpm_installed: true,
  xpm_version: '5.8.0.57',
  xpm_path: 'C:\\Program Files\\MI\\XiaomiPCManager\\5.8.0.57',
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
    case 'get_system_info':
      return {
        ...SYSTEM_INFO,
        cpu_usage: Math.round(drift(23, 18) * 10) / 10,
        gpu_usage: Math.round(drift(12, 10) * 10) / 10,
        ram_used_gb: Math.round(drift(8.3, 0.8) * 10) / 10,
      } as T;

    // ── Battery ────────────────────────────────────────────────────────────
    case 'get_battery_info':
      return {
        ...BATTERY_INFO,
        level: Math.max(5, Math.min(100, BATTERY_INFO.level)),
      } as T;

    // ── Display ────────────────────────────────────────────────────────────
    case 'get_display_info':
      return {
        brightness: _brightness,
        hdr_enabled: _hdr,
        refresh_rate_hz: 120,
        available_refresh_rates: [60, 120],
        dynamic_refresh_rate_capable: true,
        adaptive_refresh_rate: _adaptiveRefreshRate,
        ai_brightness: _aiBrightness,
        ai_brightness_config: { ..._aiBrightnessConfig },
        ambient_lux: 342.0,
      } as T;

    case 'set_brightness':
      _brightness = (args?.level as number) ?? _brightness;
      return undefined as T;

    case 'get_available_refresh_rates':
      return [60, 120] as T;

    case 'set_refresh_rate':
      return undefined as T;

    case 'get_process_list':
      return [
        { name: 'chrome', pid: 1234, cpu_percent: drift(8, 6), memory_mb: 512 },
        { name: 'Code', pid: 2345, cpu_percent: drift(3, 3), memory_mb: 256 },
        { name: 'Tauri', pid: 3456, cpu_percent: drift(1, 1), memory_mb: 128 },
        { name: 'explorer', pid: 456, cpu_percent: drift(0.5, 0.5), memory_mb: 64 },
        { name: 'System', pid: 4, cpu_percent: drift(0.3, 0.3), memory_mb: 32 },
      ] as T;

    case 'set_adaptive_refresh_rate':
      _adaptiveRefreshRate = (args?.enabled as boolean) ?? _adaptiveRefreshRate;
      return undefined as T;

    case 'set_hdr':
      _hdr = (args?.enabled as boolean) ?? _hdr;
      return undefined as T;

    case 'set_ai_brightness':
      _aiBrightness = (args?.enabled as boolean) ?? _aiBrightness;
      _aiBrightnessConfig = { ..._aiBrightnessConfig, enabled: _aiBrightness };
      return undefined as T;

    case 'get_ai_brightness_config':
      return { ..._aiBrightnessConfig } as T;

    case 'set_ai_brightness_config': {
      const cfg = args?.config as AiBrightnessConfig;
      if (cfg) {
        _aiBrightnessConfig = { ...cfg };
        _aiBrightness = cfg.enabled;
      }
      return undefined as T;
    }

    // ── Fan ────────────────────────────────────────────────────────────────
    case 'get_fan_info':
      return {
        mode: _fanMode,
        speed_rpm:
          _fanMode === 'off'
            ? 0
            : Math.round(drift(_fanMode === 'fixed' ? _fanSpeedPercent * 55 : 2100, 180)),
        speed_percent: _fanSpeedPercent,
        gpu_temp_celsius: Math.round(drift(52, 6) * 10) / 10,
      } as T;

    case 'set_fan_mode':
      _fanMode = (args?.mode as 'auto' | 'fixed' | 'off') ?? _fanMode;
      _fanSpeedPercent = (args?.speed_percent as number) ?? _fanSpeedPercent;
      return undefined as T;

    // ── Touchpad ───────────────────────────────────────────────────────────
    case 'get_touchpad_info':
      return {
        sensitivity: _touchpadSensitivity,
        haptics_enabled: _touchpadHaptics,
        haptics_intensity: _touchpadHapticsIntensity,
        gesture_screenshot: _touchpadGestureScreenshot,
        trackpad_repress: _touchpadRepress,
        edge_slide: _touchpadEdgeSlide,
      } as T;

    case 'set_touchpad_sensitivity':
      _touchpadSensitivity =
        (args?.sensitivity as 'low' | 'medium' | 'high') ?? _touchpadSensitivity;
      return undefined as T;

    case 'set_touchpad_haptics':
      _touchpadHaptics = (args?.enabled as boolean) ?? _touchpadHaptics;
      return undefined as T;

    case 'set_touchpad_haptics_intensity':
      _touchpadHapticsIntensity =
        (args?.intensity as 'low' | 'medium' | 'high') ?? _touchpadHapticsIntensity;
      return undefined as T;

    case 'set_touchpad_gesture_screenshot':
      _touchpadGestureScreenshot = (args?.enabled as boolean) ?? _touchpadGestureScreenshot;
      return undefined as T;

    case 'set_touchpad_repress':
      _touchpadRepress = (args?.enabled as boolean) ?? _touchpadRepress;
      return undefined as T;

    case 'set_touchpad_edge_slide':
      _touchpadEdgeSlide = (args?.enabled as boolean) ?? _touchpadEdgeSlide;
      return undefined as T;

    // ── Performance ────────────────────────────────────────────────────────
    case 'get_performance_mode':
      return _performanceMode as T;

    case 'set_performance_mode':
      _performanceMode = (args?.mode as PerformanceMode) ?? _performanceMode;
      return { mode: _performanceMode, hw_value: 1 } as T;

    // ── Charging ───────────────────────────────────────────────────────────
    case 'get_charging_threshold':
      return _chargingThreshold as T;

    case 'set_charging_threshold':
      _chargingThreshold = (args?.threshold as number) ?? _chargingThreshold;
      return undefined as T;

    // ── Startup ────────────────────────────────────────────────────────────
    case 'get_autostart':
      return _autostart as T;

    case 'set_autostart':
      _autostart = (args?.enabled as boolean) ?? _autostart;
      return undefined as T;

    // ── Updates ────────────────────────────────────────────────────────────
    case 'get_update_status':
      return UPDATE_STATUS as T;

    case 'trigger_driver_scan':
      return UPDATE_STATUS as T;

    // ── Hardware discovery ─────────────────────────────────────────────────
    case 'get_hardware_profile':
      return HARDWARE_PROFILE as T;

    case 'run_hardware_discovery':
      await new Promise((r) => setTimeout(r, 1200)); // simulate discovery time
      return HARDWARE_PROFILE as T;

    case 'install_driver':
      await new Promise((r) => setTimeout(r, 800));
      return 'Driver installed successfully (mock)' as T;

    // ── Credential store (secrets) ────────────────────────────────────────
    case 'get_secret': {
      const key = args?.key as string;
      // Return 'granted' for telemetry consent so the consent dialog doesn't appear
      if (key === 'telemetry_consent') return 'granted' as T;
      return null as T;
    }

    case 'set_secret':
      return undefined as T;

    case 'delete_secret':
      return undefined as T;

    // ── Tray ───────────────────────────────────────────────────────────────
    case 'open_main_window':
      return undefined as T;

    // ── AI analysis (mock) ─────────────────────────────────────────────────
    case 'analyze_system':
      return 'Mock analysis: System running optimally.' as T;

    case 'test_connection':
      return 'ok' as T;

    // ── Audio ─────────────────────────────────────────────────────────────
    case 'get_audio_volume':
      return { volume: 65, muted: false } as T;

    case 'set_audio_volume':
      return undefined as T;

    case 'set_audio_mute':
      return undefined as T;

    // ── Thermal ────────────────────────────────────────────────────────────
    case 'get_thermal_zones':
      return [{ name: 'THRM0', temperature_celsius: 45.2, critical_temp_celsius: 105 }] as T;

    case 'get_primary_thermal_zone':
      return { name: 'THRM0', temperature_celsius: 45.2, critical_temp_celsius: 105 } as T;

    // ── EC / WMI (mock) ────────────────────────────────────────────────────
    case 'is_elevated':
      return false as T;

    case 'get_ecram_map':
      return {} as T;

    case 'get_iot_device_info':
      return { device_name: 'Mock IoT Device', is_connected: true } as T;

    case 'get_iot_wifi_list':
      return { networks: [], connected_ssid: 'MockWiFi' } as T;

    // ── AI usage stats ──────────────────────────────────────────────────────
    case 'get_ai_usage':
      return {
        total_requests: 42,
        total_input_tokens: 128_500,
        total_output_tokens: 15_200,
        estimated_cost_usd: 0.23,
      } as T;

    case 'reset_ai_usage':
      return undefined as T;

    case 'read_ai_perf_logs':
      return [] as T;

    case 'write_ai_perf_log':
      return undefined as T;

    case 'open_ai_logs_dir':
      return undefined as T;

    // ── Audio devices ───────────────────────────────────────────────────────
    case 'get_audio_devices':
      return {
        playback: [
          {
            name: 'Speakers (Realtek Audio)',
            id: 'dev0',
            direction: 'playback',
            is_default: true,
            volume: 65,
            muted: false,
          },
          {
            name: 'Headphones (Bluetooth)',
            id: 'dev1',
            direction: 'playback',
            is_default: false,
            volume: 50,
            muted: false,
          },
        ],
        capture: [
          {
            name: 'Microphone (Realtek Audio)',
            id: 'dev2',
            direction: 'capture',
            is_default: true,
            volume: 80,
            muted: false,
          },
        ],
      } as T;

    // ── Screen cast ──────────────────────────────────────────────────────────
    case 'get_cast_devices':
      return [
        { name: 'Living Room TV', id: 'cast-0', device_type: 'display' },
        { name: 'Office Monitor', id: 'cast-1', device_type: 'display' },
      ] as T;

    case 'start_casting':
      return { success: true, message: 'Casting started (mock)' } as T;

    case 'stop_casting':
      return { success: true, message: 'Casting stopped (mock)' } as T;

    // ── Data management ──────────────────────────────────────────────────────
    case 'export_user_data':
      return 'C:\\Users\\mock\\miControl_export.zip' as T;

    case 'delete_all_user_data':
      return undefined as T;

    // ── File / system helpers ───────────────────────────────────────────────
    case 'reveal_in_explorer':
      return undefined as T;

    case 'relaunch_as_admin':
      return undefined as T;

    case 'resize_tray_popup':
      return undefined as T;

    // ── EC / WMI read commands ──────────────────────────────────────────────
    case 'wmi_ec_read':
      return {
        sger: 0,
        futr: 0,
        frd0: 0,
        frd1: 0,
        frd2: 0,
        frd3: 0,
        raw: [
          0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ],
      } as T;

    case 'wmi_ec_write':
      return {
        sger: 0,
        futr: 0,
        frd0: 0,
        frd1: 0,
        frd2: 0,
        frd3: 0,
        raw: [
          0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ],
      } as T;

    case 'wmi_ec_get_performance_mode':
      return 'Balanced' as T;

    case 'wmi_ec_read_battery_health':
      return 94 as T;

    case 'wmi_ec_read_adapter_power':
      return 65 as T;

    case 'wmi_ec_read_sensor_data':
      return {
        battery_health: 94,
        adapter_power: 65,
        mi_usage_type: 1,
        wmid_type: 0,
        lid_open_type: 1,
        removable_type: 0,
        current_mode: 6,
      } as T;

    // ── EC / WMI set commands (all no-ops in mock) ─────────────────────────
    case 'wmi_ec_set_performance_mode':
    case 'wmi_ec_set_auto_illumination':
    case 'wmi_ec_set_brightness_data':
    case 'wmi_ec_set_epof_flag':
    case 'wmi_ec_set_label_mode':
    case 'wmi_ec_set_lid_open_type':
    case 'wmi_ec_set_mi_usage_type':
    case 'wmi_ec_set_pl1_flag':
    case 'wmi_ec_set_removable_type':
    case 'wmi_ec_set_sagv_mode':
    case 'wmi_ec_set_wmid_type':
      return undefined as T;

    // ── HQWmi commands ──────────────────────────────────────────────────────
    case 'hq_set_performance_mode':
    case 'hq_change_boot_option':
    case 'hq_load_default':
    case 'hq_s5_rtc_wake_enable':
    case 'hq_enable_pxe_boot':
    case 'hq_set_wifi_country_code':
    case 'hq_set_shipping_country_code':
      return { method: command, req: String(args?.req ?? ''), ret: 'OK', success: true } as T;

    // ── IoT hex / region ─────────────────────────────────────────────────────
    case 'get_iot_region_hex':
      return '00000000' as T;

    case 'write_iot_hex':
      return undefined as T;

    case 'iot_notify_event':
      return undefined as T;

    // ── EC RAM raw read ──────────────────────────────────────────────────────
    case 'read_ecram_raw':
      return '00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00' as T;

    // ── Performance debug ────────────────────────────────────────────────────
    case 'get_perf_debug':
      return {
        hq_wmi_instance: 'ACPI\\PNP0C14\\0',
        hq_wmi_works: true,
        hq_wmi_test_ret: 'OK',
        vhf_device_path: HARDWARE_PROFILE.vhf_device_path,
        registry_mode: 'balance',
        overlay_mode: 'none',
      } as T;

    // ── Keyboard / hotkeys ──────────────────────────────────────────────────
    case 'get_hotkey_config':
      return {
        ai_key: { enabled: true, action: { type: 'toggle_ai_brightness' } },
        xiaomi_key: { enabled: true, action: { type: 'open_main_window' } },
        copilot_key: { enabled: false, action: { type: 'none' } },
      } as T;

    case 'set_hotkey_config':
      return undefined as T;

    case 'start_key_detect':
      return undefined as T;

    case 'get_detected_key':
      return 162 as T; // VK_LCONTROL

    case 'is_hook_active':
      return true as T;

    // ── WiFi ─────────────────────────────────────────────────────────────────
    case 'wifi_scan':
      return [
        { ssid: 'MiHome-2.4G', signal: 85, security: 'WPA2', connected: true },
        { ssid: 'TP-Link_5G', signal: 62, security: 'WPA2', connected: false },
        { ssid: 'eduroam', signal: 40, security: 'WPA2-Enterprise', connected: false },
      ] as T;

    case 'wifi_status':
      return { wifi_status: 1, ssid: 'MiHome-2.4G' } as T;

    case 'wifi_connect':
      return undefined as T;

    case 'wifi_disconnect':
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
