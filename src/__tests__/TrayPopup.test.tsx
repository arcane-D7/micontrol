import { describe, it, expect, vi } from 'vitest';
import { render } from '@testing-library/react';
import TrayPopup from '../pages/TrayPopup';
import type { Hardware } from '../pages/tabs/shared';

// Mock Tauri APIs
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue({}),
}));

// Mock useSettings hook
vi.mock('../hooks/useSettings', () => ({
  useSettings: () => ({
    isConfigured: false,
    settings: {
      tray_opacity: 1.0,
      onboardingCompleted: true,
      openai_api_key: '',
      openai_base_url: 'https://api.openai.com/v1',
      openai_model: 'gpt-4',
      perf_mode_ac: null,
      perf_mode_dc: null,
      auto_switch_perf: false,
      ai_analysis_enabled: false,
      ai_poll_interval_sec: 30,
      ai_daily_analyses: 0,
    },
    updateKey: vi.fn(),
  }),
}));

// Mock brightness presets
vi.mock('../lib/brightnessPresets', () => ({
  BRIGHTNESS_PRESETS: [],
  getActivePreset: vi.fn(() => null),
}));

function makeMockHardware(): Hardware {
  return {
    battery: { level: 75, is_charging: false, is_plugged: true },
    performanceMode: 'balance',
    setPerformanceMode: vi.fn().mockResolvedValue(undefined),
    loading: false,
    display: { brightness: 80, hdr_enabled: false, refresh_rate_hz: 60 },
    fan: { mode: 'auto', speed_rpm: 3000, speed_percent: 50 },
    audioState: { volume: 50, muted: false },
    setBrightness: vi.fn().mockResolvedValue(undefined),
    setAiBrightness: vi.fn().mockResolvedValue(undefined),
    setAiBrightnessConfig: vi.fn().mockResolvedValue(undefined),
    setFanMode: vi.fn().mockResolvedValue(undefined),
    setMasterVolume: vi.fn().mockResolvedValue(undefined),
    setMasterMute: vi.fn().mockResolvedValue(undefined),
  } as unknown as Hardware;
}

describe('TrayPopup', () => {
  it('renders without crashing', () => {
    const hw = makeMockHardware();
    const { container } = render(<TrayPopup hardware={hw} />);
    expect(container.firstChild).toBeInTheDocument();
  });

  it('renders performance mode buttons', () => {
    const hw = makeMockHardware();
    render(<TrayPopup hardware={hw} />);
    // The tray popup should have mode buttons (balance is the default)
    const buttons = document.querySelectorAll('button');
    expect(buttons.length).toBeGreaterThan(0);
  });
});
