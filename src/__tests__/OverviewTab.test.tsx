import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { setLanguage } from '../hooks/useI18n';
import OverviewTab from '../pages/tabs/overview';
import type { Hardware } from '../pages/tabs/shared';

/**
 * O Overview deixou de ser "cartão de sistema + bateria + selector". Estes
 * testes fixam o contrato novo: o resumo mostra as especificações reais (CPU,
 * GPU, NPU, memória, disco), respeita a regra de honestidade (`—` quando o
 * valor não é observável) e os atalhos navegam.
 *
 * `vi.hoisted` porque a factory de `vi.mock` corre antes do corpo do módulo —
 * uma const normal ainda estaria em TDZ quando o mock é avaliado.
 */
const { SNAPSHOT } = vi.hoisted(() => ({
  SNAPSHOT: {
    device: {
      manufacturer: 'XIAOMI',
      model: 'Xiaomi Book Pro 14',
      family: 'Mi Laptop',
      sku: 'TM2424-77079',
      bios_version: 'XMAPT4B0P0A0A',
    },
    cpu: {
      name: 'Intel(R) Core(TM) Ultra X7 358H',
      cores: 16,
      threads: 16,
      base_clock_mhz: 1900,
      l3_cache_mb: 18,
      socket: 'U3E1',
    },
    gpu: { name: 'Intel(R) Arc(TM) B390 GPU', driver_version: '32.0.101.8991' },
    npu: {
      name: 'Intel(R) NPU',
      vendor: 'Intel Corporation',
      driver_version: '32.0.100.4841',
      driver_date: '2026-07-24',
      status: 'ok',
      device_id: 'PCI\\VEN_8086&DEV_B03E',
    },
    memory: {
      total_gb: 32,
      module_count: 8,
      speed_mt_s: 9600,
      memory_type: 'LPDDR5',
      manufacturer: 'SK Hynix',
    },
    storage: {
      model: 'BIWIN CE942V8890M-1TB',
      capacity_gb: 952.98,
      free_gb: 83.7,
      system_drive: 'C:',
      bus: 'NVMe',
      media: 'SSD',
      health: 'Healthy',
    },
    display: { width: 3120, height: 2080, refresh_hz: 120, name: 'Xiaomi HDR Monitor' },
    os: {
      caption: 'Microsoft Windows 11 Pro',
      build: '26200',
      arch: '64-bit',
      uptime_secs: 90_061,
    },
    captured_at_ms: 1_794_182_052_094,
  },
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn((cmd: string) => {
    switch (cmd) {
      case 'get_overview_snapshot':
        return Promise.resolve({ ...SNAPSHOT, npu: SNAPSHOT.npu });
      case 'get_os_turbo':
        return Promise.resolve({
          enabled: true,
          throttled_processes: 14,
          power_plan: 'Best Performance',
        });
      case 'wifi_status':
        return Promise.resolve({
          connected: true,
          ssid: 'Ramtula & Freitas',
          signal: 91,
          interface: 'Intel(R) Wi-Fi 6E AX211 160MHz',
        });
      case 'get_iot_device_info':
        return Promise.resolve({
          pipe_available: true,
          model: 'xiaomi.laptop.p52',
          fw_version: '1.0.3_0010',
          bind_status: { bound: false, uid: 0 },
          device_id: 2175217920,
          device_status: 'online (0x00)',
          wifi_status: null,
          wifi_network_count: null,
        });
      case 'get_health_status':
        return Promise.resolve({
          bridge: { state: 'healthy' },
          iot: { state: 'healthy' },
          face: { state: 'healthy' },
          ambient_sensor: { state: 'healthy' },
        });
      case 'get_battery_care':
        return Promise.resolve(true);
      default:
        return Promise.resolve(null);
    }
  }),
}));

vi.mock('../components/AiAdvisor', () => ({
  default: () => <div data-testid="ai-advisor">AiAdvisor</div>,
}));

function makeMockHardware(): Hardware {
  return {
    battery: null,
    performanceMode: 'balance',
    setPerformanceMode: vi.fn().mockResolvedValue(undefined),
    loading: false,
    display: null,
    fan: null,
    audioState: null,
    systemInfo: null,
    chargingThreshold: 80,
    setChargingThreshold: vi.fn().mockResolvedValue(undefined),
    hardwareProfile: null,
  } as unknown as Hardware;
}

const ai = {} as never;

beforeEach(() => {
  setLanguage('en');
});

describe('OverviewTab', () => {
  it('renders one h1 and the summary blocks', async () => {
    render(
      <OverviewTab hw={makeMockHardware()} ai={ai} onOpenSettings={vi.fn()} onNavigate={vi.fn()} />,
    );

    expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1);
    expect(await screen.findByText('Hardware')).toBeInTheDocument();
    expect(screen.getByText('Temperature')).toBeInTheDocument();
    expect(screen.getByText('Memory and storage')).toBeInTheDocument();
    expect(screen.getByText('System state')).toBeInTheDocument();
    expect(screen.getByText('Right now')).toBeInTheDocument();
  });

  it('shows the real CPU, GPU, NPU and memory specifications', async () => {
    render(
      <OverviewTab hw={makeMockHardware()} ai={ai} onOpenSettings={vi.fn()} onNavigate={vi.fn()} />,
    );

    expect(await screen.findByText('Intel(R) Core(TM) Ultra X7 358H')).toBeInTheDocument();
    expect(screen.getByText('Intel(R) Arc(TM) B390 GPU')).toBeInTheDocument();
    // The NPU the app never showed before.
    expect(screen.getByText('Intel(R) NPU')).toBeInTheDocument();
    expect(screen.getByText('LPDDR5 · 9,600 MT/s')).toBeInTheDocument();
    expect(screen.getByText('BIWIN CE942V8890M-1TB')).toBeInTheDocument();
    expect(screen.getByText('3120 × 2080')).toBeInTheDocument();
  });

  it('renders TOPS as an honest dash, not a datasheet number', async () => {
    render(
      <OverviewTab hw={makeMockHardware()} ai={ai} onOpenSettings={vi.fn()} onNavigate={vi.fn()} />,
    );

    const tops = await screen.findByText('TOPS');
    const row = tops.closest('.mc-stat');
    expect(row).not.toBeNull();
    expect(row).toHaveTextContent('—');
    expect(row).toHaveTextContent('not exposed by Windows');
  });

  it('says the machine has no battery instead of showing 0%', async () => {
    render(
      <OverviewTab hw={makeMockHardware()} ai={ai} onOpenSettings={vi.fn()} onNavigate={vi.fn()} />,
    );

    expect(await screen.findByText('This machine does not report a battery.')).toBeInTheDocument();
  });

  it('navigates when a "Go to…" shortcut is used', async () => {
    const onNavigate = vi.fn();
    render(
      <OverviewTab
        hw={makeMockHardware()}
        ai={ai}
        onOpenSettings={vi.fn()}
        onNavigate={onNavigate}
      />,
    );

    const shortcuts = await screen.findByRole('navigation', { name: 'Go to…' });
    expect(shortcuts).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /Battery/ }));
    expect(onNavigate).toHaveBeenCalledWith('battery');
  });

  it('keeps the AI advisor outside the hairline scope', async () => {
    render(
      <OverviewTab hw={makeMockHardware()} ai={ai} onOpenSettings={vi.fn()} onNavigate={vi.fn()} />,
    );

    const advisor = await screen.findByTestId('ai-advisor');
    expect(advisor.closest('.mc-hairline')).toBeNull();
    await waitFor(() => expect(screen.getByText('Right now')).toBeInTheDocument());
  });
});
