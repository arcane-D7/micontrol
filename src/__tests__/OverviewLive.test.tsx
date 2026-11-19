import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import { setLanguage } from '../hooks/useI18n';
import { OverviewLive } from '../components/overview/OverviewLive';
import type { BatteryInfo, FanInfo, SystemInfo } from '../types/hardware';

/**
 * O bloco "Consumo vivo" — primeiro bloco do Overview. Contrato fixado:
 *  - valores presentes → número + unidade (en: '23 %', '64.8 °C', '12.5 / 32 GB');
 *  - valores ausentes → '—' (nunca zero inventado);
 *  - o refresco corre numa cadeia de setTimeout com perna runBounded (2 s),
 *    colapsando na perna de sistema do hook — e morre SEM fugas no unmount.
 */

const INFO: SystemInfo = {
  cpu_name: 'Intel(R) Core(TM) Ultra X7 358H',
  cpu_cores: 16,
  cpu_threads: 16,
  cpu_usage: 23.4,
  gpu_name: 'Intel(R) Arc(TM) B390 GPU',
  gpu_usage: 41,
  vram_used_mb: 0,
  ram_total_gb: 32,
  ram_used_gb: 12.46,
  os_version: 'Microsoft Windows 11 Pro',
};

const FAN: FanInfo = {
  mode: 'auto',
  speed_rpm: null,
  speed_percent: null,
  requested_speed_percent: 50,
  gpu_temp_celsius: 52.3,
  cpu_temp_celsius: 64.8,
  ambient_temp_celsius: null,
  tdp_watts: null,
  temp_source: 'esif',
};

const BATTERY: BatteryInfo = {
  level: 87,
  is_charging: false,
  is_plugged: true,
  health_percent: 93.3,
  cycle_count: 120,
  designed_capacity_mwh: 68_100,
  full_capacity_mwh: 63_540,
  manufacturer: 'SUNWODA',
  device_name: 'RB34501',
  serial_number: 'X',
  chemistry: 'LION',
  temperature_celsius: null,
  time_remaining_minutes: null,
  time_to_full_minutes: null,
  charge_rate_mw: 0,
  voltage_mv: 0,
  ac_input_power_mw: null,
};

function renderLive(overrides: Partial<Parameters<typeof OverviewLive>[0]> = {}) {
  const refresh = vi.fn(async () => {});
  const props = {
    systemInfo: INFO,
    fan: FAN,
    battery: BATTERY,
    refreshSystem: refresh,
    ...overrides,
  };
  const view = render(<OverviewLive {...props} />);
  return { refresh, view };
}

beforeEach(() => {
  setLanguage('en');
});

afterEach(() => {
  vi.useRealTimers();
});

describe('OverviewLive — valores presentes', () => {
  it('mostra CPU %, GPU %, RAM usada/total e bateria com unidades', () => {
    renderLive();

    expect(screen.getByTestId('live-cpu')).toHaveTextContent('23');
    expect(screen.getByTestId('live-gpu')).toHaveTextContent('41');
    expect(screen.getByTestId('live-ram')).toHaveTextContent('12.5 / 32');
    expect(screen.getByTestId('live-battery')).toHaveTextContent('87');

    // Unidades da casa (span próprio, nunca colado ao número).
    expect(screen.getAllByText('%').length).toBeGreaterThanOrEqual(3);
    expect(screen.getAllByText('°C')).toHaveLength(2);
    expect(screen.getByText('GB')).toBeInTheDocument();

    // Sub-linhas de temperatura dos tiles de processador.
    expect(screen.getByTestId('live-cpu')).toHaveTextContent('64.8');
    expect(screen.getByTestId('live-gpu')).toHaveTextContent('52.3');

    // Estado da bateria (ligada, não a carregar).
    expect(screen.getByTestId('live-battery')).toHaveTextContent('On AC');
  });

  it('distingue os três estados da bateria', () => {
    const { view } = renderLive({ battery: { ...BATTERY, is_charging: true } });
    expect(screen.getByTestId('live-battery')).toHaveTextContent('Charging');
    view.unmount();

    renderLive({ battery: { ...BATTERY, is_plugged: false } });
    expect(screen.getByTestId('live-battery')).toHaveTextContent('On battery');
  });
});

describe('OverviewLive — honestidade', () => {
  it('sem leituras tudo é "—", nunca zero inventado', () => {
    renderLive({ systemInfo: null, fan: null, battery: null });

    // 7 espaços sem leitura: CPU %, temp CPU, GPU %, temp GPU, RAM, nível e
    // estado da bateria. Se alguém trocar um deles por 0, este teste falha.
    expect(screen.getAllByText('—')).toHaveLength(7);
    expect(screen.queryByText('%')).toBeNull();
    expect(screen.queryByText('GB')).toBeNull();
  });

  it('temperatura ausente é "—" mesmo com utilização presente', () => {
    renderLive({ fan: { ...FAN, cpu_temp_celsius: null, gpu_temp_celsius: null } });
    expect(screen.getByTestId('live-cpu')).toHaveTextContent('23');
    expect(screen.getByTestId('live-cpu')).toHaveTextContent('—');
  });
});

describe('OverviewLive — refresco sem fugas', () => {
  it('refresca a perna de sistema a cada 2 s e pára limpo no unmount', async () => {
    vi.useFakeTimers();
    const { refresh, view } = renderLive();

    // Marca inicial imediata; a perna corre e agenda a próxima.
    expect(refresh).toHaveBeenCalledTimes(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(vi.getTimerCount()).toBe(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });
    expect(refresh).toHaveBeenCalledTimes(2);

    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('sem refreshSystem o bloco não abre marcadores (nada a fugir)', () => {
    vi.useFakeTimers();
    render(<OverviewLive systemInfo={INFO} fan={FAN} battery={BATTERY} />);
    expect(vi.getTimerCount()).toBe(0);
  });
});
