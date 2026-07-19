import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { getUserFriendlyMessage, parseErrorResponse } from '../types/error';
import type { TranslateFn } from '../types/error';
import { t } from './useI18n';

// Adapter: the i18n `t` function uses a typed StringKey union, but
// getUserFriendlyMessage expects a plain (key: string) => string.
// This wrapper bridges the two without breaking type safety of `t`.
const translate: TranslateFn = (key) => t(key as never);

// ── Types (re-exported from src/types/hardware.ts for backward compat) ────────

export type {
  PerformanceMode,
  PerformanceResult,
  SystemInfo,
  ProcessInfo,
  BatteryInfo,
  AiBrightnessConfig,
  DisplayInfo,
  FanInfo,
  AiPerfLogEntry,
  TouchpadInfo,
  BiosInfo,
  XiaomiDriverInfo,
  UpdateStatus,
  MissingDriver,
  HardwareCapabilities,
  HardwareProfile,
  IotRegionName,
  EramMap,
  AudioVolumeResult,
  IotDeviceInfo,
  IotWifiList,
  IotEvent,
  PowerEvent,
  PowerEventType,
  LaptopStatus,
  BindStatusInfo,
  WiFiStatusInfo,
  WiFiItemInfo,
  HqWmiResponse,
  ThermalZoneInfo,
} from '../types/hardware';

