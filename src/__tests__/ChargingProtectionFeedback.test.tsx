import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ChargingProtection from '../components/ChargingProtection';

const invokeMock = vi.fn();

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

/**
 * Feedback regressions for audit 2026-11-07 §4.3 #1 and #2:
 *  - a click on 40–80% while protection is off must not be a dead click;
 *  - the card must name the charging state (96% with an 80% limit) and the
 *    confirmation must carry value + time.
 */
describe('ChargingProtection feedback', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_battery_care') return Promise.resolve(false);
      return Promise.resolve(undefined);
    });
  });

  it('turns protection on and applies the level in one click', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn().mockResolvedValue(undefined);
    render(<ChargingProtection threshold={100} onThresholdChange={onChange} />);

    await waitFor(() =>
      expect(screen.getByText('Battery protection is off', { exact: true })).toBeInTheDocument(),
    );

    const level = screen.getByRole('button', { name: /^60%/ });
    expect(level).toBeEnabled();

    await user.click(level);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('set_battery_care', { enabled: true });
      expect(onChange).toHaveBeenCalledWith(60);
    });
    // Confirmation names the value and the local time.
    expect(await screen.findByText(/Battery protection on at \d{2}:\d{2}/)).toBeInTheDocument();
  });

  it('names the paused state when the battery is above the limit', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_battery_care') return Promise.resolve(true);
      return Promise.resolve(undefined);
    });
    render(
      <ChargingProtection
        threshold={80}
        onThresholdChange={vi.fn().mockResolvedValue(undefined)}
        battery={{ level: 96, is_charging: false, is_plugged: true }}
      />,
    );

    // 96% with an 80% limit used to print "AC connected · 96%" and stop there.
    expect(await screen.findByText('Paused at 96% — resumes below 80%')).toBeInTheDocument();
    expect(screen.getByText('Why?')).toBeInTheDocument();
  });

  it('reports charging up to the limit when the level is below it', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_battery_care') return Promise.resolve(true);
      return Promise.resolve(undefined);
    });
    render(
      <ChargingProtection
        threshold={80}
        onThresholdChange={vi.fn().mockResolvedValue(undefined)}
        battery={{ level: 54, is_charging: true, is_plugged: true }}
      />,
    );

    expect(await screen.findByText('Charging up to 80%')).toBeInTheDocument();
  });
});
