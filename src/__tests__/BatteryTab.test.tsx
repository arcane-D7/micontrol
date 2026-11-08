import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, act } from '@testing-library/react';
import BatteryTab from '../pages/tabs/battery';
import type { Hardware } from '../pages/tabs/shared';

// Mock Tauri APIs — `get_battery_care` responde true para o switch nascer ligado.
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn((cmd: string) =>
    cmd === 'get_battery_care' ? Promise.resolve(true) : Promise.resolve({}),
  ),
}));

// Mock child components: a tab é o que está a ser testado (orquestração),
// não o interior de cada cartão.
vi.mock('../components/BatteryInfo', () => ({
  default: ({ battery, threshold }: { battery: unknown; threshold?: number }) => (
    <div data-testid="battery-info">
      {battery ? 'Has Battery' : 'No Battery'}|threshold:{threshold}
    </div>
  ),
  BatteryDetails: () => <div data-testid="battery-details">Details</div>,
}));

vi.mock('../components/ChargingProtection', () => ({
  default: ({ threshold }: { threshold: number }) => (
    <div data-testid="charging-protection">Threshold: {threshold}%</div>
  ),
}));

function makeMockHardware(battery: unknown, extra: Partial<Hardware> = {}): Hardware {
  return {
    battery,
    chargingThreshold: 80,
    loading: false,
    error: null,
    setChargingThreshold: vi.fn().mockResolvedValue(undefined),
    refreshBattery: vi.fn().mockResolvedValue(undefined),
    ...extra,
  } as unknown as Hardware;
}

/** Monta a tab e deixa assentar a leitura de `get_battery_care`. */
async function renderTab(battery: unknown, extra: Partial<Hardware> = {}) {
  const hw = makeMockHardware(battery, extra);
  await act(async () => {
    render(<BatteryTab hw={hw} />);
  });
  return hw;
}

describe('BatteryTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renderiza o cartão de estado, a proteção e os detalhes', async () => {
    await renderTab({ level: 50, is_charging: true });
    expect(screen.getByTestId('battery-info')).toHaveTextContent('Has Battery');
    expect(screen.getByTestId('charging-protection')).toHaveTextContent('Threshold: 80%');
    expect(screen.getByTestId('battery-details')).toBeInTheDocument();
  });

  it('sem bateria esconde os controlos que dependem dela', async () => {
    await renderTab(null);
    expect(screen.getByTestId('battery-info')).toHaveTextContent('No Battery');
    expect(screen.queryByTestId('charging-protection')).not.toBeInTheDocument();
    expect(screen.queryByTestId('battery-details')).not.toBeInTheDocument();
  });

  it('passa o limite atual ao cartão de estado (a fase não pode mentir)', async () => {
    await renderTab({ level: 96, is_charging: false, is_plugged: true });
    expect(screen.getByTestId('battery-info')).toHaveTextContent('threshold:80');
  });

  it('lê o estado da proteção no arranque', async () => {
    const { invoke } = await import('@tauri-apps/api/core');
    await renderTab({ level: 50 });
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('get_battery_care'));
  });
});
