import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

// ── Type definitions matching Rust structs ───────────────────────────────────

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

export interface HardwareState {
  battery: BatteryInfo | null;
  display: DisplayInfo | null;
  fan: FanInfo | null;
  touchpad: TouchpadInfo | null;
  system_info: SystemInfo | null;
  performance_mode: PerformanceMode | null;
  charging_threshold: number | null;
  audio: AudioVolumeResult | null;
}

export interface HardwareRefreshErrors {
  system_info: string | null;
  battery: string | null;
  display: string | null;
  fan: string | null;
  touchpad: string | null;
  performance_mode: string | null;
  charging_threshold: string | null;
  audio: string | null;
}

const EMPTY_REFRESH_ERRORS: HardwareRefreshErrors = {
  system_info: null,
  battery: null,
  display: null,
  fan: null,
  touchpad: null,
  performance_mode: null,
  charging_threshold: null,
  audio: null,
};

function formatInvokeError(reason: unknown): string {
  if (reason instanceof Error) return reason.message;
  return String(reason);
}

const REFRESH_ERROR_LABELS: Record<keyof HardwareRefreshErrors, string> = {
  system_info: 'system',
  battery: 'battery',
  display: 'display',
  fan: 'fan',
  touchpad: 'touchpad',
  performance_mode: 'performance',
  charging_threshold: 'charging',
  audio: 'audio',
};

// ── Hardware hook ────────────────────────────────────────────────────────────

