import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import BatteryTab from '../pages/tabs/battery';
import { t } from '../hooks/useI18n';
import type { Hardware } from '../pages/tabs/shared';
import type { BatteryInfo as BatteryData } from '../types/hardware';

/**
 * Smoke test da composição real (sem mocks dos cartões): a tab, o herói, a
 * proteção e os detalhes montados em conjunto — o que o utilizador vê.
 */
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn((cmd: string) =>
    cmd === 'get_battery_care' ? Promise.resolve(true) : Promise.resolve({}),
  ),
}));

const battery: BatteryData = {
  level: 96,
  is_charging: false,
  is_plugged: true,
  health_percent: 95,
  cycle_count: 0,
  designed_capacity_mwh: 68200,
  full_capacity_mwh: 65000,
  manufacturer: 'COSMX',
  device_name: 'BX70',
  serial_number: 'x',
  chemistry: 'LiP',
  temperature_celsius: 31.4,
  time_remaining_minutes: null,
  time_to_full_minutes: null,
  charge_rate_mw: 0,
  voltage_mv: 17280,
  ac_input_power_mw: 65000,
};

function hw(overrides: Partial<Hardware> = {}): Hardware {
  return {
    battery,
    chargingThreshold: 80,
    loading: false,
    error: null,
    setChargingThreshold: vi.fn().mockResolvedValue(undefined),
    refreshBattery: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  } as unknown as Hardware;
}

describe('BatteryTab — composição real', () => {
  beforeEach(() => vi.clearAllMocks());

  it('mostra as 9 peças sem rebentar: herói, fase, limite, derivada e detalhes', async () => {
    await act(async () => {
      render(<BatteryTab hw={hw()} />);
    });

    expect(screen.getByText('96%')).toBeInTheDocument();
    expect(screen.getByTestId('charge-phase')).toHaveTextContent('96%');
    expect(screen.getByTestId('limit-derived')).toBeInTheDocument();
    expect(screen.getByRole('switch')).toBeInTheDocument();
    expect(screen.getByRole('radiogroup')).toBeInTheDocument();
    // Detalhes: 5 linhas, tensão com vírgula pt-PT (o nbsp é \u00a0).
    expect(document.querySelectorAll('.mc-stat')).toHaveLength(5);
    expect(document.body.textContent).toContain('17,28');
  });

  it('sem leitura não há valores nem controlos, mas há um estado nomeado', async () => {
    await act(async () => {
      render(<BatteryTab hw={hw({ battery: null, loading: false, error: 'EC down' })} />);
    });

    expect(screen.getByText(t('battery.unavailable'))).toBeInTheDocument();
    expect(screen.queryByRole('switch')).not.toBeInTheDocument();
  });
});
