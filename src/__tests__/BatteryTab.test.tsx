import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import BatteryTab from '../pages/tabs/battery';
import type { Hardware } from '../pages/tabs/shared';

// Mock Tauri APIs
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue({}),
}));

// Mock child components
vi.mock('../components/BatteryInfo', () => ({
  default: ({ battery }: { battery: unknown }) => (
    <div data-testid="battery-info">{battery ? 'Has Battery' : 'No Battery'}</div>
  ),
}));

vi.mock('../components/ChargingThreshold', () => ({
  default: ({ threshold }: { threshold: number }) => (
    <div data-testid="charging-threshold">Threshold: {threshold}%</div>
  ),
}));

function makeMockHardware(battery: unknown): Hardware {
  return {
    battery,
    chargingThreshold: 80,
    setChargingThreshold: vi.fn().mockResolvedValue(undefined),
  } as unknown as Hardware;
}

describe('BatteryTab', () => {
  it('renders battery info and charging threshold', () => {
    const hw = makeMockHardware({ level: 50, is_charging: true });
    render(<BatteryTab hw={hw} />);
    expect(screen.getByTestId('battery-info')).toBeInTheDocument();
    expect(screen.getByTestId('charging-threshold')).toBeInTheDocument();
    expect(screen.getByText('Threshold: 80%')).toBeInTheDocument();
  });

  it('renders without battery data', () => {
    const hw = makeMockHardware(null);
    render(<BatteryTab hw={hw} />);
    expect(screen.getByTestId('battery-info')).toBeInTheDocument();
  });
});
