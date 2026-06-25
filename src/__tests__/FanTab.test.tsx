import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import FanTab from '../pages/tabs/fan';
import type { Hardware } from '../pages/tabs/shared';

// Mock Tauri APIs
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue({}),
}));

// Mock FanControl to test that FanTab passes props correctly
vi.mock('../components/FanControl', () => ({
  default: ({ fan, onModeChange }: { fan: unknown; onModeChange: unknown }) => (
    <div data-testid="fan-control">
      Fan: {fan ? 'present' : 'null'} | ModeChange: {typeof onModeChange}
    </div>
  ),
}));

function makeMockHardware(fan: unknown): Hardware {
  return {
    fan,
    setFanMode: vi.fn().mockResolvedValue(undefined),
  } as unknown as Hardware;
}

describe('FanTab', () => {
  it('renders FanControl with fan data', () => {
    const hw = makeMockHardware({ mode: 'auto', speed_rpm: 3000, speed_percent: 50 });
    render(<FanTab hw={hw} />);
    expect(screen.getByTestId('fan-control')).toBeInTheDocument();
    expect(screen.getByText(/Fan: present/)).toBeInTheDocument();
  });

  it('renders FanControl even without fan data', () => {
    const hw = makeMockHardware(null);
    render(<FanTab hw={hw} />);
    expect(screen.getByTestId('fan-control')).toBeInTheDocument();
    expect(screen.getByText(/Fan: null/)).toBeInTheDocument();
  });
});
