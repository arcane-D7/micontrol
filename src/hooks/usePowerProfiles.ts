/**
 * usePowerProfiles — app-level lifecycle for the "Power Profiles" toggle.
 *
 * ## Why this hook exists (audit 2026-11-07, P2-6)
 *
 * The AC/DC auto-switch used to be a `useEffect` **inside** `PerformanceTab`.
 * `MainWindow` renders tabs with `key={activeTab}`, so the effect — and the
 * toggle's only consumer — died every time the user left the Performance tab,
 * and the hardware poll that fed it stops while the window is hidden. The
 * toggle was therefore decorative: it only worked while the user was staring at
 * the tab.
 *
 * The hook is mounted once by the shell (`MainWindow`), so it outlives tab
 * switches, and it does **not** own the switch loop at all:
 *
 * | half | owner | where |
 * |---|---|---|
 * | apply the mode on AC/DC | `hw::scenario_rules` (Rust, 5 s IoT tick) | elevated dispatcher |
 * | config (`RulesEnabled`, AC/DC modes) | this hook | `HKCU\SOFTWARE\MiControl\IotEvents` via `set_scenario_rules` |
 * | display | this hook (subscribe) | `get_scenario_rules` poll |
 *
 * That split is what makes the feature survive: the Rust listener runs without
 * the WebView, without the window, and without the tab. The hook only mirrors
 * the persisted settings into the user-scope registry key and watches the
 * backend status so the card can show what actually happened.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { AppSettings } from '../types/settings';
import type { PerformanceMode } from '../types/hardware';

/** Backend view — mirrors `ScenarioRulesView` in `commands/hardware.rs`. */
export interface ScenarioRulesView {
  enabled: boolean;
  on_ac_mode: PerformanceMode | null;
  on_battery_mode: PerformanceMode | null;
  last_event: string | null;
  last_mode: PerformanceMode | null;
  last_ok: boolean | null;
  last_detail: string | null;
  last_at_unix_ms: number | null;
  applied_total: number;
  failed_total: number;
}

export interface PowerProfilesController {
  /** Toggle state (source of truth for the UI; persisted in settings). */
  enabled: boolean;
  acMode: PerformanceMode | null;
  dcMode: PerformanceMode | null;
  /** Last backend status snapshot, or null before the first successful read. */
  status: ScenarioRulesView | null;
  /** Last HKCU mirror error, or null. */
  syncError: string | null;
  setEnabled: (enabled: boolean) => void;
  setAcMode: (mode: PerformanceMode | null) => void;
  setDcMode: (mode: PerformanceMode | null) => void;
}

/** How often the UI re-reads the backend status while visible. */
const STATUS_POLL_MS = 15_000;

export function usePowerProfiles(
  settings: AppSettings,
  updateKey: <K extends keyof AppSettings>(key: K, value: AppSettings[K]) => void,
): PowerProfilesController {
  const [status, setStatus] = useState<ScenarioRulesView | null>(null);
  const [syncError, setSyncError] = useState<string | null>(null);

  const enabled = settings.auto_switch_perf;
  const acMode = settings.perf_mode_ac;
  const dcMode = settings.perf_mode_dc;

  // ── Mirror the config into the user-scope registry key ────────────────────
  // The Rust listener reads HKCU; localStorage alone is invisible to it. The
  // write is cheap (no elevation) and idempotent, so it is safe to re-run on
  // every settings change.
  const lastSyncedRef = useRef<string>('');
  useEffect(() => {
    const signature = JSON.stringify([enabled, acMode, dcMode]);
    if (signature === lastSyncedRef.current) return;
    lastSyncedRef.current = signature;

    let cancelled = false;
    void (async () => {
      try {
        const view = await invoke<ScenarioRulesView>('set_scenario_rules', {
          enabled,
          onAcMode: acMode ?? null,
          onBatteryMode: dcMode ?? null,
        });
        if (cancelled) return;
        setStatus(view);
        setSyncError(null);
      } catch (e) {
        if (cancelled) return;
        // A failed mirror must not break the settings UI: keep local state and
        // surface the reason so the card can show "not synced".
        setSyncError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [enabled, acMode, dcMode]);

  // ── Subscribe to the backend status ───────────────────────────────────────
  const refresh = useCallback(async () => {
    try {
      const view = await invoke<ScenarioRulesView>('get_scenario_rules');
      setStatus(view);
    } catch {
      // Backend unavailable (or a test/window without Tauri IPC): keep the
      // last known snapshot instead of blanking the card.
    }
  }, []);

  useEffect(() => {
    const refreshIfVisible = () => {
      if (typeof document !== 'undefined' && document.visibilityState !== 'visible') {
        return;
      }
      void refresh();
    };

    refreshIfVisible();
    const id = window.setInterval(refreshIfVisible, STATUS_POLL_MS);
    document.addEventListener('visibilitychange', refreshIfVisible);
    return () => {
      window.clearInterval(id);
      document.removeEventListener('visibilitychange', refreshIfVisible);
    };
  }, [refresh]);

  const setEnabled = useCallback(
    (value: boolean) => updateKey('auto_switch_perf', value),
    [updateKey],
  );
  const setAcMode = useCallback(
    (mode: PerformanceMode | null) => updateKey('perf_mode_ac', mode),
    [updateKey],
  );
  const setDcMode = useCallback(
    (mode: PerformanceMode | null) => updateKey('perf_mode_dc', mode),
    [updateKey],
  );

  return {
    enabled,
    acMode,
    dcMode,
    status,
    syncError,
    setEnabled,
    setAcMode,
    setDcMode,
  };
}