export function useHardware() {
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null);
  const [battery, setBattery] = useState<BatteryInfo | null>(null);
  const [display, setDisplay] = useState<DisplayInfo | null>(null);
  const [fan, setFan] = useState<FanInfo | null>(null);
  const [touchpad, setTouchpad] = useState<TouchpadInfo | null>(null);
  const [performanceMode, setPerformanceModeState] = useState<PerformanceMode>('balance');
  const [lastPerfResult, setLastPerfResult] = useState<PerformanceResult | null>(null);
  const [chargingThreshold, setChargingThresholdState] = useState<number>(80);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [refreshErrors, setRefreshErrors] = useState<HardwareRefreshErrors>(EMPTY_REFRESH_ERRORS);
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus | null>(null);
  const [loadingUpdate, setLoadingUpdate] = useState(false);
  const [hardwareProfile, setHardwareProfile] = useState<HardwareProfile | null>(null);
  const [loadingDiscovery, setLoadingDiscovery] = useState(false);
  const [audioState, setAudioState] = useState<AudioVolumeResult | null>(null);

  const hasLoadedOnce = useRef(false);
  // Prevent the 2 s poll from overwriting optimistic touchpad state immediately
  // after a user write. Cleared automatically once the lock expires.
  const touchpadDirtyUntil = useRef<number>(0);
  const touchpadRef = useRef<TouchpadInfo | null>(null);
  const displayRef = useRef<DisplayInfo | null>(null);
  const fanRef = useRef<FanInfo | null>(null);
  const performanceModeRef = useRef<PerformanceMode>('balance');
  const chargingThresholdRef = useRef<number>(80);
  const audioStateRef = useRef<AudioVolumeResult | null>(null);

  const refresh = useCallback(async () => {
    if (!hasLoadedOnce.current) setLoading(true);

    const nextErrors: HardwareRefreshErrors = { ...EMPTY_REFRESH_ERRORS };

    try {
      const state = await invoke<HardwareState>('get_hardware_state_batch');

      if (state.system_info !== null) setSystemInfo(state.system_info);
      else nextErrors.system_info = 'no data';

      if (state.battery !== null) setBattery(state.battery);
      else nextErrors.battery = 'no data';

      if (state.display !== null) setDisplay(state.display);
      else nextErrors.display = 'no data';

      if (state.fan !== null) setFan(state.fan);
      else nextErrors.fan = 'no data';

      // Only update touchpad from poll when no user write is in flight.
      if (state.touchpad !== null && Date.now() >= touchpadDirtyUntil.current) {
        setTouchpad(state.touchpad);
      } else if (state.touchpad === null) {
        nextErrors.touchpad = 'no data';
      }

      if (state.performance_mode !== null) setPerformanceModeState(state.performance_mode);
      else nextErrors.performance_mode = 'no data';

      if (state.charging_threshold !== null) setChargingThresholdState(state.charging_threshold);
      else nextErrors.charging_threshold = 'no data';

      if (state.audio !== null) setAudioState(state.audio);
      else nextErrors.audio = 'no data';
    } catch (e) {
      nextErrors.system_info = formatInvokeError(e);
      nextErrors.battery = formatInvokeError(e);
      nextErrors.display = formatInvokeError(e);
      nextErrors.fan = formatInvokeError(e);
      nextErrors.touchpad = formatInvokeError(e);
      nextErrors.performance_mode = formatInvokeError(e);
      nextErrors.charging_threshold = formatInvokeError(e);
      nextErrors.audio = formatInvokeError(e);
    }

    setRefreshErrors(nextErrors);
    const failedSubsystems = Object.entries(nextErrors)
      .filter(([, value]) => Boolean(value))
      .map(([key]) => REFRESH_ERROR_LABELS[key as keyof HardwareRefreshErrors]);
    setError(failedSubsystems.length ? `Refresh failed for: ${failedSubsystems.join(', ')}` : null);

    hasLoadedOnce.current = true;
    setLoading(false);
  }, []);

  useEffect(() => {
    const refreshIfVisible = () => {
      // The tray popup window is pre-created and often kept hidden.
      // Skip polling while hidden to avoid background WMI/query churn.
      if (typeof document !== 'undefined' && document.visibilityState !== 'visible') {
        return;
      }
      void refresh();
    };

    refreshIfVisible();
    // Poll every 2 s while the webview is visible so external hardware changes
    // (Fn brightness keys, power events, fan fluctuations) stay responsive.
    const interval = setInterval(refreshIfVisible, 2000);

    const onVisibilityChange = () => {
      if (typeof document !== 'undefined' && document.visibilityState === 'visible') {
        void refresh();
      }
    };
    document.addEventListener('visibilitychange', onVisibilityChange);

    return () => {
      document.removeEventListener('visibilitychange', onVisibilityChange);
      clearInterval(interval);
    };
  }, [refresh]);

  const setPerformanceMode = useCallback(async (mode: PerformanceMode) => {
    const snap = performanceModeRef.current;
    setPerformanceModeState(mode);
    try {
      const result = await invoke<PerformanceResult>('set_performance_mode', { mode });
      setLastPerfResult(result);
    } catch (e) {
      setPerformanceModeState(snap);
      console.error('[perf] set_performance_mode failed:', e);
      throw e;
    }
  }, []);

  const setChargingThreshold = useCallback(async (threshold: number) => {
    const snap = chargingThresholdRef.current;
    setChargingThresholdState(threshold);
    try {
      await invoke('set_charging_threshold', { threshold });
    } catch (e) {
      setChargingThresholdState(snap);
      console.error('[charge] set_charging_threshold failed:', e);
      throw e;
    }
  }, []);

  const setBrightness = useCallback(async (level: number) => {
    const snap = displayRef.current;
    setDisplay((prev) => (prev ? { ...prev, brightness: level } : null));
    try {
      await invoke('set_brightness', { level });
    } catch (e) {
      setDisplay(snap);
      console.error('[display] set_brightness failed:', e);
      throw e;
    }
  }, []);

  const setHdr = useCallback(async (enabled: boolean) => {
    const snap = displayRef.current;
    setDisplay((prev) => (prev ? { ...prev, hdr_enabled: enabled } : null));
    try {
      await invoke('set_hdr', { enabled });
    } catch (e) {
      setDisplay(snap);
      console.error('[display] set_hdr failed:', e);
      throw e;
    }
  }, []);

  const setAiBrightness = useCallback(async (enabled: boolean) => {
    const snap = displayRef.current;
    setDisplay((prev) => (prev ? { ...prev, ai_brightness: enabled } : null));
    try {
      await invoke('set_ai_brightness', { enabled });
    } catch (e) {
      setDisplay(snap);
      console.error('[display] set_ai_brightness failed:', e);
      throw e;
    }
  }, []);

  const setAiBrightnessConfig = useCallback(async (config: AiBrightnessConfig) => {
    const snap = displayRef.current;
    setDisplay((prev) =>
      prev ? { ...prev, ai_brightness: config.enabled, ai_brightness_config: config } : null,
    );
    try {
      await invoke('set_ai_brightness_config', { config });
    } catch (e) {
      setDisplay(snap);
      console.error('[display] set_ai_brightness_config failed:', e);
      throw e;
    }
  }, []);

  const setFanMode = useCallback(async (mode: 'auto' | 'fixed' | 'off', speedPercent?: number) => {
    const snap = fanRef.current;
    setFan((prev) =>
      prev ? { ...prev, mode, speed_percent: speedPercent ?? prev.speed_percent } : null,
    );
    try {
      await invoke('set_fan_mode', { mode, speed_percent: speedPercent ?? 50 });
    } catch (e) {
      setFan(snap);
      console.error('[fan] set_fan_mode failed:', e);
      throw e;
    }
  }, []);

  // Keep stable refs so error-revert closures (with empty deps) can restore
  // the previous snapshot of any state that supports optimistic updates.
  useEffect(() => {
    touchpadRef.current = touchpad;
  }, [touchpad]);
  useEffect(() => {
    displayRef.current = display;
  }, [display]);
  useEffect(() => {
    fanRef.current = fan;
  }, [fan]);
  useEffect(() => {
    performanceModeRef.current = performanceMode;
  }, [performanceMode]);
  useEffect(() => {
    chargingThresholdRef.current = chargingThreshold;
  }, [chargingThreshold]);
  useEffect(() => {
    audioStateRef.current = audioState;
  }, [audioState]);

  const setTouchpadSensitivity = useCallback(
    async (sensitivity: 'low' | 'medium' | 'high' | 'very_high') => {
      const snap = touchpadRef.current;
      touchpadDirtyUntil.current = Date.now() + 3000;
      setTouchpad((s) => (s ? { ...s, sensitivity } : null));
      try {
        await invoke('set_touchpad_sensitivity', { sensitivity });
        touchpadDirtyUntil.current = 0;
      } catch (e) {
        setTouchpad(snap);
        touchpadDirtyUntil.current = 0;
        console.error('[touchpad] set_touchpad_sensitivity failed:', e);
        throw e;
      }
    },
    [],
  );

  const setTouchpadHaptics = useCallback(async (enabled: boolean) => {
    const snap = touchpadRef.current;
    touchpadDirtyUntil.current = Date.now() + 3000;
    setTouchpad((s) => (s ? { ...s, haptics_enabled: enabled } : null));
    try {
      await invoke('set_touchpad_haptics', { enabled });
      touchpadDirtyUntil.current = 0;
    } catch (e) {
      setTouchpad(snap);
      touchpadDirtyUntil.current = 0;
      console.error('[touchpad] set_touchpad_haptics failed:', e);
      throw e;
    }
  }, []);

  const setTouchpadHapticsIntensity = useCallback(async (intensity: 'low' | 'medium' | 'high') => {
    const snap = touchpadRef.current;
    touchpadDirtyUntil.current = Date.now() + 3000;
    setTouchpad((s) => (s ? { ...s, haptics_intensity: intensity } : null));
    try {
      await invoke('set_touchpad_haptics_intensity', { intensity });
      touchpadDirtyUntil.current = 0;
    } catch (e) {
      setTouchpad(snap);
      touchpadDirtyUntil.current = 0;
      console.error('[touchpad] set_touchpad_haptics_intensity failed:', e);
      throw e;
    }
  }, []);

  const setTouchpadGestureScreenshot = useCallback(async (enabled: boolean) => {
    const snap = touchpadRef.current;
    touchpadDirtyUntil.current = Date.now() + 3000;
    setTouchpad((s) => (s ? { ...s, gesture_screenshot: enabled } : null));
    try {
      await invoke('set_touchpad_gesture_screenshot', { enabled });
      touchpadDirtyUntil.current = 0;
    } catch (e) {
      setTouchpad(snap);
      touchpadDirtyUntil.current = 0;
      console.error('[touchpad] set_touchpad_gesture_screenshot failed:', e);
      throw e;
    }
  }, []);

  const setTouchpadRepress = useCallback(async (enabled: boolean) => {
    const snap = touchpadRef.current;
    touchpadDirtyUntil.current = Date.now() + 3000;
    setTouchpad((s) => (s ? { ...s, trackpad_repress: enabled } : null));
    try {
      await invoke('set_touchpad_repress', { enabled });
      touchpadDirtyUntil.current = 0;
    } catch (e) {
      setTouchpad(snap);
      touchpadDirtyUntil.current = 0;
      console.error('[touchpad] set_touchpad_repress failed:', e);
      throw e;
    }
  }, []);

  const setTouchpadEdgeSlide = useCallback(async (enabled: boolean) => {
    const snap = touchpadRef.current;
    touchpadDirtyUntil.current = Date.now() + 3000;
    setTouchpad((s) => (s ? { ...s, edge_slide: enabled } : null));
    try {
      await invoke('set_touchpad_edge_slide', { enabled });
      touchpadDirtyUntil.current = 0;
    } catch (e) {
      setTouchpad(snap);
      touchpadDirtyUntil.current = 0;
      console.error('[touchpad] set_touchpad_edge_slide failed:', e);
      throw e;
    }
  }, []);

  const setRefreshRate = useCallback(async (hz: number) => {
    const snap = displayRef.current;
    setDisplay((prev) => (prev ? { ...prev, refresh_rate_hz: hz } : null));
    try {
      await invoke('set_refresh_rate', { hz });
    } catch (e) {
      setDisplay(snap);
      console.error('[display] set_refresh_rate failed:', e);
      throw e;
    }
  }, []);

  const setAdaptiveRefreshRate = useCallback(async (enabled: boolean) => {
    const snap = displayRef.current;
    setDisplay((prev) => (prev ? { ...prev, adaptive_refresh_rate: enabled } : null));
    try {
      await invoke('set_adaptive_refresh_rate', { enabled });
    } catch (e) {
      setDisplay(snap);
      console.error('[display] set_adaptive_refresh_rate failed:', e);
      throw e;
    }
  }, []);

  const getProcessList = useCallback(async () => {
    try {
      return await invoke<ProcessInfo[]>('get_process_list');
    } catch (e) {
      console.error('[sys] get_process_list failed:', e);
      return [];
    }
  }, []);

  // Update status is NOT polled — fetched once on mount + manually
  const refreshUpdateStatus = useCallback(async () => {
    setLoadingUpdate(true);
    try {
      // Run the scan and a minimum 2-second visual feedback delay in parallel.
      const [status] = await Promise.all([
        invoke<UpdateStatus>('get_update_status'),
        new Promise<void>((resolve) => setTimeout(resolve, 2000)),
      ]);
      setUpdateStatus(status);
    } catch (e) {
      // non-fatal — update panel shows fallback
      console.warn('get_update_status error:', e);
    } finally {
      setLoadingUpdate(false);
    }
  }, []);

  useEffect(() => {
    void refreshUpdateStatus();
  }, [refreshUpdateStatus]);

  // Hardware profile — fetched once at mount; re-fetched after a discovery run
  const refreshHardwareProfile = useCallback(async () => {
    try {
      const profile = await invoke<HardwareProfile | null>('get_hardware_profile');
      setHardwareProfile(profile ?? null);
    } catch (e) {
      console.warn('get_hardware_profile error:', e);
    }
  }, []);

  const runHardwareDiscovery = useCallback(async () => {
    setLoadingDiscovery(true);
    try {
      const profile = await invoke<HardwareProfile>('run_hardware_discovery');
      setHardwareProfile(profile);
      return profile;
    } catch (e) {
      console.warn('run_hardware_discovery error:', e);
      throw e;
    } finally {
      setLoadingDiscovery(false);
    }
  }, []);

  const installDriver = useCallback(async (driverName: string) => {
    try {
      return await invoke<string>('install_driver', { driverName });
    } catch (e) {
      console.error('[setup] install_driver failed:', e);
      throw e;
    }
  }, []);

  // ── AI performance log commands ───────────────────────────────────────────
  const writeAiPerfLog = useCallback(async (entry: AiPerfLogEntry) => {
    try {
      await invoke('write_ai_perf_log', { entry });
    } catch (e) {
      console.error('[perf] write_ai_perf_log failed:', e);
    }
  }, []);

  const readAiPerfLogs = useCallback(async (limit?: number) => {
    try {
      return await invoke<AiPerfLogEntry[]>('read_ai_perf_logs', { limit });
    } catch (e) {
      console.error('[perf] read_ai_perf_logs failed:', e);
      return [];
    }
  }, []);

  const openAiLogsDir = useCallback(async () => {
    try {
      await invoke('open_ai_logs_dir');
    } catch (e) {
      console.warn('[perf] open_ai_logs_dir failed:', e);
    }
  }, []);

  const getEcramMap = useCallback(async () => {
    try {
      return await invoke<EramMap>('get_ecram_map');
    } catch (e) {
      console.error('[iot] get_ecram_map failed:', e);
      throw e;
    }
  }, []);

  const getIotRegionHex = useCallback(async (region: IotRegionName) => {
    try {
      return await invoke<string>('get_iot_region_hex', { region });
    } catch (e) {
      console.error('[iot] get_iot_region_hex failed:', e);
      throw e;
    }
  }, []);

  const writeIotHex = useCallback(async (address: string, hexData: string) => {
    try {
      await invoke('write_iot_hex', { address, hexData });
    } catch (e) {
      console.error('[iot] write_iot_hex failed:', e);
      throw e;
    }
  }, []);

  const readEcramRaw = useCallback(async (address: string, count: number) => {
    try {
      return await invoke<string>('read_ecram_raw', { address, count });
    } catch (e) {
      console.error('[iot] read_ecram_raw failed:', e);
      throw e;
    }
  }, []);

  const isElevated = useCallback(async () => {
    try {
      return await invoke<boolean>('is_elevated');
    } catch (e) {
      console.error('[iot] is_elevated failed:', e);
      return false;
    }
  }, []);

  const relaunchAsAdmin = useCallback(async () => {
    try {
      await invoke('relaunch_as_admin');
    } catch (e) {
      console.error('[app] relaunch_as_admin failed:', e);
      throw e;
    }
  }, []);

  // Audio state is now polled as part of the batched get_hardware_state_batch.
  // The individual getAudioState helper is kept for one-shot reads (e.g. after
  // user-initiated volume/mute changes).
  const getAudioState = useCallback(async () => {
    try {
      const result = await invoke<AudioVolumeResult>('get_audio_volume');
      setAudioState(result);
    } catch (e) {
      console.error('getAudioState failed:', e);
    }
  }, []);

  const setMasterVolume = useCallback(
    async (volumeFraction: number) => {
      const volume = Math.round(volumeFraction * 100);
      const snap = audioStateRef.current;
      setAudioState((prev) => (prev ? { ...prev, volume } : null));
      try {
        await invoke('set_audio_volume', { volume });
        await getAudioState();
      } catch (e) {
        setAudioState(snap);
        console.error('[audio] set_audio_volume failed:', e);
        throw e;
      }
    },
    [getAudioState],
  );

  const setMasterMute = useCallback(
    async (muted: boolean) => {
      const snap = audioStateRef.current;
      setAudioState((prev) => (prev ? { ...prev, muted } : null));
      try {
        await invoke('set_audio_mute', { muted });
        await getAudioState();
      } catch (e) {
        setAudioState(snap);
        console.error('[audio] set_audio_mute failed:', e);
        throw e;
      }
    },
    [getAudioState],
  );

  useEffect(() => {
    void refreshHardwareProfile();
  }, [refreshHardwareProfile]);

  return useMemo(
    () => ({
      systemInfo,
      battery,
      display,
      fan,
      touchpad,
      performanceMode,
      lastPerfResult,
      chargingThreshold,
      loading,
      error,
      refreshErrors,
      refresh,
      setPerformanceMode,
      setChargingThreshold,
      setBrightness,
      setHdr,
      setAiBrightness,
      setAiBrightnessConfig,
      setFanMode,
      setTouchpadSensitivity,
      setTouchpadHaptics,
      setTouchpadHapticsIntensity,
      setTouchpadGestureScreenshot,
      setTouchpadRepress,
      setTouchpadEdgeSlide,
      updateStatus,
      loadingUpdate,
      refreshUpdateStatus,
      hardwareProfile,
      loadingDiscovery,
      refreshHardwareProfile,
      runHardwareDiscovery,
      installDriver,
      setRefreshRate,
      setAdaptiveRefreshRate,
      getProcessList,
      writeAiPerfLog,
      readAiPerfLogs,
      openAiLogsDir,
      getEcramMap,
      getIotRegionHex,
      writeIotHex,
      readEcramRaw,
      isElevated,
      relaunchAsAdmin,
      audioState,
      getAudioState,
      setMasterVolume,
      setMasterMute,
    }),
    [
      systemInfo,
      battery,
      display,
      fan,
      touchpad,
      performanceMode,
      lastPerfResult,
      chargingThreshold,
      loading,
      error,
      refreshErrors,
      refresh,
      setPerformanceMode,
      setChargingThreshold,
      setBrightness,
      setHdr,
      setAiBrightness,
      setAiBrightnessConfig,
      setFanMode,
      setTouchpadSensitivity,
      setTouchpadHaptics,
      setTouchpadHapticsIntensity,
      setTouchpadGestureScreenshot,
      setTouchpadRepress,
      setTouchpadEdgeSlide,
      updateStatus,
      loadingUpdate,
      refreshUpdateStatus,
      hardwareProfile,
      loadingDiscovery,
      refreshHardwareProfile,
      runHardwareDiscovery,
      installDriver,
      setRefreshRate,
      setAdaptiveRefreshRate,
      getProcessList,
      writeAiPerfLog,
      readAiPerfLogs,
      openAiLogsDir,
      getEcramMap,
      getIotRegionHex,
      writeIotHex,
      readEcramRaw,
      isElevated,
      relaunchAsAdmin,
      audioState,
      getAudioState,
      setMasterVolume,
      setMasterMute,
    ],
  );
}
