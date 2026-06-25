import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import SettingsTab from '../pages/tabs/settings';
import type { AiSettings } from '../pages/tabs/shared';

// Mock Tauri APIs
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue({}),
}));

// Mock SettingsPage to test that SettingsTab passes props correctly
vi.mock('../components/SettingsPage', () => ({
  default: ({ telemetryConsent }: { telemetryConsent: string | null }) => (
    <div data-testid="settings-page">Consent: {telemetryConsent ?? 'null'}</div>
  ),
}));

function makeMockAiSettings(): AiSettings {
  return {
    settings: {
      onboardingCompleted: true,
      openai_api_key: '',
      openai_base_url: 'https://api.openai.com/v1',
      openai_model: 'gpt-4',
      perf_mode_ac: null,
      perf_mode_dc: null,
      auto_switch_perf: false,
      tray_opacity: 1.0,
      ai_analysis_enabled: false,
      ai_poll_interval_sec: 30,
      ai_daily_analyses: 0,
    },
    saveSettings: vi.fn(),
    testConnection: vi.fn().mockResolvedValue(undefined),
    getTelemetryConsent: vi.fn().mockResolvedValue('granted'),
    setTelemetryConsent: vi.fn().mockResolvedValue(undefined),
    revokeTelemetryConsent: vi.fn().mockResolvedValue(undefined),
    setOnboardingCompleted: vi.fn(),
    isConfigured: false,
    updateKey: vi.fn(),
  } as unknown as AiSettings;
}

describe('SettingsTab', () => {
  it('renders SettingsPage without crashing', async () => {
    const ai = makeMockAiSettings();
    render(<SettingsTab ai={ai} onTabChange={vi.fn()} />);
    expect(screen.getByTestId('settings-page')).toBeInTheDocument();
  });

  it('loads telemetry consent on mount', async () => {
    const ai = makeMockAiSettings();
    render(<SettingsTab ai={ai} onTabChange={vi.fn()} />);
    // The mock getTelemetryConsent returns 'granted'
    expect(ai.getTelemetryConsent).toHaveBeenCalled();
  });
});
