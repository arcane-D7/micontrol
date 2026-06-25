import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import DisplayTab from '../pages/tabs/display';
import type { Hardware } from '../pages/tabs/shared';

// Mock Tauri APIs
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue({}),
}));

// Mock DisplaySettings to test that DisplayTab passes props correctly
vi.mock('../components/DisplaySettings', () => ({
  default: ({ display }: { display: unknown }) => (
    <div data-testid="display-settings">Display: {display ? 'present' : 'null'}</div>
  ),
}));

function makeMockHardware(display: unknown): Hardware {
  return {
    display,
    hardwareProfile: null,
    setBrightness: vi.fn().mockResolvedValue(undefined),
    setHdr: vi.fn().mockResolvedValue(undefined),
    setAiBrightness: vi.fn().mockResolvedValue(undefined),
    setAiBrightnessConfig: vi.fn().mockResolvedValue(undefined),
    setRefreshRate: vi.fn().mockResolvedValue(undefined),
    setAdaptiveRefreshRate: vi.fn().mockResolvedValue(undefined),
  } as unknown as Hardware;
}

describe('DisplayTab', () => {
  it('renders DisplaySettings with display data', () => {
    const hw = makeMockHardware({ brightness: 80, hdr_enabled: false });
    render(<DisplayTab hw={hw} />);
    expect(screen.getByTestId('display-settings')).toBeInTheDocument();
    expect(screen.getByText(/Display: present/)).toBeInTheDocument();
  });

  it('renders DisplaySettings without display data', () => {
    const hw = makeMockHardware(null);
    render(<DisplayTab hw={hw} />);
    expect(screen.getByTestId('display-settings')).toBeInTheDocument();
    expect(screen.getByText(/Display: null/)).toBeInTheDocument();
  });
});
