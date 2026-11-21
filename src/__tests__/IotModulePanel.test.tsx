import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import IotModulePanel, { type IotModulePanelHw } from '../components/IotModulePanel';
import { t } from '../hooks/useI18n';
import type { EramMap } from '../types/hardware';

/**
 * The UX contract for EC RAM, in one place:
 *
 *  1. ERAM/SMA2 blocked by the IoT driver (0x80070005, even for SYSTEM) is the
 *     platform's EXPECTED state → calm "—" + hint, NO error banner.
 *  2. A dead ecram_service IS a fault → red banner.
 *  3. The ranges the driver does expose keep showing real bytes.
 */

const blockedMap: EramMap = {
  misc0: 0,
  misc1: 0,
  control_flags_1b: 0,
  ai_limit_enabled: false,
  long_battery_limit_enabled: false,
  cpu_temp_c: 0,
  fan_rpm: 0,
  fan2_rpm: 0,
  cpu_power_w: 0,
  smart_mode_type: 0,
  smart_mode_data: 0,
  smart_mode_profile: null,
  qfan_mode: 0,
  perf_profile: 1,
  tdp_w: 0,
  ac_flags: 0,
  ac_connected: true,
  ac_adapter_w: 65,
  battery_current_ma: 0,
  battery_capacity_mah: 0,
  battery_voltage_mv: 0,
  charge_threshold_pct: 0,
  battery_temp_c: 0,
  display_brightness_level: 0,
  keyboard_backlight_level: 0,
  raw_hex: 'UNAVAILABLE',
  available: false,
  blocked_regions: ['ERAM', 'SMA2'],
  blocked_reason:
    'ERAM could not be read: ecram_service answered "DeviceIoControl ECRAM_READ: Access is denied. (0x80070005)"',
  blocked_kind: 'driver_blocked',
  perf_profile_from_wmi: true,
};

const availableMap: EramMap = {
  ...blockedMap,
  cpu_temp_c: 61,
  fan_rpm: 2400,
  battery_voltage_mv: 17120,
  raw_hex: 'ff'.repeat(0x100),
  available: true,
  blocked_regions: [],
  blocked_reason: null,
  blocked_kind: null,
};

const DRIVER_BLOCK = JSON.stringify({
  code: 'ecram_driver_blocked',
  message:
    'ERAM could not be read: ecram_service answered "DeviceIoControl ECRAM_READ: Access is denied. (0x80070005)"',
});

const SERVICE_DOWN = JSON.stringify({
  code: 'ecram_service_unavailable',
  message:
    'ecram_service is not answering — Open pipe: The system cannot find the file specified. (0x80070002)',
});

function makeHw(overrides: Partial<Record<string, unknown>>): IotModulePanelHw {
  return {
    getEcramMap: vi.fn().mockResolvedValue(blockedMap),
    getIotRegionHex: vi.fn().mockResolvedValue('00ff'),
    readEcramRaw: vi.fn().mockResolvedValue('00ff'),
    writeIotHex: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  } as unknown as IotModulePanelHw;
}

describe('IotModulePanel — EC RAM honesty/UX', () => {
  it('regiões bloqueadas pelo driver: estado calmo (— e hint), sem banner de erro', async () => {
    const hw = makeHw({
      getIotRegionHex: vi.fn(async (region: string) => {
        if (region === 'ERAM' || region === 'SMA2') throw DRIVER_BLOCK;
        return region === 'IOT_STATUS' ? '0008010200000000' : 'aabbccdd';
      }),
    });
    render(<IotModulePanel hw={hw} />);

    expect(await screen.findByText(t('ecrDebug.ecramBlockedTitle'))).toBeInTheDocument();
    expect(screen.getByText(t('ecrDebug.ecramBlockedBody'))).toBeInTheDocument();
    // The expected driver block must NOT raise the alert banner.
    expect(screen.queryByRole('alert')).toBeNull();
    // EC-derived values are not fabricated: they render as "—".
    expect(screen.getAllByText('—').length).toBeGreaterThan(5);
    // The ranges the driver does expose keep their real bytes.
    expect(await screen.findByText('00 08 01 02 00 00 00 00')).toBeInTheDocument();
    // Technical detail stays available, collapsed.
    expect(screen.getByText(t('ecrDebug.ecramBlockedDetailLabel'))).toBeInTheDocument();
  });

  it('serviço EC em baixo: banner de erro real (role=alert)', async () => {
    const hw = makeHw({ getEcramMap: vi.fn().mockRejectedValue(SERVICE_DOWN) });
    render(<IotModulePanel hw={hw} />);

    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('ecram_service is not answering');
    expect(screen.queryByText(t('ecrDebug.ecramBlockedTitle'))).toBeNull();
  });

  it('ERAM legível: valores reais, sem card de bloqueio', async () => {
    const hw = makeHw({ getEcramMap: vi.fn().mockResolvedValue(availableMap) });
    render(<IotModulePanel hw={hw} />);

    expect(await screen.findByText('61 °C')).toBeInTheDocument();
    expect(screen.getByText('17120 mV')).toBeInTheDocument();
    expect(screen.queryByText(t('ecrDebug.ecramBlockedTitle'))).toBeNull();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('região inesperadamente falhada (não bloqueada) mantém o erro visível', async () => {
    const hw = makeHw({
      getEcramMap: vi.fn().mockResolvedValue(availableMap),
      getIotRegionHex: vi.fn(async (region: string) => {
        if (region === 'SMA2') throw SERVICE_DOWN;
        return '00ff';
      }),
    });
    render(<IotModulePanel hw={hw} />);

    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('SMA2');
  });
});
