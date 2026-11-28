import type { EramMap } from '../types/hardware';

/**
 * Placeholder rendered instead of a number that is not a measurement.
 * U+2014 EM DASH, the same glyph the rest of the UI already uses for "no data".
 */
export const ECRAM_NO_DATA = '—';

/**
 * Whether the EC RAM snapshot behind an `EramMap` holds real measurements.
 *
 * `get_ecram_map` falls back to `partial_eram_map_from_wmi()` when the IoT
 * driver denies the ERAM read on this platform: every numeric field is 0 and
 * `raw_hex` is the literal `"UNAVAILABLE"`. Those zeros are absence of data,
 * not "0 W" / "0 mV" / "0 °C" — rendering them as measurements tells the user
 * the hardware is idle at 0 V while the PC is on the charger.
 *
 * Prefers the backend's explicit `ecram_available` flag when it exists and
 * falls back to inspecting `raw_hex` for older backends.
 */
export function isEcramAvailable(map: EramMap | null | undefined): boolean {
  if (!map) return false;
  if (typeof map.ecram_available === 'boolean') return map.ecram_available;
  if (typeof map.raw_hex !== 'string' || map.raw_hex.length === 0) return false;
  return map.raw_hex.trim().toUpperCase() !== 'UNAVAILABLE';
}

/**
 * Format one EC value: `value` + optional unit when the map is trustworthy,
 * `—` otherwise. Every reader of `EramMap` must go through this so a blocked
 * ERAM never renders as a reading.
 */
export function ecramText(
  available: boolean,
  value: number | string | null | undefined,
  unit?: string,
): string {
  if (!available) return ECRAM_NO_DATA;
  if (value === null || value === undefined) return ECRAM_NO_DATA;
  return unit ? `${value} ${unit}` : String(value);
}
