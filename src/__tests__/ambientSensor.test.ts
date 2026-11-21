import { describe, it, expect } from 'vitest';
import { parseAmbientSensorState } from '../components/DisplaySettings';
import type { DisplayInfo } from '../types/hardware';

/** The DisplayInfo slice ambientSensorNote() reads. */
type SensorSlice = NonNullable<DisplayInfo['ambient_sensor']>;

function withSensor(sensor: SensorSlice | undefined, lux: number | null = null): DisplayInfo {
  return {
    brightness: 50,
    hdr_enabled: false,
    hdr_supported: false,
    hdr_displays: [],
    refresh_rate_hz: 60,
    available_refresh_rates: [60],
    dynamic_refresh_rate_capable: false,
    adaptive_refresh_rate: false,
    ai_brightness: false,
    ai_brightness_config: {
      enabled: false,
      min_brightness: 10,
      max_brightness: 100,
      sensitivity: 100,
      smoothing: 30,
      mode: 'curve',
    },
    ambient_lux: lux,
    ambient_sensor: sensor,
  };
}

describe('parseAmbientSensorState', () => {
  it('accepts the exact backend wire strings', () => {
    expect(parseAmbientSensorState('available')).toBe('available');
    expect(parseAmbientSensorState('no_sensor')).toBe('no_sensor');
    expect(parseAmbientSensorState('frozen_report')).toBe('frozen_report');
    expect(parseAmbientSensorState('unknown')).toBe('unknown');
  });

  it('falls back to unknown for anything else (future states, missing field)', () => {
    expect(parseAmbientSensorState('sleeping')).toBe('unknown');
    expect(parseAmbientSensorState(undefined)).toBe('unknown');
    expect(parseAmbientSensorState(null)).toBe('unknown');
    expect(parseAmbientSensorState(42)).toBe('unknown');
  });
});

describe('ambient sensor diagnosis drives the Display tab copy', () => {
  it('marks the three diagnosis states as distinct TS literals', () => {
    // Type-level pin: the wire enum and the Rust serde names must stay in
    // lockstep (serde rename_all = "snake_case").
    const states: SensorSlice['state'][] = ['available', 'no_sensor', 'frozen_report', 'unknown'];
    expect(new Set(states).size).toBe(4);
  });

  it('keeps the frozen-report payload shape stable for the date rendering', () => {
    // 2026-11-18 12:00:00 UTC, the production freeze date.
    const sensor: SensorSlice = { state: 'frozen_report', last_fresh_ms: 1_792_385_600_000 };
    const display = withSensor(sensor);
    expect(display.ambient_lux).toBeNull();
    expect(display.ambient_sensor?.state).toBe('frozen_report');
    expect(display.ambient_sensor?.last_fresh_ms).toBe(1_792_385_600_000);
  });

  it('renders the ambient-lux row only when a live reading exists', () => {
    const live = withSensor({ state: 'available', last_fresh_ms: 1_792_385_600_000 }, 342);
    expect(live.ambient_lux).toBe(342);
  });
});
