import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import OverviewTab from '../pages/tabs/overview';
import type { Hardware } from '../pages/tabs/shared';

// Mock Tauri APIs
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue({}),
}));

// Mock the heavy child components to keep the test focused on rendering
vi.mock('../components/SystemInfoCard', () => ({
  default: () => <div data-testid="system-info-card">SystemInfo</div>,
}));

vi.mock('../components/BatteryInfo', () => ({
  default: ({ battery }: { battery: unknown }) => (
    <div data-testid="battery-info">{battery ? 'BatteryInfo' : 'NoBattery'}</div>
  ),
}));

vi.mock('../components/PerformanceModeSelector', () => ({
  default: () => <div data-testid="perf-selector">PerfSelector</div>,
}));

vi.mock('../components/AiAdvisor', () => ({
  default: () => <div data-testid="ai-advisor">AiAdvisor</div>,
}));

function makeMockHardware(): Hardware {
  return {
    battery: null,
    performanceMode: 'balance',
    setPerformanceMode: vi.fn().mockResolvedValue(undefined),
    loading: false,
    display: null,
    fan: null,
    audioState: null,
    setBrightness: vi.fn().mockResolvedValue(undefined),
    setAiBrightness: vi.fn().mockResolvedValue(undefined),
    setAiBrightnessConfig: vi.fn().mockResolvedValue(undefined),
    setFanMode: vi.fn().mockResolvedValue(undefined),
    setMasterVolume: vi.fn().mockResolvedValue(undefined),
    setMasterMute: vi.fn().mockResolvedValue(undefined),
    systemInfo: null,
    getProcessList: vi.fn().mockResolvedValue([]),
    chargingThreshold: 80,
    setChargingThreshold: vi.fn().mockResolvedValue(undefined),
    hardwareProfile: null,
    setHdr: vi.fn().mockResolvedValue(undefined),
    setRefreshRate: vi.fn().mockResolvedValue(undefined),
    setAdaptiveRefreshRate: vi.fn().mockResolvedValue(undefined),
    touchpadInfo: null,
    setTouchpadSensitivity: vi.fn().mockResolvedValue(undefined),
    setTouchpadHaptics: vi.fn().mockResolvedValue(undefined),
    setTouchpadHapticsIntensity: vi.fn().mockResolvedValue(undefined),
    setTouchpadGestureScreenshot: vi.fn().mockResolvedValue(undefined),
    setTouchpadRepress: vi.fn().mockResolvedValue(undefined),
    setTouchpadEdgeSlide: vi.fn().mockResolvedValue(undefined),
  } as unknown as Hardware;
}

describe('OverviewTab', () => {
  it('renders without crashing', () => {
    const hw = makeMockHardware();
    const ai = {
      settings: {},
      saveSettings: vi.fn(),
      testConnection: vi.fn(),
      getTelemetryConsent: vi.fn(),
      setTelemetryConsent: vi.fn(),
      revokeTelemetryConsent: vi.fn(),
      setOnboardingCompleted: vi.fn(),
      isConfigured: false,
      updateKey: vi.fn(),
    } as unknown;
    render(<OverviewTab hw={hw} ai={ai as never} onOpenSettings={vi.fn()} />);
    expect(screen.getByTestId('system-info-card')).toBeInTheDocument();
    expect(screen.getByTestId('battery-info')).toBeInTheDocument();
    expect(screen.getByTestId('perf-selector')).toBeInTheDocument();
  });
});
