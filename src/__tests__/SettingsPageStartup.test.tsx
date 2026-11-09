import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import SettingsPage from '../components/SettingsPage';
import type { AppSettings } from '../types/settings';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('../components/AiConfigForm', () => ({ default: () => null }));
vi.mock('../components/PrivacyConsentSection', () => ({ default: () => null }));

const SETTINGS = {
  onboardingCompleted: true,
  openai_api_key: '',
  openai_base_url: 'https://api.openai.com/v1',
  openai_model: 'gpt-4',
  perf_mode_ac: null,
  perf_mode_dc: null,
  auto_switch_perf: false,
  tray_opacity: 1,
  ai_analysis_enabled: false,
  ai_poll_interval_sec: 30,
  ai_daily_analyses: 0,
} as unknown as AppSettings;

function renderSettings(onOpenStartup = vi.fn()) {
  return {
    onOpenStartup,
    ...render(
      <SettingsPage
        settings={SETTINGS}
        onSave={vi.fn()}
        onTest={vi.fn().mockResolvedValue(undefined)}
        telemetryConsent={null}
        onRevokeConsent={vi.fn().mockResolvedValue(undefined)}
        onGrantConsent={vi.fn().mockResolvedValue(undefined)}
        onOpenPrivacyPolicy={vi.fn()}
        onReplayOnboarding={vi.fn()}
        onOpenStartup={onOpenStartup}
      />,
    ),
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === 'get_autostart') return Promise.resolve(true);
    if (cmd === 'get_error_log_config') {
      return Promise.resolve({ enabled: true, retention_days: 7, log_path: 'C:\\Logs' });
    }
    if (cmd === 'mcp_get_enabled') return Promise.resolve(false);
    return Promise.resolve(undefined);
  });
});

describe('SettingsPage — MiControl autostart', () => {
  it('lê o estado real e alterna via set_autostart', async () => {
    renderSettings();

    const toggle = await screen.findByRole('switch', {
      name: /Start MiControl with Windows/,
    });
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'true'));

    fireEvent.click(toggle);

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_autostart', { enabled: false }),
    );
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'false'));
  });

  it('abre a tab de arranque a partir do botão de gestão', async () => {
    const { onOpenStartup } = renderSettings();
    fireEvent.click(await screen.findByRole('button', { name: 'Manage startup entries' }));
    expect(onOpenStartup).toHaveBeenCalledTimes(1);
  });
});
