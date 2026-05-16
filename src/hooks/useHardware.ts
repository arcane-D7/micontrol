import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";

// ── Type definitions matching Rust structs ───────────────────────────────────

export type PerformanceMode =
  | "silence"
  | "balance"
  | "turbo"
  | "smart"
  | "long_battery"
  | "decepticon"
  | "smart_acceleration";

export interface SystemInfo {
  cpu_name: string;
  cpu_cores: number;
  cpu_threads: number;
  cpu_usage: number;
  gpu_name: string;
  ram_total_gb: number;
  ram_used_gb: number;
  os_version: string;
}

export interface BatteryInfo {
  level: number;
  is_charging: boolean;
  is_plugged: boolean;
  health_percent: number;
  cycle_count: number;
  designed_capacity_mah: number;
  full_capacity_mah: number;
  manufacturer: string;
  device_name: string;
  temperature_celsius: number | null;
  time_remaining_minutes: number | null;
}

export interface AiBrightnessConfig {
  enabled: boolean;
  min_brightness: number;  // 5-80
  max_brightness: number;  // 20-100
  sensitivity: number;     // 10-200
  smoothing: number;       // 0-90
}

export interface DisplayInfo {
  brightness: number;
  hdr_enabled: boolean;
  refresh_rate_hz: number;
  ai_brightness: boolean;
  ai_brightness_config: AiBrightnessConfig;
}

export interface FanInfo {
  mode: "auto" | "fixed" | "off";
  speed_rpm: number;
  speed_percent: number;
  gpu_temp_celsius: number;
}

export interface TouchpadInfo {
  sensitivity: "low" | "medium" | "high";
  haptics_enabled: boolean;
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

// ── Hardware hook ────────────────────────────────────────────────────────────

export function useHardware() {
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null);
  const [battery, setBattery] = useState<BatteryInfo | null>(null);
  const [display, setDisplay] = useState<DisplayInfo | null>(null);
  const [fan, setFan] = useState<FanInfo | null>(null);
  const [touchpad, setTouchpad] = useState<TouchpadInfo | null>(null);
  const [performanceMode, setPerformanceModeState] =
    useState<PerformanceMode>("balance");
  const [chargingThreshold, setChargingThresholdState] = useState<number>(80);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus | null>(null);
  const [loadingUpdate, setLoadingUpdate] = useState(false);
  const [hardwareProfile, setHardwareProfile] = useState<HardwareProfile | null>(null);
  const [loadingDiscovery, setLoadingDiscovery] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [sys, bat, disp, fanData, tp, pm, ct] = await Promise.allSettled([
        invoke<SystemInfo>("get_system_info"),
        invoke<BatteryInfo>("get_battery_info"),
        invoke<DisplayInfo>("get_display_info"),
        invoke<FanInfo>("get_fan_info"),
        invoke<TouchpadInfo>("get_touchpad_info"),
        invoke<PerformanceMode>("get_performance_mode"),
        invoke<number>("get_charging_threshold"),
      ]);
      if (sys.status === "fulfilled") setSystemInfo(sys.value);
      if (bat.status === "fulfilled") setBattery(bat.value);
      if (disp.status === "fulfilled") setDisplay(disp.value);
      if (fanData.status === "fulfilled") setFan(fanData.value);
      if (tp.status === "fulfilled") setTouchpad(tp.value);
      if (pm.status === "fulfilled") setPerformanceModeState(pm.value);
      if (ct.status === "fulfilled") setChargingThresholdState(ct.value);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const interval = setInterval(() => void refresh(), 5000);
    return () => clearInterval(interval);
  }, [refresh]);

  const setPerformanceMode = useCallback(async (mode: PerformanceMode) => {
    await invoke("set_performance_mode", { mode });
    setPerformanceModeState(mode);
  }, []);

  const setChargingThreshold = useCallback(async (threshold: number) => {
    await invoke("set_charging_threshold", { threshold });
    setChargingThresholdState(threshold);
  }, []);

  const setBrightness = useCallback(async (level: number) => {
    await invoke("set_brightness", { level });
    setDisplay((prev) => (prev ? { ...prev, brightness: level } : null));
  }, []);

  const setHdr = useCallback(async (enabled: boolean) => {
    await invoke("set_hdr", { enabled });
    setDisplay((prev) => (prev ? { ...prev, hdr_enabled: enabled } : null));
  }, []);

  const setAiBrightness = useCallback(async (enabled: boolean) => {
    await invoke("set_ai_brightness", { enabled });
    setDisplay((prev) => (prev ? { ...prev, ai_brightness: enabled } : null));
  }, []);

  const setAiBrightnessConfig = useCallback(async (config: AiBrightnessConfig) => {
    await invoke("set_ai_brightness_config", { config });
    setDisplay((prev) => prev ? { ...prev, ai_brightness: config.enabled, ai_brightness_config: config } : null);
  }, []);

  const setFanMode = useCallback(
    async (mode: "auto" | "fixed" | "off", speedPercent?: number) => {
      await invoke("set_fan_mode", { mode, speed_percent: speedPercent ?? 50 });
      setFan((prev) =>
        prev ? { ...prev, mode, speed_percent: speedPercent ?? prev.speed_percent } : null
      );
    },
    []
  );

  const setTouchpadSensitivity = useCallback(
    async (sensitivity: "low" | "medium" | "high") => {
      await invoke("set_touchpad_sensitivity", { sensitivity });
      setTouchpad((prev) => (prev ? { ...prev, sensitivity } : null));
    },
    []
  );

  const setTouchpadHaptics = useCallback(async (enabled: boolean) => {
    await invoke("set_touchpad_haptics", { enabled });
    setTouchpad((prev) => (prev ? { ...prev, haptics_enabled: enabled } : null));
  }, []);

  // Update status is NOT polled — fetched once on mount + manually
  const refreshUpdateStatus = useCallback(async () => {
    setLoadingUpdate(true);
    try {
      const status = await invoke<UpdateStatus>("get_update_status");
      setUpdateStatus(status);
    } catch (e) {
      // non-fatal — update panel shows fallback
      console.warn("get_update_status error:", e);
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
      const profile = await invoke<HardwareProfile | null>("get_hardware_profile");
      setHardwareProfile(profile ?? null);
    } catch (e) {
      console.warn("get_hardware_profile error:", e);
    }
  }, []);

  const runHardwareDiscovery = useCallback(async () => {
    setLoadingDiscovery(true);
    try {
      const profile = await invoke<HardwareProfile>("run_hardware_discovery");
      setHardwareProfile(profile);
      return profile;
    } catch (e) {
      console.warn("run_hardware_discovery error:", e);
      throw e;
    } finally {
      setLoadingDiscovery(false);
    }
  }, []);

  const installDriver = useCallback(async (driverName: string) => {
    return invoke<string>("install_driver", { driverName });
  }, []);

  useEffect(() => {
    void refreshHardwareProfile();
  }, [refreshHardwareProfile]);

  return {
    systemInfo,
    battery,
    display,
    fan,
    touchpad,
    performanceMode,
    chargingThreshold,
    loading,
    error,
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
    updateStatus,
    loadingUpdate,
    refreshUpdateStatus,
    hardwareProfile,
    loadingDiscovery,
    refreshHardwareProfile,
    runHardwareDiscovery,
    installDriver,
  };
}
