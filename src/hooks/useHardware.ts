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
  FanModeResult,
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
  FanModeResult,
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
  AdapterPowerReading,
  HqWmiResponse,
  ThermalZoneInfo,
  ProcessTaskInfo,
  NetworkInterfaceSample,
} from '../types/hardware';

import { createPollGuard, type PollGuard } from '../lib/pollGuard';
import { runBounded } from '../lib/boundedPoll';

// ── Hardware hook ────────────────────────────────────────────────────────────

export function useHardware() {
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null);
  const [battery, setBattery] = useState<BatteryInfo | null>(null);
  const [display, setDisplay] = useState<DisplayInfo | null>(null);
  const [fan, setFan] = useState<FanInfo | null>(null);
  const [touchpad, setTouchpad] = useState<TouchpadInfo | null>(null);
  const [performanceMode, setPerformanceModeState] = useState<PerformanceMode>('balance');
  const [lastPerfResult, setLastPerfResult] = useState<PerformanceResult | null>(null);
  // True while a set_performance_mode round-trip is in flight (async
  // confirmation). Drives the "applying" UI state — replaces the old
  // setTimeout-based mode switching.
  const [perfApplying, setPerfApplying] = useState(false);
  // Perf-mode writes can take seconds (elevated bridge round-trip). Until the
  // confirmed result arrives (plus a small grace), the slow poll must NOT
  // overwrite the optimistic state — otherwise the UI "jumps" back to the old
  // mode mid-transition (async fix, no physical timer).
  const perfWriteDirtyUntil = useRef<number>(0);
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
  const fastPollInFlightRef = useRef(false);
  const systemPollInFlightRef = useRef(false);
  const slowPollInFlightRef = useRef(false);
  const initialLoadInFlightRef = useRef(false);
  // Cancellation for in-flight polls: React state must not be written after
  // unmount, and a slow-poll batch that lost its race must not clobber a
  // newer batch (STAB-ROUND-1, P1-c).
  const mountedRef = useRef(true);
  const slowPollEpochRef = useRef(0);
  // Watchdogs: if a poll leg hangs (e.g. WMI wedged after sleep), the in-flight
  // flag stays true forever and every later poll is skipped → CPU/GPU freeze in
  // the overview. Storing the start time lets the next tick detect the stall and
  // reset the flag (the abandoned promise keeps running, but cannot block us).
  const fastPollStartedAtRef = useRef(0);
  const systemPollStartedAtRef = useRef(0);
  const slowPollStartedAtRef = useRef(0);
  // Hard cap for one fan/audio poll leg. get_fan_info can take very long when
  // WMI is degraded; beyond this we abandon it and keep the fast tier moving.
  const FAST_POLL_LEG_MS = 6000;
  const STALL_MS = 8000;

  // Fan leg guard (dogfood H-04). The old Promise.race abandoned the *result*
  // but not the call: every 2 s a new get_fan_info piled on top of the still
  // running one (2 221 abandoned calls / 74 min). The guard allows one call in
  // flight and backs off 6 s → 12 s → … → 60 s after a timeout; a read that
  // returns resets the backoff. Created once per hook instance.
  const fanGuardRef = useRef<PollGuard | null>(null);
  if (!fanGuardRef.current) {
    fanGuardRef.current = createPollGuard({
      timeoutMs: FAST_POLL_LEG_MS,
      baseBackoffMs: FAST_POLL_LEG_MS,
      maxBackoffMs: 60_000,
      onTimeout: ({ streak, retryInMs }) =>
        console.warn(
          `[hardware] get_fan_info exceeded ${FAST_POLL_LEG_MS} ms — abandoned` +
            ` (streak ${streak}, next attempt in ${retryInMs} ms)`,
        ),
    });
  }

  // ── Tiered polling (S11-002) ─────────────────────────────────────────────
  // Fast tier: 2 s — CPU usage, GPU usage, RAM (get_system_info) + fan speed,
  // CPU temp, GPU temp, audio. get_system_info is polled on its OWN leg so a
  // slow/hung get_fan_info can never freeze the overview CPU/GPU values.
  // Slow tier: 15 s — battery, display, touchpad
  // Poll-once: system info — fetched on mount only

  const FAST_POLL_INTERVAL = 2000;
  const SLOW_POLL_INTERVAL = 15000;
  // STAB-ROUND-1 (P1-c): five invokes used to hit the backend on the same
  // tick every 15 s. Two in flight plus a staggered start is enough for the
  // slow tier, and stops the batch from flooding the blocking pool exactly
  // when a stalled battery query is already holding it.
  const SLOW_POLL_CONCURRENCY = 2;
  const SLOW_POLL_JITTER_MS = 400;

  // Safe wrapper: catches per-invoke failures so one failing query doesn't
  // abort the entire batch. Returns the fallback value on failure. If the
  // query is essential (fallback is null → no data for that hardware), the
  // error is surfaced so the user sees why a feature is unavailable.
  const safe = useCallback(
    <T>(p: Promise<T>, fallback: T, label: string): Promise<T> =>
      p.catch((e) => {
        console.warn(`[hardware] ${label} failed:`, e);
        if (fallback === null) {
          setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
        }
        return fallback;
      }),
    [],
  );

  /** One bounded `get_fan_info`. `null` = skipped, timed out or failed. */
  const pollFan = useCallback(
    async (label: string): Promise<FanInfo | null> => {
      const guard = fanGuardRef.current;
      if (!guard) return null;
      const outcome = await guard.run(() => safe(invoke<FanInfo>('get_fan_info'), null, label));
      return outcome.status === 'ok' ? outcome.value : null;
    },
    [safe],
  );

  // System leg (CPU/GPU/RAM) — polled alone so a hung get_fan_info cannot
  // freeze the overview. This is the hottest, cheapest query (PDH caches).
  const fastPollSystem = useCallback(async () => {
    // Stall watchdog: a previous leg may have hung with no timeout (abandoned
    // promise never resolves) → reset the flag so polling keeps working.
    if (systemPollInFlightRef.current && systemPollStartedAtRef.current) {
      if (Date.now() - systemPollStartedAtRef.current > STALL_MS) {
        systemPollInFlightRef.current = false;
      } else {
        return;
      }
    }
    if (systemPollInFlightRef.current || initialLoadInFlightRef.current) return;
    systemPollInFlightRef.current = true;
    systemPollStartedAtRef.current = Date.now();
    try {
      const systemResult = await safe(
        invoke<SystemInfo>('get_system_info'),
        null,
        'get_system_info',
      );
      if (systemResult) setSystemInfo(systemResult);
    } finally {
      systemPollInFlightRef.current = false;
    }
  }, [safe]);

  const fastPoll = useCallback(async () => {
    // Stall watchdog (see fastPollSystem).
    if (fastPollInFlightRef.current && fastPollStartedAtRef.current) {
      if (Date.now() - fastPollStartedAtRef.current > STALL_MS) {
        fastPollInFlightRef.current = false;
      } else {
        return;
      }
    }
    if (fastPollInFlightRef.current || initialLoadInFlightRef.current) return;
    fastPollInFlightRef.current = true;
    fastPollStartedAtRef.current = Date.now();
    try {
      const [fanResult, audioResult] = await Promise.all([
        // Bounded fan leg: one call in flight, exponential backoff after a
        // timeout (see fanGuardRef). A skipped leg keeps the last value.
        pollFan('get_fan_info'),
        safe(invoke<AudioVolumeResult>('get_audio_volume'), null, 'get_audio_volume'),
      ]);
      if (fanResult) setFan(fanResult);
      if (audioResult) setAudioState(audioResult);
      // Do NOT clear the error here. Polls run independently; a successful fast
      // poll must not erase an error surfaced by another reading's failure.
      // Errors are cleared by an all-success initial load or clearError().
    } finally {
      fastPollInFlightRef.current = false;
    }
  }, [safe, pollFan]);

  const slowPoll = useCallback(async () => {
    if (slowPollInFlightRef.current && slowPollStartedAtRef.current) {
      if (Date.now() - slowPollStartedAtRef.current > STALL_MS) {
        slowPollInFlightRef.current = false;
      } else {
        return;
      }
    }
    if (slowPollInFlightRef.current || initialLoadInFlightRef.current) return;
    slowPollInFlightRef.current = true;
    slowPollStartedAtRef.current = Date.now();
    const epoch = ++slowPollEpochRef.current;
    const cancelled = () => !mountedRef.current || slowPollEpochRef.current !== epoch;
    try {
      // P1-c: bounded concurrency + jitter instead of five simultaneous
      // invokes. Each leg is already `safe()`-wrapped, so a rejected leg is a
      // null value, not an exception; `skipped` (cancelled) is just ignored.
      const outcomes = await runBounded<unknown>(
        [
          () => safe(invoke<BatteryInfo>('get_battery_info'), null, 'get_battery_info'),
          () => safe(invoke<DisplayInfo>('get_display_info'), null, 'get_display_info'),
          () => safe(invoke<TouchpadInfo>('get_touchpad_info'), null, 'get_touchpad_info'),
          () => safe(invoke<PerformanceMode>('get_performance_mode'), null, 'get_performance_mode'),
          () => safe(invoke<number>('get_charging_threshold'), 80, 'get_charging_threshold'),
        ],
        {
          limit: SLOW_POLL_CONCURRENCY,
          jitterMs: SLOW_POLL_JITTER_MS,
          isCancelled: cancelled,
        },
      );
      // No state writes after unmount, and no stale batch overwriting a newer
      // one (the stall watchdog can release the in-flight flag while an old
      // batch is still pending).
      if (cancelled()) return;
      const valueOf = <T>(index: number): T | null => {
        const outcome = outcomes[index];
        return outcome && outcome.status === 'fulfilled' ? (outcome.value as T) : null;
      };
      const batteryResult = valueOf<BatteryInfo>(0);
      const displayResult = valueOf<DisplayInfo>(1);
      const touchpadResult = valueOf<TouchpadInfo>(2);
      const perfMode = valueOf<PerformanceMode>(3);
      const chargeThreshold = valueOf<number>(4);
      if (batteryResult !== null) setBattery(batteryResult);
      if (displayResult !== null) setDisplay(displayResult);
      // Only update touchpad from poll when no user write is in flight.
      if (touchpadResult !== null && Date.now() >= touchpadDirtyUntil.current) {
        setTouchpad(touchpadResult);
      }
      // Same guard for perf mode: never let a poll clobber an in-flight (or
      // recently confirmed) user change with stale data from the old mode.
      if (perfMode && Date.now() >= perfWriteDirtyUntil.current) {
        setPerformanceModeState(perfMode);
      }
      if (chargeThreshold !== null) setChargingThresholdState(chargeThreshold);
      // Do NOT clear the error here (see fastPoll). Only the initial load with
      // full success or clearError() clears the error state.
    } finally {
      slowPollInFlightRef.current = false;
    }
  }, [safe]);

  const initialLoadRef = useRef(false);

  useEffect(() => {
    mountedRef.current = true;
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
      initialLoadInFlightRef.current = true;
      const initialLoadStartedAt = Date.now();
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
            // Bounded fan leg at boot too, through the same guard: get_fan_info
            // can hang when WMI is wedged — never let it pin
            // initialLoadInFlightRef forever (which would block ALL later polls
            // → frozen overview), and never stack a second boot read on it.
            pollFan('init:get_fan_info'),
            safe(invoke<SystemInfo>('get_system_info'), null, 'init:get_system_info'),
            safe(invoke<BatteryInfo>('get_battery_info'), null, 'init:get_battery_info'),
            safe(invoke<DisplayInfo>('get_display_info'), null, 'init:get_display_info'),
            safe(invoke<TouchpadInfo>('get_touchpad_info'), null, 'init:get_touchpad_info'),
            safe(
              invoke<PerformanceMode>('get_performance_mode'),
              null,
              'init:get_performance_mode',
            ),
            safe(invoke<number>('get_charging_threshold'), 80, 'init:get_charging_threshold'),
          ]);
          if (fanResult) setFan(fanResult);
          if (systemResult) setSystemInfo(systemResult);
          if (batteryResult !== null) setBattery(batteryResult);
          if (displayResult !== null) setDisplay(displayResult);
          if (touchpadResult !== null) setTouchpad(touchpadResult);
          if (perfMode) setPerformanceModeState(perfMode);
          setChargingThresholdState(chargeThreshold);
          // Only clear the error when all essential readings succeeded; a
          // safe() miss for hardware data keeps the error visible.
          if (fanResult && systemResult && batteryResult !== null && touchpadResult !== null) {
            setError(null);
          }

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
          initialLoadInFlightRef.current = false;
          setLoading(false);
        }
      };
      void doInitialLoad();
      // Watchdog: if the whole initial load is still in flight past the stall
      // window (some non-fan leg hung), release the flag so polling starts even
      // if the abandoned promise never resolves. get_system_info will fill in
      // the overview within 2 s regardless.
      setTimeout(
        () => {
          if (initialLoadInFlightRef.current) {
            const elapsed = Date.now() - initialLoadStartedAt;
            if (elapsed > STALL_MS + FAST_POLL_LEG_MS) {
              console.warn(
                '[hardware] Initial load exceeded',
                elapsed,
                'ms — releasing polling (abandoned load keeps running in background)',
              );
              initialLoadInFlightRef.current = false;
            }
          }
        },
        STALL_MS + FAST_POLL_LEG_MS + 1000,
      );
    }

    // Fast polling interval: the system leg (CPU/GPU/RAM) runs on its own
    // every tick, so a slow/hung fan query can never delay the overview.
    const systemInterval = setInterval(() => refreshIfVisible(fastPollSystem), FAST_POLL_INTERVAL);
    const fastInterval = setInterval(() => refreshIfVisible(fastPoll), FAST_POLL_INTERVAL);

    // Slow polling interval
    const slowInterval = setInterval(() => refreshIfVisible(slowPoll), SLOW_POLL_INTERVAL);

    const onVisibilityChange = () => {
      if (typeof document !== 'undefined' && document.visibilityState === 'visible') {
        void fastPollSystem();
        void fastPoll();
        void slowPoll();
      }
    };
    const onWindowFocus = () => {
      void fastPollSystem();
      void fastPoll();
      void slowPoll();
    };
    document.addEventListener('visibilitychange', onVisibilityChange);
    window.addEventListener('focus', onWindowFocus);

    return () => {
      // Cancel queued/in-flight slow-poll legs: the epoch check makes any
      // result that lands after this point a no-op (P1-c).
      mountedRef.current = false;
      slowPollEpochRef.current += 1;
      document.removeEventListener('visibilitychange', onVisibilityChange);
      window.removeEventListener('focus', onWindowFocus);
      clearInterval(systemInterval);
      clearInterval(fastInterval);
      clearInterval(slowInterval);
    };
  }, [fastPoll, fastPollSystem, slowPoll, safe, pollFan]);

  const setPerformanceMode = useCallback(async (mode: PerformanceMode) => {
    const snap = performanceModeRef.current;
    // Optimistic UI + guard: polls must not clobber the selection during the
    // elevated round-trip (which can take seconds). No physical timer — the
    // confirmed `result.mode` from the backend decides the final state.
    perfWriteDirtyUntil.current = Date.now() + 8000;
    setPerfApplying(true);
    setPerformanceModeState(mode);
    try {
      const result = await invoke<PerformanceResult>('set_performance_mode', { mode });
      setLastPerfResult(result);
      // Apply the CONFIRMED mode (source of truth from the bridge) — this is
      // the async confirmation that ends the transition.
      setPerformanceModeState(result.mode);
      perfWriteDirtyUntil.current = Date.now() + 1500; // grace after settle
      setError(null);
    } catch (e) {
      perfWriteDirtyUntil.current = 0;
      setPerformanceModeState(snap);
      console.error('[perf] set_performance_mode failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      throw e;
    } finally {
      setPerfApplying(false);
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

  /** Re-read the battery once (stale-state retry button). Additive: no contract change. */
  const refreshBattery = useCallback(async () => {
    const result = await safe(
      invoke<BatteryInfo>('get_battery_info'),
      null,
      'refresh:get_battery_info',
    );
    if (result !== null) {
      setBattery(result);
      setError(null);
    }
    return result;
  }, [safe]);

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

  /**
   * Apply a fan mode and return what actually happened.
   *
   * The optimistic update only touches `mode` and the *requested* percentage —
   * measured RPM/percentage stay untouched, because they are readback values
   * and only the next poll can change them.
   */
  const setFanMode = useCallback(
    async (mode: 'auto' | 'fixed' | 'off', speedPercent?: number): Promise<FanModeResult> => {
      const snap = fanRef.current;
      setFan((prev) =>
        prev
          ? {
              ...prev,
              mode,
              requested_speed_percent: speedPercent ?? prev.requested_speed_percent,
            }
          : null,
      );
      try {
        // Tauri v2 converts snake_case Rust parameters to camelCase JS keys.
        // The command declares `speed_percent: u8`, so the payload key MUST be
        // `speedPercent` — `speed_percent` is silently unmatched and the whole
        // call fails with "missing required key speedPercent" (dogfood: every
        // fan mode/fixed-speed change failed this way). Pinned by
        // useHardware.setFanMode tests.
        const result = await invoke<FanModeResult>('set_fan_mode', {
          mode,
          speedPercent: speedPercent ?? 50,
        });
        setError(null);
        return result;
      } catch (e) {
        setFan(snap);
        console.error('[fan] set_fan_mode failed:', e);
        setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
        throw e;
      }
    },
    [],
  );

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

  /**
   * Global task-manager snapshot: every process with per-PID CPU/GPU/NPU/RAM.
   * Sorted by CPU% descending, top 100.
   */
  const getTaskManager = useCallback(async () => {
    try {
      const result = await invoke<ProcessTaskInfo[]>('get_task_manager');
      setError(null);
      return result;
    } catch (e) {
      console.error('[sys] get_task_manager failed:', e);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
      return [];
    }
  }, []);

  /** Kill a process by PID (Task Manager "End task"). */
  const killProcess = useCallback(async (pid: number) => {
    const result = await invoke<void>('kill_process', { pid });
    return result;
  }, []);

  /** Network throughput per interface (bytes/sec, up+down combined). */
  const getNetworkPerf = useCallback(async () => {
    try {
      const result = await invoke<NetworkInterfaceSample[]>('get_network_perf');
      setError(null);
      return result;
    } catch (e) {
      console.error('[sys] get_network_perf failed:', e);
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
      // Do NOT clear a hardware error here — this auto-runs on mount and
      // would erase a real hardware failure surfaced by the initial load.
      // Errors are cleared by an all-success initial load or clearError().
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
      // Do NOT clear a hardware error here — see refreshUpdateStatus.
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
    return invoke<AdapterPowerReading>('wmi_ec_read_adapter_power');
  }, []);

  const wmiEcReadSensorData = useCallback(async () => {
    return invoke<EcSensorData>('wmi_ec_read_sensor_data');
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
      // A perna rápida de sistema (CPU/GPU/RAM) exposta para refresco a pedido
      // (ex.: o bloco "Consumo vivo" do Overview). Tem as mesmas guardas de
      // in-flight/stall da perna — chamar isto nunca abre uma segunda sampler.
      refreshSystem: fastPollSystem,
      perfApplying,
      loading,
      error,
      clearError,
      setPerformanceMode,
      setChargingThreshold,
      refreshBattery,
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
      getTaskManager,
      killProcess,
      getNetworkPerf,
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
    }),
    [
      fanState,
      systemState,
      batteryState,
      displayState,
      touchpadState,
      fastPollSystem,
      perfApplying,
      loading,
      error,
      clearError,
      setPerformanceMode,
      setChargingThreshold,
      refreshBattery,
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
      getTaskManager,
      killProcess,
      getNetworkPerf,
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
