import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { useHardware } from '../hooks/useHardware';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

// Suppress console.error from the hook's catch blocks during error tests

const mockFanInfo = {
  mode: 'auto' as const,
  speed_rpm: 3000,
  speed_percent: 50,
  gpu_temp_celsius: 60,
  cpu_temp_celsius: 70,
  tdp_watts: 45,
};
const mockSystemInfo = {
  cpu_name: 'i7',
  cpu_cores: 8,
  cpu_threads: 16,
  cpu_usage: 30,
  gpu_name: 'RTX',
  gpu_usage: 20,
  vram_used_mb: 1000,
  ram_total_gb: 32,
  ram_used_gb: 16,
  os_version: 'Win11',
};
const mockBatteryInfo = {
  level: 80,
  is_charging: true,
  is_plugged: true,
  health_percent: 95,
  cycle_count: 100,
  designed_capacity_mwh: 80000,
  full_capacity_mwh: 76000,
  manufacturer: 'Xiaomi',
  device_name: 'Battery',
  temperature_celsius: 30,
  time_remaining_minutes: 120,
  time_to_full_minutes: null,
  charge_rate_mw: 50000,
  voltage_mv: 12000,
  ac_input_power_mw: 65000,
};
const mockDisplayInfo = {
  brightness: 80,
  hdr_enabled: false,
  refresh_rate_hz: 60,
  available_refresh_rates: [60, 120],
  dynamic_refresh_rate_capable: false,
  adaptive_refresh_rate: false,
  ai_brightness: false,
  ai_brightness_config: {
    enabled: false,
    min_brightness: 5,
    max_brightness: 100,
    sensitivity: 100,
    smoothing: 50,
    mode: 'curve',
  },
  ambient_lux: null,
};
const mockTouchpadInfo = {
  sensitivity: 'medium' as const,
  haptics_enabled: true,
  haptics_intensity: 'medium' as const,
  gesture_screenshot: true,
  trackpad_repress: false,
  edge_slide: true,
};
const mockUpdateStatus = {
  bios: {
    version: '1.0',
    release_date: '2024-01-01',
    manufacturer: 'Xiaomi',
    serial_number: 'SN123',
  },
  xiaomi_drivers: [],
  last_xpm_scan: null,
  xpm_driver_cache: {},
  xpm_installed: false,
  xpm_version: null,
  xpm_path: null,
};
const mockHardwareProfile = {
  discovered_at: Date.now(),
  device_model: 'Xiaomi Book',
  vhf_device_path: null,
  touchpad_hid_path: null,
  touchscreen_hid_path: null,
  stylus_hid_path: null,
  iot_pipe_path: null,
  iot_service_name: null,
  igcl_dll_path: null,
  mi_registry_present: true,
  missing_drivers: [],
  capabilities: {
    has_vhf_performance: true,
    has_touchpad_hid: true,
    has_touchscreen: false,
    has_stylus: false,
    has_igcl: false,
    has_iot_charging: true,
    has_mi_registry: true,
  },
};

function mockInvokeImplementation(cmd: string): Promise<unknown> {
  switch (cmd) {
    case 'get_fan_info':
      return Promise.resolve(mockFanInfo);
    case 'get_system_info':
      return Promise.resolve(mockSystemInfo);
    case 'get_battery_info':
      return Promise.resolve(mockBatteryInfo);
    case 'get_display_info':
      return Promise.resolve(mockDisplayInfo);
    case 'get_touchpad_info':
      return Promise.resolve(mockTouchpadInfo);
    case 'get_update_status':
      return Promise.resolve(mockUpdateStatus);
    case 'get_hardware_profile':
      return Promise.resolve(mockHardwareProfile);
    default:
      return Promise.resolve({});
  }
}

describe('useHardware', () => {
  let consoleErrorSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(invoke).mockImplementation(mockInvokeImplementation as never);
    consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
  });

  afterEach(() => {
    consoleErrorSpy.mockRestore();
  });

  it('initial load sets loading=true then false after data arrives', async () => {
    const { result } = renderHook(() => useHardware());

    expect(result.current.loading).toBe(true);

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.fan).toEqual(mockFanInfo);
    expect(result.current.systemInfo).toEqual(mockSystemInfo);
    expect(result.current.battery).toEqual(mockBatteryInfo);
    expect(result.current.display).toEqual(mockDisplayInfo);
    expect(result.current.touchpad).toEqual(mockTouchpadInfo);
  });

  it('error state is set when invoke throws', async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string) => {
      if (cmd === 'get_fan_info') return Promise.reject('Hardware unavailable');
      return mockInvokeImplementation(cmd);
    }) as never);

    const { result } = renderHook(() => useHardware());

    await waitFor(() => {
      expect(result.current.error).not.toBeNull();
    });

    expect(result.current.error).toContain('Hardware unavailable');
  });

  it('setPerformanceMode calls invoke and updates state optimistically', async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'set_performance_mode') {
        return Promise.resolve({
          success: true,
          method: 'vhf',
          mode: args?.mode,
        });
      }
      return mockInvokeImplementation(cmd);
    }) as never);

    const { result } = renderHook(() => useHardware());

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    await act(async () => {
      await result.current.setPerformanceMode('turbo');
    });

    expect(invoke).toHaveBeenCalledWith('set_performance_mode', { mode: 'turbo' });
    expect(result.current.performanceMode).toBe('turbo');
    expect(result.current.lastPerfResult).toEqual({
      success: true,
      method: 'vhf',
      mode: 'turbo',
    });
  });

  it('setBrightness calls invoke and reverts on error', async () => {
    const originalBrightness = mockDisplayInfo.brightness;

    vi.mocked(invoke).mockImplementation(((cmd: string) => {
      if (cmd === 'set_brightness') {
        return Promise.reject('Brightness set failed');
      }
      return mockInvokeImplementation(cmd);
    }) as never);

    const { result } = renderHook(() => useHardware());

    // Wait for initial load to complete
    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });
    // Wait for hardwareProfile to be loaded (refreshHardwareProfile completes)
    await waitFor(() => {
      expect(result.current.hardwareProfile).not.toBeNull();
    });
    // Wait for updateStatus to finish (has a 2s delay) so it doesn't clear our error
    await waitFor(
      () => {
        expect(result.current.loadingUpdate).toBe(false);
      },
      { timeout: 5000 },
    );

    await expect(
      act(async () => {
        await result.current.setBrightness(50);
      }),
    ).rejects.toThrow('Brightness set failed');

    expect(invoke).toHaveBeenCalledWith('set_brightness', { level: 50 });
    expect(result.current.display?.brightness).toBe(originalBrightness);
  });

  it('clearError clears the error state', async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string) => {
      if (cmd === 'get_fan_info') return Promise.reject('Init error');
      return mockInvokeImplementation(cmd);
    }) as never);

    const { result } = renderHook(() => useHardware());

    await waitFor(() => {
      expect(result.current.error).not.toBeNull();
    });

    act(() => {
      result.current.clearError();
    });

    expect(result.current.error).toBeNull();
  });
});
