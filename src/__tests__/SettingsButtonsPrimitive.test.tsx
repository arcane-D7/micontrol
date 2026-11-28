import { describe, it, expect, vi } from 'vitest';
import { render } from '@testing-library/react';
import SettingsPage from '../components/SettingsPage';
import type { AppSettings } from '../types/settings';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue({}) }));

/** Classes herdadas que deixaram de ser canónicas na vaga hairline. */
const LEGACY_CLASSES = [
  'btn',
  'btn-primary',
  'btn-secondary',
  'btn-ghost',
  'btn-danger',
  'btn-sm',
  'btn-full',
  'chip-btn',
];

const baseSettings: AppSettings = {
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
  mcp_integration_enabled: false,
};

function renderPage(consent: 'granted' | 'denied' | null) {
  return render(
    <SettingsPage
      settings={baseSettings}
      onSave={vi.fn()}
      onTest={vi.fn().mockResolvedValue(undefined)}
      telemetryConsent={consent}
      onRevokeConsent={vi.fn().mockResolvedValue(undefined)}
      onGrantConsent={vi.fn().mockResolvedValue(undefined)}
      onOpenPrivacyPolicy={vi.fn()}
      onReplayOnboarding={vi.fn()}
      onOpenStartup={vi.fn()}
    />,
  );
}

describe('Settings: todos os botões usam o primitivo canónico', () => {
  for (const consent of [null, 'granted', 'denied'] as const) {
    it(`consent=${consent}: cada <button> tem classe mc-* e nenhuma herdada`, () => {
      const { container, unmount } = renderPage(consent);
      const buttons = Array.from(container.querySelectorAll('button'));
      expect(buttons.length).toBeGreaterThan(0);
      for (const button of buttons) {
        const classes = button.className.split(/\s+/).filter(Boolean);
        expect(
          classes.some((c) => c.startsWith('mc-')),
          `botão sem primitivo: "${button.className}" ("${button.textContent}")`,
        ).toBe(true);
        for (const legacy of LEGACY_CLASSES) {
          expect(classes, `classe herdada "${legacy}" em "${button.className}"`).not.toContain(
            legacy,
          );
        }
      }
      unmount();
    });
  }

  it('as classes herdadas (chip-btn, btn-*) desapareceram do DOM', () => {
    const { container } = renderPage('granted');
    expect(container.querySelector('.chip-btn')).toBeNull();
    expect(container.querySelector('.btn-ghost')).toBeNull();
    expect(container.querySelector('.btn-primary')).toBeNull();
    expect(container.querySelector('.btn-secondary')).toBeNull();
  });
});
