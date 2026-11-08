/**
 * usePowerProfiles — lifecycle tests for the app-level Power Profiles hook.
 *
 * Regression target (audit 2026-11-07, P2-6): the AC/DC auto-switch used to be
 * a `useEffect` inside `PerformanceTab`, which `MainWindow` remounts on every
 * tab change (`key={activeTab}`). These tests pin the new contract:
 *  - the config mirror runs on the shell's timeline, independent of any tab;
 *  - the status poll runs on its own interval and on visibilitychange;
 *  - a hidden window does not poll;
 *  - a failed mirror is surfaced instead of being swallowed.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { usePowerProfiles, type ScenarioRulesView } from '../hooks/usePowerProfiles';
import { DEFAULT_SETTINGS } from '../hooks/useSettings';
import type { AppSettings } from '../types/settings';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

const mockedInvoke = vi.mocked(invoke);

function view(over: Partial<ScenarioRulesView> = {}): ScenarioRulesView {
  return {
    enabled: false,
    on_ac_mode: null,
    on_battery_mode: null,
    last_event: null,
    last_mode: null,
    last_ok: null,
    last_detail: null,
    last_at_unix_ms: null,
    applied_total: 0,
    failed_total: 0,
    ...over,
  };
}

function settings(patch: Partial<AppSettings> = {}): AppSettings {
  return { ...DEFAULT_SETTINGS, ...patch };
}

/** Invocations of one Tauri command, in order. */
function callsOf(cmd: string): unknown[][] {
  return (mockedInvoke.mock.calls as unknown[][]).filter((call) => call[0] === cmd);
}

function setVisibility(state: 'visible' | 'hidden') {
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    get: () => state,
  });
}

beforeEach(() => {
  mockedInvoke.mockReset();
  setVisibility('visible');
});

afterEach(() => {
  vi.useRealTimers();
  setVisibility('visible');
});

describe('usePowerProfiles', () => {
  it('mirrors the persisted settings into the user-scope registry key', async () => {
    mockedInvoke.mockResolvedValue(view({ enabled: true }));
    const updateKey = vi.fn();

    renderHook(() =>
      usePowerProfiles(
        settings({ auto_switch_perf: true, perf_mode_ac: 'turbo', perf_mode_dc: 'balance' }),
        updateKey,
      ),
    );

    await waitFor(() => expect(callsOf('set_scenario_rules')).toHaveLength(1));
    expect(callsOf('set_scenario_rules')[0][1]).toEqual({
      enabled: true,
      onAcMode: 'turbo',
      onBatteryMode: 'balance',
    });
  });

  it('re-mirrors when the toggle changes, without a remount', async () => {
    mockedInvoke.mockResolvedValue(view());
    const updateKey = vi.fn();

    const { rerender } = renderHook(({ s }: { s: AppSettings }) => usePowerProfiles(s, updateKey), {
      initialProps: { s: settings() },
    });
    await waitFor(() => expect(callsOf('set_scenario_rules')).toHaveLength(1));

    rerender({ s: settings({ auto_switch_perf: true, perf_mode_ac: 'turbo' }) });

    await waitFor(() => expect(callsOf('set_scenario_rules')).toHaveLength(2));
    expect(callsOf('set_scenario_rules')[1][1]).toEqual({
      enabled: true,
      onAcMode: 'turbo',
      onBatteryMode: null,
    });
  });

  it('subscribes to backend status and never applies a mode itself', async () => {
    mockedInvoke.mockImplementation(async (cmd: string) =>
      cmd === 'get_scenario_rules'
        ? view({ last_event: 'ac_on', last_mode: 'turbo', last_ok: true, applied_total: 1 })
        : view(),
    );

    const { result } = renderHook(() => usePowerProfiles(settings(), vi.fn()));

    await waitFor(() => expect(result.current.status?.last_event).toBe('ac_on'));
    expect(result.current.status?.applied_total).toBe(1);

    // The hook must never call the applying command: Rust owns the switch loop.
    expect(callsOf('set_performance_mode')).toHaveLength(0);
  });

  it('polls on its own clock, with no tab in the picture', async () => {
    vi.useFakeTimers();
    mockedInvoke.mockResolvedValue(view());

    renderHook(() => usePowerProfiles(settings(), vi.fn()));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    const before = callsOf('get_scenario_rules').length;
    expect(before).toBeGreaterThanOrEqual(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(15_000);
    });
    expect(callsOf('get_scenario_rules').length).toBe(before + 1);
  });

  it('skips the status poll while the window is hidden, refreshes on show', async () => {
    mockedInvoke.mockResolvedValue(view());

    renderHook(() => usePowerProfiles(settings(), vi.fn()));
    await waitFor(() => expect(callsOf('get_scenario_rules')).toHaveLength(1));

    setVisibility('hidden');
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
      await Promise.resolve();
    });
    expect(callsOf('get_scenario_rules')).toHaveLength(1);

    setVisibility('visible');
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
      await Promise.resolve();
    });
    await waitFor(() => expect(callsOf('get_scenario_rules')).toHaveLength(2));
  });

  it('surfaces a failed mirror instead of silently dropping the toggle', async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'set_scenario_rules') throw new Error('access denied');
      return view();
    });

    const { result } = renderHook(() => usePowerProfiles(settings(), vi.fn()));

    await waitFor(() => expect(result.current.syncError).toContain('access denied'));
  });

  it('exposes setters that write through the settings hook', () => {
    // Never resolve: this test only exercises the setters, and a late resolve
    // would schedule a state update after the test has finished.
    mockedInvoke.mockImplementation(() => new Promise(() => {}));
    const updateKey = vi.fn();

    const { result } = renderHook(() => usePowerProfiles(settings(), updateKey));

    act(() => result.current.setEnabled(true));
    act(() => result.current.setAcMode('turbo'));
    act(() => result.current.setDcMode(null));

    expect(updateKey).toHaveBeenCalledWith('auto_switch_perf', true);
    expect(updateKey).toHaveBeenCalledWith('perf_mode_ac', 'turbo');
    expect(updateKey).toHaveBeenCalledWith('perf_mode_dc', null);
  });
});