import type {
  PerformanceMode,
  PerformanceResult,
  SystemInfo,
  ProcessInfo,
  BatteryInfo,
  AiBrightnessConfig,
  DisplayInfo,
  FanInfo,
  AiPerfLogEntry,
  TouchpadInfo,
  UpdateStatus,
  HardwareProfile,
  IotRegionName,
  EramMap,
  AudioVolumeResult,
  IotDeviceInfo,
  IotWifiList,
  IotEvent,
  WmaaResponse,
  EcSensorData,
  HqWmiResponse,
  ThermalZoneInfo,
} from '../types/hardware';

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

  const clearError = useCallback(() => setError(null), []);
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus | null>(null);
  const [loadingUpdate, setLoadingUpdate] = useState(false);
  const [hardwareProfile, setHardwareProfile] = useState<HardwareProfile | null>(null);
  const [loadingDiscovery, setLoadingDiscovery] = useState(false);
  const [audioState, setAudioState] = useState<AudioVolumeResult | null>(null);

  // Prevent the 2 s fast poll from overwriting optimistic touchpad state immediately
  // after a user write. Cleared automatically once the lock expires.
  const touchpadDirtyUntil = useRef<number>(0);
  const touchpadRef = useRef<TouchpadInfo | null>(null);
  const displayRef = useRef<DisplayInfo | null>(null);
  const fanRef = useRef<FanInfo | null>(null);
  const performanceModeRef = useRef<PerformanceMode>('balance');
  const chargingThresholdRef = useRef<number>(80);
  const audioStateRef = useRef<AudioVolumeResult | null>(null);

  // ── Tiered polling (S11-002) ─────────────────────────────────────────────
  // Fast tier: 2 s — fan speed, CPU temp, GPU temp, CPU usage, GPU usage
  // Slow tier: 15 s — battery, display, touchpad
  // Poll-once: system info — fetched on mount only

  const FAST_POLL_INTERVAL = 2000;
  const SLOW_POLL_INTERVAL = 15000;

  const fastPoll = useCallback(async () => {
    try {
      const [fanResult, systemResult, audioResult] = await Promise.all([
        invoke<FanInfo>('get_fan_info'),
        invoke<SystemInfo>('get_system_info'),
        invoke<AudioVolumeResult>('get_audio_volume'),
      ]);
      setFan(fanResult);
      if (systemResult) {
        setSystemInfo(systemResult);
      }
      setAudioState(audioResult);
      setError(null);
    } catch (e) {
      console.error('Fast poll failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
    }
  }, []);

  const slowPoll = useCallback(async () => {
    try {
      const [batteryResult, displayResult, touchpadResult, perfMode, chargeThreshold] =
        await Promise.all([
          invoke<BatteryInfo>('get_battery_info'),
          invoke<DisplayInfo>('get_display_info'),
          invoke<TouchpadInfo>('get_touchpad_info'),
          invoke<PerformanceMode>('get_performance_mode'),
          invoke<number>('get_charging_threshold'),
        ]);
      if (batteryResult !== null) setBattery(batteryResult);
      if (displayResult !== null) setDisplay(displayResult);
      // Only update touchpad from poll when no user write is in flight.
      if (touchpadResult !== null && Date.now() >= touchpadDirtyUntil.current) {
        setTouchpad(touchpadResult);
      }
      if (perfMode) setPerformanceModeState(perfMode);
      setChargingThresholdState(chargeThreshold);
      setError(null);
    } catch (e) {
      console.error('Slow poll failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
    }
  }, []);

  const initialLoadRef = useRef(false);

  useEffect(() => {
    const refreshIfVisible = (pollFn: () => Promise<void>) => {
      // The tray popup window is pre-created and often kept hidden.
      // Skip polling while hidden to avoid background WMI/query churn.
      if (typeof document !== 'undefined' && document.visibilityState !== 'visible') {
        return;
      }
      void pollFn();
    };

    // Initial load: fetch everything once + system info (poll-once)
    if (!initialLoadRef.current) {
      initialLoadRef.current = true;
      setLoading(true);
      const doInitialLoad = async () => {
        try {
          const [
            fanResult,
            systemResult,
            batteryResult,
            displayResult,
            touchpadResult,
            perfMode,
            chargeThreshold,
          ] = await Promise.all([
            invoke<FanInfo>('get_fan_info'),
            invoke<SystemInfo>('get_system_info'),
            invoke<BatteryInfo>('get_battery_info'),
            invoke<DisplayInfo>('get_display_info'),
            invoke<TouchpadInfo>('get_touchpad_info'),
            invoke<PerformanceMode>('get_performance_mode'),
            invoke<number>('get_charging_threshold'),
          ]);
          setFan(fanResult);
          setSystemInfo(systemResult);
          if (batteryResult !== null) setBattery(batteryResult);
          if (displayResult !== null) setDisplay(displayResult);
          if (touchpadResult !== null) setTouchpad(touchpadResult);
          if (perfMode) setPerformanceModeState(perfMode);
          setChargingThresholdState(chargeThreshold);
          setError(null);

          // Load hardware profile, auto-discover if not cached
          try {
            const profile = await invoke<HardwareProfile | null>('get_hardware_profile');
            if (profile) {
              setHardwareProfile(profile);
            } else {
              // Profile not cached — trigger discovery automatically
              try {
                const discovered = await invoke<HardwareProfile>('run_hardware_discovery');
                setHardwareProfile(discovered);
              } catch (e) {
                console.warn('[hardware] Auto-discovery failed:', e);
              }
            }
          } catch (e) {
            console.warn('[hardware] Failed to load profile:', e);
          }
        } catch (e) {
          console.error('Initial hardware load failed:', e);
          setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
        } finally {
          setLoading(false);
        }
      };
      void doInitialLoad();
    }

    // Fast polling interval
    const fastInterval = setInterval(() => refreshIfVisible(fastPoll), FAST_POLL_INTERVAL);

    // Slow polling interval
    const slowInterval = setInterval(() => refreshIfVisible(slowPoll), SLOW_POLL_INTERVAL);

    const onVisibilityChange = () => {
      if (typeof document !== 'undefined' && document.visibilityState === 'visible') {
        void fastPoll();
        void slowPoll();
      }
    };
    document.addEventListener('visibilitychange', onVisibilityChange);

    return () => {
      document.removeEventListener('visibilitychange', onVisibilityChange);
      clearInterval(fastInterval);
      clearInterval(slowInterval);
    };
  }, [fastPoll, slowPoll]);

  const setPerformanceMode = useCallback(async (mode: PerformanceMode) => {
    const snap = performanceModeRef.current;
    setPerformanceModeState(mode);
    try {
      const result = await invoke<PerformanceResult>('set_performance_mode', { mode });
      setLastPerfResult(result);
      setError(null);
    } catch (e) {
      setPerformanceModeState(snap);
      console.error('[perf] set_performance_mode failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  const setChargingThreshold = useCallback(async (threshold: number) => {
    const snap = chargingThresholdRef.current;
    setChargingThresholdState(threshold);
    try {
      await invoke('set_charging_threshold', { threshold });
      setError(null);
    } catch (e) {
      setChargingThresholdState(snap);
      console.error('[charge] set_charging_threshold failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  const setBrightness = useCallback(async (level: number) => {
    const snap = displayRef.current;
    setDisplay((prev) => (prev ? { ...prev, brightness: level } : null));
    try {
      await invoke('set_brightness', { level });
      setError(null);
    } catch (e) {
      setDisplay(snap);
      console.error('[display] set_brightness failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  const setHdr = useCallback(async (enabled: boolean) => {
    const snap = displayRef.current;
    setDisplay((prev) => (prev ? { ...prev, hdr_enabled: enabled } : null));
    try {
      await invoke('set_hdr', { enabled });
      setError(null);
    } catch (e) {
      setDisplay(snap);
      console.error('[display] set_hdr failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  const setAiBrightness = useCallback(async (enabled: boolean) => {
    const snap = displayRef.current;
    setDisplay((prev) => (prev ? { ...prev, ai_brightness: enabled } : null));
    try {
      await invoke('set_ai_brightness', { enabled });
      setError(null);
    } catch (e) {
      setDisplay(snap);
      console.error('[display] set_ai_brightness failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
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
      setError(null);
    } catch (e) {
      setDisplay(snap);
      console.error('[display] set_ai_brightness_config failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
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
      setError(null);
    } catch (e) {
      setFan(snap);
      console.error('[fan] set_fan_mode failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  // Keep stable refs so error-revert closures (with empty deps) can restore
  // the previous snapshot of any state that supports optimistic updates.
  useEffect(() => {
    touchpadRef.current = touchpad;
    displayRef.current = display;
    fanRef.current = fan;
    performanceModeRef.current = performanceMode;
    chargingThresholdRef.current = chargingThreshold;
    audioStateRef.current = audioState;
  }, [touchpad, display, fan, performanceMode, chargingThreshold, audioState]);

  const setTouchpadSensitivity = useCallback(
    async (sensitivity: 'low' | 'medium' | 'high' | 'very_high') => {
      const snap = touchpadRef.current;
      touchpadDirtyUntil.current = Date.now() + 3000;
      setTouchpad((s) => (s ? { ...s, sensitivity } : null));
      try {
        await invoke('set_touchpad_sensitivity', { sensitivity });
        touchpadDirtyUntil.current = 0;
        setError(null);
      } catch (e) {
        setTouchpad(snap);
        touchpadDirtyUntil.current = 0;
        console.error('[touchpad] set_touchpad_sensitivity failed:', e);
        setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
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
      setError(null);
    } catch (e) {
      setTouchpad(snap);
      touchpadDirtyUntil.current = 0;
      console.error('[touchpad] set_touchpad_haptics failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
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
      setError(null);
    } catch (e) {
      setTouchpad(snap);
      touchpadDirtyUntil.current = 0;
      console.error('[touchpad] set_touchpad_haptics_intensity failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
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
      setError(null);
    } catch (e) {
      setTouchpad(snap);
      touchpadDirtyUntil.current = 0;
      console.error('[touchpad] set_touchpad_gesture_screenshot failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
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
      setError(null);
    } catch (e) {
      setTouchpad(snap);
      touchpadDirtyUntil.current = 0;
      console.error('[touchpad] set_touchpad_repress failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
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
      setError(null);
    } catch (e) {
      setTouchpad(snap);
      touchpadDirtyUntil.current = 0;
      console.error('[touchpad] set_touchpad_edge_slide failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  const setRefreshRate = useCallback(async (hz: number) => {
    const snap = displayRef.current;
    setDisplay((prev) => (prev ? { ...prev, refresh_rate_hz: hz } : null));
    try {
      await invoke('set_refresh_rate', { hz });
      setError(null);
    } catch (e) {
      setDisplay(snap);
      console.error('[display] set_refresh_rate failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  const setAdaptiveRefreshRate = useCallback(async (enabled: boolean) => {
    const snap = displayRef.current;
    setDisplay((prev) => (prev ? { ...prev, adaptive_refresh_rate: enabled } : null));
    try {
      await invoke('set_adaptive_refresh_rate', { enabled });
      setError(null);
    } catch (e) {
      setDisplay(snap);
      console.error('[display] set_adaptive_refresh_rate failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  const getProcessList = useCallback(async () => {
    try {
      const result = await invoke<ProcessInfo[]>('get_process_list');
      setError(null);
      return result;
    } catch (e) {
      console.error('[sys] get_process_list failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
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
      setError(null);
    } catch (e) {
      // non-fatal — update panel shows fallback
      console.warn('get_update_status error:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
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
      setError(null);
    } catch (e) {
      console.warn('get_hardware_profile error:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
    }
  }, []);

  const runHardwareDiscovery = useCallback(async () => {
    setLoadingDiscovery(true);
    try {
      const profile = await invoke<HardwareProfile>('run_hardware_discovery');
      setHardwareProfile(profile);
      setError(null);
      return profile;
    } catch (e) {
      console.warn('run_hardware_discovery error:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    } finally {
      setLoadingDiscovery(false);
    }
  }, []);

  const installDriver = useCallback(async (driverName: string) => {
    try {
      const result = await invoke<string>('install_driver', { driverName });
      setError(null);
      return result;
    } catch (e) {
      console.error('[setup] install_driver failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  // ── AI performance log commands ───────────────────────────────────────────
  const writeAiPerfLog = useCallback(async (entry: AiPerfLogEntry) => {
    try {
      await invoke('write_ai_perf_log', { entry });
      setError(null);
    } catch (e) {
      console.error('[perf] write_ai_perf_log failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
    }
  }, []);

  const readAiPerfLogs = useCallback(async (limit?: number) => {
    try {
      const result = await invoke<AiPerfLogEntry[]>('read_ai_perf_logs', { limit });
      setError(null);
      return result;
    } catch (e) {
      console.error('[perf] read_ai_perf_logs failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      return [];
    }
  }, []);

  const openAiLogsDir = useCallback(async () => {
    try {
      await invoke('open_ai_logs_dir');
      setError(null);
    } catch (e) {
      console.warn('[perf] open_ai_logs_dir failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
    }
  }, []);

  const getEcramMap = useCallback(async () => {
    try {
      const result = await invoke<EramMap>('get_ecram_map');
      setError(null);
      return result;
    } catch (e) {
      console.error('[iot] get_ecram_map failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  const getIotRegionHex = useCallback(async (region: IotRegionName) => {
    try {
      const result = await invoke<string>('get_iot_region_hex', { region });
      setError(null);
      return result;
    } catch (e) {
      console.error('[iot] get_iot_region_hex failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  const writeIotHex = useCallback(async (address: string, hexData: string) => {
    try {
      await invoke('write_iot_hex', { address, hexData });
      setError(null);
    } catch (e) {
      console.error('[iot] write_iot_hex failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  const readEcramRaw = useCallback(async (address: string, count: number) => {
    try {
      const result = await invoke<string>('read_ecram_raw', { address, count });
      setError(null);
      return result;
    } catch (e) {
      console.error('[iot] read_ecram_raw failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  const isElevated = useCallback(async () => {
    try {
      const result = await invoke<boolean>('is_elevated');
      setError(null);
      return result;
    } catch (e) {
      console.error('[iot] is_elevated failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      return false;
    }
  }, []);

  const relaunchAsAdmin = useCallback(async () => {
    try {
      await invoke('relaunch_as_admin');
      setError(null);
    } catch (e) {
      console.error('[app] relaunch_as_admin failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  // ── Consolidated IoT commands (S28-005) ───────────────────────────────────

  const getIotDeviceInfo = useCallback(async () => {
    try {
      const result = await invoke<IotDeviceInfo>('get_iot_device_info');
      setError(null);
      return result;
    } catch (e) {
      console.error('[iot] get_iot_device_info failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  const getIotWifiList = useCallback(async () => {
    try {
      const result = await invoke<IotWifiList>('get_iot_wifi_list');
      setError(null);
      return result;
    } catch (e) {
      console.error('[iot] get_iot_wifi_list failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  const iotNotifyEvent = useCallback(async (event: IotEvent) => {
    try {
      await invoke('iot_notify_event', { event });
      setError(null);
    } catch (e) {
      console.error('[iot] iot_notify_event failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
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
      setError(null);
    } catch (e) {
      console.error('getAudioState failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
    }
  }, []);

  const setMasterVolume = useCallback(async (volumeFraction: number) => {
    const volume = Math.round(volumeFraction * 100);
    const snap = audioStateRef.current;
    setAudioState((prev) => (prev ? { ...prev, volume } : null));
    try {
      await invoke('set_audio_volume', { volume });
      setError(null);
    } catch (e) {
      setAudioState(snap);
      console.error('[audio] set_audio_volume failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  const setMasterMute = useCallback(async (muted: boolean) => {
    const snap = audioStateRef.current;
    setAudioState((prev) => (prev ? { ...prev, muted } : null));
    try {
      await invoke('set_audio_mute', { muted });
      setError(null);
    } catch (e) {
      setAudioState(snap);
      console.error('[audio] set_audio_mute failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  const setDefaultAudioDevice = useCallback(async (deviceId: string) => {
    try {
      await invoke('set_audio_default_endpoint', { deviceId });
      setError(null);
    } catch (e) {
      console.error('[audio] set_default_endpoint failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    }
  }, []);

  // ── WMAA / WMI MiInterface (elevated bridge) ──────────────────────────────
  // Direct EC access via WMI MiInterface — bypasses IoTDriver process check.
  // All commands require admin privileges (auto-elevated through the bridge).

  const wmiEcRead = useCallback(async (fun2: number, fun3: number) => {
    return invoke<WmaaResponse>('wmi_ec_read', { fun2, fun3 });
  }, []);

  const wmiEcWrite = useCallback(async (fun2: number, fun3: number, fun4: number) => {
    return invoke<WmaaResponse>('wmi_ec_write', { fun2, fun3, fun4 });
  }, []);

  const wmiEcGetPerformanceMode = useCallback(async () => {
    return invoke<string>('wmi_ec_get_performance_mode');
  }, []);

  const wmiEcSetPerformanceMode = useCallback(async (mode: number) => {
    await invoke('wmi_ec_set_performance_mode', { mode });
  }, []);

  const wmiEcReadBatteryHealth = useCallback(async () => {
    return invoke<number>('wmi_ec_read_battery_health');
  }, []);

  const wmiEcReadAdapterPower = useCallback(async () => {
    return invoke<number>('wmi_ec_read_adapter_power');
  }, []);

  const wmiEcReadSensorData = useCallback(async () => {
    return invoke<EcSensorData>('wmi_ec_read_sensor_data');
  }, []);

  const wmiEcSetBrightnessData = useCallback(async (level: number) => {
    await invoke('wmi_ec_set_brightness_data', { level });
  }, []);

  const wmiEcSetSagvMode = useCallback(async (mode: number) => {
    await invoke('wmi_ec_set_sagv_mode', { mode });
  }, []);

  const wmiEcSetPl1Flag = useCallback(async (enabled: boolean) => {
    await invoke('wmi_ec_set_pl1_flag', { enabled });
  }, []);

  const wmiEcSetEpofFlag = useCallback(async (enabled: boolean) => {
    await invoke('wmi_ec_set_epof_flag', { enabled });
  }, []);

  const wmiEcSetMiUsageType = useCallback(async (enabled: boolean) => {
    await invoke('wmi_ec_set_mi_usage_type', { enabled });
  }, []);

  const wmiEcSetWmidType = useCallback(async (val: number) => {
    await invoke('wmi_ec_set_wmid_type', { val });
  }, []);

  const wmiEcSetLidOpenType = useCallback(async (val: number) => {
    await invoke('wmi_ec_set_lid_open_type', { val });
  }, []);

  const wmiEcSetRemovableType = useCallback(async (val: number) => {
    await invoke('wmi_ec_set_removable_type', { val });
  }, []);

  const wmiEcSetAutoIllumination = useCallback(async (enabled: boolean) => {
    await invoke('wmi_ec_set_auto_illumination', { enabled });
  }, []);

  const wmiEcSetLabelMode = useCallback(async (enabled: boolean) => {
    await invoke('wmi_ec_set_label_mode', { enabled });
  }, []);

  // ── HQWmiCommonInterface (BIOS control via WMI) ──────────────────────────

  const hqSetPerformanceMode = useCallback(async (req: string) => {
    return await invoke<HqWmiResponse>('hq_set_performance_mode', { req });
  }, []);

  const hqChangeBootOption = useCallback(async (req: string) => {
    return await invoke<HqWmiResponse>('hq_change_boot_option', { req });
  }, []);

  const hqLoadDefault = useCallback(async (req: string) => {
    return await invoke<HqWmiResponse>('hq_load_default', { req });
  }, []);

  const hqS5RtcWakeEnable = useCallback(async (req: string) => {
    return await invoke<HqWmiResponse>('hq_s5_rtc_wake_enable', { req });
  }, []);

  const hqEnablePxeBoot = useCallback(async (req: string) => {
    return await invoke<HqWmiResponse>('hq_enable_pxe_boot', { req });
  }, []);

  const hqSetWifiCountryCode = useCallback(async (req: string) => {
    return await invoke<HqWmiResponse>('hq_set_wifi_country_code', { req });
  }, []);

  const hqSetShippingCountryCode = useCallback(async (req: string) => {
    return await invoke<HqWmiResponse>('hq_set_shipping_country_code', { req });
  }, []);

  // ── Thermal Zone (ACPI temperature) ──────────────────────────────────────

  const getThermalZones = useCallback(async () => {
    return await invoke<ThermalZoneInfo[]>('get_thermal_zones');
  }, []);

  const getPrimaryThermalZone = useCallback(async () => {
    return await invoke<ThermalZoneInfo>('get_primary_thermal_zone');
  }, []);

  useEffect(() => {
    void refreshHardwareProfile();
  }, [refreshHardwareProfile]);

  // ── Split useMemo into logical groups (S11-003, S24-014) ─────────────────
  // Each slice only re-renders when its own data changes.
  // Consumer components are wrapped in React.memo to prevent cascading re-renders.
  const fanState = useMemo(
    () => ({ fan, performanceMode, lastPerfResult, chargingThreshold }),
    [fan, performanceMode, lastPerfResult, chargingThreshold],
  );

  const batteryState = useMemo(() => battery, [battery]);
  const displayState = useMemo(() => display, [display]);
  const touchpadState = useMemo(() => touchpad, [touchpad]);
  const systemState = useMemo(() => systemInfo, [systemInfo]);

  // Flat wrapper: keeps the old interface working for existing callers
  /* eslint-disable react-hooks/exhaustive-deps -- stable callbacks; listing all would be noise */
  return useMemo(
    () => ({
      ...fanState,
      ...systemState,
      battery: batteryState,
      display: displayState,
      touchpad: touchpadState,
      systemInfo: systemState,
      fan: fanState.fan,
      performanceMode: fanState.performanceMode,
      lastPerfResult: fanState.lastPerfResult,
      chargingThreshold: fanState.chargingThreshold,
      loading,
      error,
      clearError,
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
      getIotDeviceInfo,
      getIotWifiList,
      iotNotifyEvent,
      audioState,
      getAudioState,
      setMasterVolume,
      setMasterMute,
    }),
    [
      fanState,
      systemState,
      batteryState,
      displayState,
      touchpadState,
      loading,
      error,
      clearError,
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
      getIotDeviceInfo,
      getIotWifiList,
      iotNotifyEvent,
      audioState,
      getAudioState,
      setMasterVolume,
      setMasterMute,
      setDefaultAudioDevice,
      // WMAA / WMI MiInterface (elevated)
      wmiEcRead,
      wmiEcWrite,
      wmiEcGetPerformanceMode,
      wmiEcSetPerformanceMode,
      wmiEcReadBatteryHealth,
      wmiEcReadAdapterPower,
      wmiEcReadSensorData,
      wmiEcSetBrightnessData,
      wmiEcSetSagvMode,
      wmiEcSetPl1Flag,
      wmiEcSetEpofFlag,
      wmiEcSetMiUsageType,
      wmiEcSetWmidType,
      wmiEcSetLidOpenType,
      wmiEcSetRemovableType,
      wmiEcSetAutoIllumination,
      wmiEcSetLabelMode,
      // HQWmiCommonInterface
      hqSetPerformanceMode,
      hqChangeBootOption,
      hqLoadDefault,
      hqS5RtcWakeEnable,
      hqEnablePxeBoot,
      hqSetWifiCountryCode,
      hqSetShippingCountryCode,
      // Thermal zone
      getThermalZones,
      getPrimaryThermalZone,
    ],
  );
  /* eslint-enable react-hooks/exhaustive-deps */
}
