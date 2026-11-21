import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import ScreenCast from '../components/ScreenCast';

const invokeMock = vi.fn();

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

const addToastMock = vi.fn();
vi.mock('../contexts/ToastContext', () => ({
  useToast: () => ({ addToast: addToastMock }),
}));

vi.mock('../hooks/useI18n', async (importOriginal) => {
  type Original = Record<string, unknown>;
  const actual = (await importOriginal()) as Original;
  return {
    ...actual,
    t: (key: string, vars?: Record<string, string | number>) => {
      let value = MOCK_CAST[key] ?? key;
      if (vars) {
        for (const [k, v] of Object.entries(vars)) {
          value = value.split(`{${k}}`).join(String(v));
        }
      }
      return value;
    },
  };
});

const MOCK_CAST: Record<string, string> = {
  'cast.title': 'Cast to another screen',
  'cast.subtitle': 'Project to a wireless display',
  'cast.stateTitle': 'Session state',
  'cast.stateActive': 'Connected to {name}',
  'cast.stateInactive': 'No active projection session',
  'cast.stateNoEndpoints': 'No projection endpoints paired with this PC yet',
  'cast.lastAttempt': 'Last connection attempt',
  'cast.noAttempt': 'None yet',
  'cast.connect': 'Connect',
  'cast.connecting': 'Connecting...',
  'cast.connectRefused': 'Windows refused the programmatic cast',
  'cast.connectError': 'Connection attempt failed',
  'cast.openPanel': 'Open Windows panel (Win + K)',
  'cast.scanNow': 'Scan for nearby displays',
  'cast.scanning': 'Scanning for projection devices (about 6 seconds)...',
  'cast.scanDone': 'Scan finished: {count} projection device(s) in range.',
  'cast.scanIncomplete': 'Scan window expired before discovery completed; the list may be partial.',
  'cast.scanError': 'Scan error',
  'cast.error': 'Cast error',
  'cast.stopped': 'Connect panel closed',
  'cast.stop': 'Close panel',
  'cast.scan': 'Refresh paired displays',
  'cast.noDevices': 'No displays are paired with this PC yet.',
  'cast.availableDevices': 'Displays paired with this PC',
  'cast.connectedChip': 'Connected',
  'cast.disconnectedChip': 'Not connected',
  'cast.unknownChip': 'State unknown',
  'cast.knownOnly': 'Only displays already paired with this PC can be listed here.',
  'cast.probe': 'Check what Windows allows',
  'cast.probeReport': 'API check result',
  'cast.panelNoteTitle': 'Windows owns the projection',
  'cast.panelFallbackNote': 'MiControl cannot start desktop mirroring through a public API.',
};

const device = (over: Partial<{ id: string; name: string; connected: boolean | null }> = {}) => ({
  id: over.id ?? 'ep-1',
  name: over.name ?? 'Living Room TV',
  device_type: 'miracast',
  // `in` instead of ?? : null is a MEANINGFUL value here (unknown state).
  connected: 'connected' in over ? over.connected! : false,
});

beforeEach(() => {
  invokeMock.mockReset();
  addToastMock.mockReset();
  // Default: every invoke resolves harmlessly so effects settle.
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === 'get_cast_state') {
      return Promise.resolve({
        active: null,
        device_name: null,
        detail: 'none',
        last_attempt: null,
      });
    }
    return Promise.resolve([]);
  });
});

