import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import AiConfigForm from '../components/AiConfigForm';
import type { AppSettings } from '../types/settings';

vi.mock('../hooks/useI18n', () => ({
  t: (key: string, opts?: Record<string, unknown>) => {
    if (opts) {
      return Object.entries(opts).reduce((s, [k, v]) => s.replace(`{${k}}`, String(v)), key);
    }
    return key;
  },
}));

const mockSettings: AppSettings = {
  onboardingCompleted: true,
  openai_api_key: 'sk-test',
  openai_base_url: 'https://api.openai.com/v1',
  openai_model: 'gpt-4o-mini',
  perf_mode_ac: null,
  perf_mode_dc: null,
  auto_switch_perf: false,
  tray_opacity: 1.0,
  ai_analysis_enabled: false,
  ai_poll_interval_sec: 60,
  ai_daily_analyses: 2,
};

describe('AiConfigForm', () => {
  const mockOnUpdate = vi.fn();
  const mockOnTestConnection = vi.fn().mockResolvedValue(undefined);

  beforeEach(() => {
    vi.clearAllMocks();
  });

  function renderForm(overrides?: Partial<Parameters<typeof AiConfigForm>[0]>) {
    return render(
      <AiConfigForm
        settings={mockSettings}
        onUpdate={mockOnUpdate}
        onTestConnection={mockOnTestConnection}
        {...overrides}
      />,
    );
  }

  it('renders with API key field (password type by default)', () => {
    renderForm();

    const apiKeyInput = screen.getByDisplayValue('sk-test');
    expect(apiKeyInput).toBeInTheDocument();
    expect(apiKeyInput).toHaveAttribute('type', 'password');
  });

  it('clicking show/hide toggle changes input type', async () => {
    const user = userEvent.setup();
    renderForm();

    const apiKeyInput = screen.getByDisplayValue('sk-test');
    expect(apiKeyInput).toHaveAttribute('type', 'password');

    // Click the eye button to show the key
    const toggleButton = screen.getByTitle('settings.showKey');
    await user.click(toggleButton);

    expect(apiKeyInput).toHaveAttribute('type', 'text');

    // Click again to hide
    const hideButton = screen.getByTitle('settings.hideKey');
    await user.click(hideButton);

    expect(apiKeyInput).toHaveAttribute('type', 'password');
  });

  it('changing model preset calls onUpdate', async () => {
    const user = userEvent.setup();
    renderForm();

    // Find the GPT-4o preset button (not mini) — mock i18n returns the key
    const gpt4oButton = screen.getByText('settings.presetGpt4o');
    await user.click(gpt4oButton);

    // Clicking a preset marks the form dirty; then we need to click Save
    const saveButton = screen.getByText('settings.save');
    await user.click(saveButton);

    expect(mockOnUpdate).toHaveBeenCalledWith(expect.objectContaining({ openai_model: 'gpt-4o' }));
  });

  it('clicking save calls onUpdate with draft', async () => {
    const user = userEvent.setup();
    renderForm();

    // Modify the API key to make the form dirty
    const apiKeyInput = screen.getByDisplayValue('sk-test');
    await user.clear(apiKeyInput);
    await user.type(apiKeyInput, 'sk-new-key');

    const saveButton = screen.getByText('settings.save');
    await user.click(saveButton);

    expect(mockOnUpdate).toHaveBeenCalledWith(
      expect.objectContaining({ openai_api_key: 'sk-new-key' }),
    );
  });

  it('clicking test calls onTestConnection', async () => {
    const user = userEvent.setup();
    renderForm();

    // The test button should be enabled since API key is set
    const testButton = screen.getByText('settings.testConnection');
    await user.click(testButton);

    await vi.waitFor(() => {
      expect(mockOnTestConnection).toHaveBeenCalledTimes(1);
    });
  });
});
