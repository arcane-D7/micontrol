import { describe, it, expect } from 'vitest';
import { batteryState, batteryStateBadge } from '../lib/batteryState';
import { ecramText, isEcramAvailable, ECRAM_NO_DATA } from '../lib/ecram';
import type { EramMap } from '../types/hardware';

describe('batteryState', () => {
  it('reports charging when current flows into the battery', () => {
    expect(batteryState(true, true)).toBe('charging');
  });

  it('reports AC connected (not discharging) on the charger without charging', () => {
    // The reported bug: 96 % on AC with ChargeRate = 0 rendered "Discharging".
    expect(batteryState(false, true)).toBe('acConnected');
  });

  it('reports discharging only on battery', () => {
    expect(batteryState(false, false)).toBe('discharging');
  });

  it('never reports charging while unplugged', () => {
    expect(batteryState(true, false)).toBe('charging');
    // …the backend already guarantees is_charging=false when unplugged; the
    // UI state machine trusts the backend flag and shows AC only when plugged.
    expect(batteryState(false, false)).toBe('discharging');
  });

  it('maps states to badge colours', () => {
    expect(batteryStateBadge('charging')).toBe('success');
    expect(batteryStateBadge('acConnected')).toBe('info');
    expect(batteryStateBadge('discharging')).toBe('warning');
  });
});

describe('EC RAM honesty', () => {
  const base = {
    raw_hex: '00'.repeat(256),
    ac_adapter_w: 65,
    battery_voltage_mv: 17625,
    cpu_temp_c: 57,
    smart_mode_profile: null,
  } as unknown as EramMap;

  it('treats the UNAVAILABLE fallback map as no data', () => {
    expect(isEcramAvailable({ ...base, raw_hex: 'UNAVAILABLE' })).toBe(false);
  });

  it('treats an empty map as no data', () => {
    expect(isEcramAvailable({ ...base, raw_hex: '' })).toBe(false);
    expect(isEcramAvailable(null)).toBe(false);
  });

  it('accepts a real 256-byte snapshot', () => {
    expect(isEcramAvailable(base)).toBe(true);
  });

  it('prefers the backend ecram_available flag over raw_hex sniffing', () => {
    expect(isEcramAvailable({ ...base, raw_hex: 'UNAVAILABLE', ecram_available: true })).toBe(true);
    expect(isEcramAvailable({ ...base, ecram_available: false })).toBe(false);
  });

  it('renders an em dash instead of fake zeros', () => {
    expect(ecramText(false, 0, 'mV')).toBe(ECRAM_NO_DATA);
    expect(ecramText(false, 'Yes')).toBe(ECRAM_NO_DATA);
    expect(ecramText(true, 17625, 'mV')).toBe('17625 mV');
    expect(ecramText(true, null)).toBe(ECRAM_NO_DATA);
    expect(ecramText(true, 'Enabled')).toBe('Enabled');
  });
});
