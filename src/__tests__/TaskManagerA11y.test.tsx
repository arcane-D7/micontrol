import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import TaskManagerTab from '../pages/tabs/taskmanager';
import type { Hardware } from '../pages/tabs/shared';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue([]) }));

const PROCESSES = [
  {
    name: 'alpha.exe',
    pid: 10,
    cpu_percent: 5,
    memory_mb: 100,
    gpu_percent: 0,
    npu_percent: 0,
    thread_count: 3,
  },
  {
    name: 'beta.exe',
    pid: 20,
    cpu_percent: 50,
    memory_mb: 200,
    gpu_percent: 0,
    npu_percent: 0,
    thread_count: 4,
  },
];

function makeHardware(): Hardware {
  return {
    getTaskManager: vi.fn().mockResolvedValue(PROCESSES),
    getNetworkPerf: vi.fn().mockResolvedValue([{ name: 'Wi-Fi', bytes_per_sec: 1024 }]),
    killProcess: vi.fn().mockResolvedValue(undefined),
    systemInfo: null,
  } as unknown as Hardware;
}

/** Regression guards for audit 2026-11-07 P0-1 and P0-2. */
describe('TaskManagerTab accessibility', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('names every sortable column with a real button and exposes aria-sort', async () => {
    render(<TaskManagerTab hw={makeHardware()} />);

    const cpuButton = await screen.findByRole('button', { name: 'Sort by CPU %' });
    const cpuHeader = cpuButton.closest('th');

    expect(cpuHeader).not.toBeNull();
    expect(cpuHeader).toHaveAttribute('scope', 'col');
    // Default sort is CPU descending.
    expect(cpuHeader).toHaveAttribute('aria-sort', 'descending');

    // The button is the interactive element, so Enter/Space work natively.
    fireEvent.click(cpuButton);
    expect(cpuHeader).toHaveAttribute('aria-sort', 'ascending');
  });

  it('marks unsorted columns as aria-sort="none" and leaves them keyboard reachable', async () => {
    render(<TaskManagerTab hw={makeHardware()} />);

    const ramButton = await screen.findByRole('button', { name: 'Sort by RAM' });
    expect(ramButton.closest('th')).toHaveAttribute('aria-sort', 'none');
  });

  it('gives every destructive button the name of the process it ends', async () => {
    render(<TaskManagerTab hw={makeHardware()} />);

    expect(await screen.findByRole('button', { name: 'End alpha.exe' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'End beta.exe' })).toBeInTheDocument();
  });

  it('labels the filter input instead of relying on the placeholder', async () => {
    render(<TaskManagerTab hw={makeHardware()} />);

    expect(await screen.findByLabelText('Filter processes by name or PID')).toBeInTheDocument();
  });
});