describe('ScreenCast', () => {
  it('renders the fallback card with an explicit Windows-panel path', async () => {
    render(<ScreenCast />);
    expect(await screen.findByText(/Check what Windows allows/)).toBeDefined();
    expect(screen.getByText(/Win \+ K/)).toBeDefined();
  });

  it('opens the panel through open_cast_panel, not start_casting, when the panel button fires', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_cast_state')
        return Promise.resolve({ active: null, device_name: null, detail: '', last_attempt: null });
      if (cmd === 'get_cast_devices') return Promise.resolve([]);
      return Promise.resolve({ success: true, message: 'panel opened' });
    });
    render(<ScreenCast />);
    const panelBtn = await screen.findByText(/Open Windows panel/);
    fireEvent.click(panelBtn.closest('button') as HTMLElement);
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('open_cast_panel');
    });
    expect(invokeMock).not.toHaveBeenCalledWith('start_casting', expect.anything());
  });

  it('shows the state card as connected when an endpoint reports IsConnected', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_cast_state') {
        return Promise.resolve({
          active: true,
          device_name: 'Office Display',
          detail: 'endpoint reports connection',
          last_attempt: null,
        });
      }
      return Promise.resolve([]);
    });
    render(<ScreenCast />);
    expect(await screen.findByText('Connected to Office Display')).toBeDefined();
  });

  it('distinguishes no-endpoints (none) from no-session (false) in the state card', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_cast_state') {
        return Promise.resolve({
          active: false,
          device_name: null,
          detail: 'endpoints exist, none connected',
          last_attempt: null,
        });
      }
      return Promise.resolve([]);
    });
    render(<ScreenCast />);
    expect(await screen.findByText('No active projection session')).toBeDefined();
    // And must NOT render the no-endpoints line for active=false.
    expect(screen.queryByText('No projection endpoints paired with this PC yet')).toBeNull();
  });

  it('renders chips from the real connected field: true -> Connected, false -> Not connected, null -> unknown', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_cast_devices') {
        return Promise.resolve([
          device({ id: 'a', name: 'Alpha', connected: true }),
          device({ id: 'b', name: 'Beta', connected: false }),
          device({ id: 'c', name: 'Gamma', connected: null }),
        ]);
      }
      if (cmd === 'get_cast_state')
        return Promise.resolve({ active: null, device_name: null, detail: '', last_attempt: null });
      return Promise.resolve([]);
    });
    const { container } = render(<ScreenCast />);
    await screen.findByText('Alpha');
    const text = container.textContent ?? '';
    expect(text).toContain('Connected');
    expect(text).toContain('Not connected');
    expect(text).toContain('State unknown');
  });

  it('Connect is disabled until a device is selected and calls start_casting with the id', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_cast_devices')
        return Promise.resolve([device({ id: 'ep-9', name: 'Den TV' })]);
      if (cmd === 'get_cast_state')
        return Promise.resolve({ active: null, device_name: null, detail: '', last_attempt: null });
      return Promise.resolve({ success: false, message: 'Windows refused: InvalidCastingSource' });
    });
    render(<ScreenCast />);
    await screen.findByText('Den TV');

    const findConnectBtn = () =>
      Array.from(document.querySelectorAll('button')).find((b) =>
        b.textContent?.includes('Connect'),
      ) as HTMLButtonElement;
    expect(findConnectBtn().disabled).toBe(true);

    fireEvent.click(screen.getByText('Den TV'));
    // Re-query: the selection re-render replaces the button node.
    const connectBtn = findConnectBtn();
    expect(connectBtn.disabled).toBe(false);
    fireEvent.click(connectBtn);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('start_casting', { deviceId: 'ep-9' });
    });
    // The refusal message is shown verbatim (honest error), not replaced.
    await waitFor(() => {
      expect(addToastMock).toHaveBeenCalledWith(
        expect.objectContaining({
          message: 'Windows refused: InvalidCastingSource',
          type: 'error',
        }),
      );
    });
  });

  it('the scan reports a partial window honestly when completed=false', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_cast_state')
        return Promise.resolve({ active: null, device_name: null, detail: '', last_attempt: null });
      if (cmd === 'scan_cast_devices') {
        return Promise.resolve({
          duration_ms: 6000,
          devices: [device({ id: 's1', name: 'Found One' })],
          added_events: 1,
          updated_events: 0,
          completed: false,
        });
      }
      return Promise.resolve([]);
    });
    render(<ScreenCast />);
    const scanBtn = (await screen.findByText(/Scan for nearby displays/)).closest('button');
    fireEvent.click(scanBtn as HTMLElement);
    await waitFor(() => {
      expect(addToastMock).toHaveBeenCalledWith(
        expect.objectContaining({
          message: 'Scan window expired before discovery completed; the list may be partial.',
        }),
      );
    });
    await waitFor(() => {
      expect(screen.getByText('Found One')).toBeDefined();
    });
  });

  it('the probe panel renders the backend evidence lines verbatim', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_cast_state')
        return Promise.resolve({ active: null, device_name: null, detail: '', last_attempt: null });
      if (cmd === 'probe_cast_capabilities') {
        return Promise.resolve({
          probed_at_unix_ms: 1,
          screen_mirroring_by_api: false,
          conclusion:
            'Windows does not allow starting desktop Miracast mirroring through a public API.',
          probes: [
            {
              api: 'MediaPlayer.GetAsCastingSource()',
              status: 'ok',
              detail: 'empty media source ok',
            },
            {
              api: 'CastingConnection.RequestStartCastingAsync(empty source)',
              status: 'refused',
              detail: 'Den TV answered: InvalidCastingSource',
            },
          ],
        });
      }
      return Promise.resolve([]);
    });
    const { container } = render(<ScreenCast />);
    const probeBtn = (await screen.findByText(/Check what Windows allows/)).closest('button');
    fireEvent.click(probeBtn as HTMLElement);
    await waitFor(() => {
      expect(
        (container.textContent ?? '').includes(
          'Windows does not allow starting desktop Miracast mirroring',
        ),
      ).toBe(true);
    });
    expect(
      (container.textContent ?? '').includes(
        'refused - CastingConnection.RequestStartCastingAsync(empty source): Den TV answered: InvalidCastingSource',
      ),
    ).toBe(true);
  });

  it('a failed probe surfaces an error toast with a retry', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_cast_state')
        return Promise.resolve({ active: null, device_name: null, detail: '', last_attempt: null });
      if (cmd === 'probe_cast_capabilities') return Promise.reject(new Error('probe boom'));
      return Promise.resolve([]);
    });
    render(<ScreenCast />);
    const probeBtn = (await screen.findByText(/Check what Windows allows/)).closest('button');
    fireEvent.click(probeBtn as HTMLElement);
    await waitFor(() => {
      expect(addToastMock).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' }));
    });
  });
});
