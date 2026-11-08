/**
 * IoT Module panel — EC RAM access through the IoT driver.
 *
 * Shared by the System and Setup tabs (it used to be copy-pasted in both, and
 * the copies drifted). The honesty contract lives here:
 *
 *  - ERAM (0xFE0B0300) and SMA2 (0xFE0B0A00) are refused by the driver's
 *    address allowlist with 0x80070005 **even for SYSTEM**. That is this
 *    platform's EXPECTED state, so it renders as a calm "—" plus a hint —
 *    never as an error banner. The backend classifies it
 *    (`blocked_kind: 'driver_blocked'`).
 *  - A missing/dead `ecram_service` (or any unclassified failure) IS a fault
 *    and keeps the red banner.
 *  - The ranges the driver does expose (IOT_STATUS 0xFE0B0F00, IOT_SENSORS
 *    0xFE0B0F08, sensor block 0xFE0B0E00) keep showing real bytes.
 *  - Values that come from WMI (AC adapter watts, performance mode) stay real
 *    even when the EC block is blocked, and are labelled as such.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { Badge } from './ui';
import { Icon } from './ui/icons';
import { t } from '../hooks/useI18n';
import type { useHardware } from '../hooks/useHardware';
import type { EramMap, IotRegionName } from '../types/hardware';
import { getUserFriendlyMessage, parseErrorResponse, type TranslateFn } from '../types/error';

const translate: TranslateFn = (key) => t(key as never);

/** Safely convert any error to a display string, avoiding [object Object]. */
function errToString(e: unknown): string {
  return getUserFriendlyMessage(parseErrorResponse(e), translate);
}

type HardwareInstance = ReturnType<typeof useHardware>;

/** Props shape exported for tests and for the tabs that render the panel. */
export type IotModulePanelHw = HardwareInstance;

const IOT_REGIONS: IotRegionName[] = ['ERAM', 'SMA2', 'IOT_STATUS', 'IOT_SENSORS'];

/** Backend error code for "the driver refused this address" (expected). */
const CODE_DRIVER_BLOCKED = 'ecram_driver_blocked';

function formatHexPreview(hex: string, bytes = 16): string {
  if (!hex) return '—';
  const trimmed = hex.slice(0, bytes * 2);
  const pairs = trimmed.match(/.{1,2}/g) ?? [];
  return `${pairs.join(' ')}${hex.length > trimmed.length ? ' ...' : ''}`;
}

const ERAM_BASE = 0xfe0b0300;

function formatPerfProfile(value: number): string {
  switch (value) {
    case 0:
      return 'Balance';
    case 1:
      return 'Performance';
    case 2:
      return 'Silence';
    default:
      return `0x${value.toString(16).toUpperCase().padStart(2, '0')}`;
  }
}

const ERAM_KNOWN_WRITES: {
  label: string;
  offset: number;
  values: { label: string; byte: number }[];
}[] = [
  {
    label: 'Performance Profile',
    offset: 0x40,
    values: [
      { label: 'Balanced', byte: 0x00 },
      { label: 'Performance', byte: 0x01 },
      { label: 'Silent', byte: 0x02 },
    ],
  },
  {
    label: 'AI Limit (AILM)',
    offset: 0x1b,
    values: [
      { label: 'Enable AI Limit', byte: 0x04 },
      { label: 'Disable AI Limit', byte: 0x00 },
    ],
  },
  {
    label: 'Long Battery Limit (LBLM)',
    offset: 0x1b,
    values: [
      { label: 'Enable Long Battery Limit', byte: 0x08 },
      { label: 'Disable Long Battery Limit', byte: 0x00 },
    ],
  },
];

/** Per-region read outcome, kept so a blocked region is not mistaken for a fault. */
type RegionFailure = { code: string; message: string };

/** How a single region should be presented. */
type RegionState = 'ok' | 'blocked' | 'error' | 'pending';

const MUTED = 'var(--text-muted)';

export default function IotModulePanel({ hw }: { hw: HardwareInstance }) {
  const [ecramMap, setEcramMap] = useState<EramMap | null>(null);
  const [regions, setRegions] = useState<Partial<Record<IotRegionName, string>>>({});
  const [regionFailures, setRegionFailures] = useState<
    Partial<Record<IotRegionName, RegionFailure>>
  >({});
  const [loading, setLoading] = useState(false);
  /** Only UNEXPECTED failures land here (red banner). */
  const [error, setError] = useState<string | null>(null);
  const [rawReadAddr, setRawReadAddr] = useState('0xFE0B0300');
  const [rawReadCount, setRawReadCount] = useState('16');
  const [rawReadResult, setRawReadResult] = useState<string | null>(null);
  const [rawReadLoading, setRawReadLoading] = useState(false);
  const [writeAddress, setWriteAddress] = useState('0xFE0B0300');
  const [writeHex, setWriteHex] = useState('');
  const [writeStatus, setWriteStatus] = useState<string | null>(null);
  const timeoutRefs = useRef<number[]>([]);

  useEffect(() => {
    return () => {
      timeoutRefs.current.forEach((id) => clearTimeout(id));
      timeoutRefs.current = [];
    };
  }, []);

  const refreshIot = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const map = await hw.getEcramMap();
      setEcramMap(map);

      const hexes: Partial<Record<IotRegionName, string>> = {};
      const failures: Partial<Record<IotRegionName, RegionFailure>> = {};
      // One blocked region must not abort the panel: IOT_STATUS/IOT_SENSORS
      // and the map itself stay usable.
      await Promise.all(
        IOT_REGIONS.map(async (region) => {
          try {
            hexes[region] = await hw.getIotRegionHex(region);
          } catch (e) {
            const parsed = parseErrorResponse(e);
            failures[region] = { code: parsed.code, message: parsed.message };
          }
        }),
      );
      setRegions(hexes);
      setRegionFailures(failures);

      // Red banner only for failures that are NOT the platform's expected
      // driver block. A dead ecram_service is a real fault.
      const unexpected = IOT_REGIONS.filter((region) => {
        const failure = failures[region];
        if (!failure) return false;
        const blockedByMap = map.blocked_regions?.includes(region) ?? false;
        return failure.code !== CODE_DRIVER_BLOCKED && !blockedByMap;
      });
      if (unexpected.length > 0) {
        setError(
          unexpected.map((region) => `${region}: ${failures[region]?.message ?? ''}`).join(' · '),
        );
      }
    } catch (e) {
      // The map itself failed — that is never the expected state.
      setError(errToString(e));
    } finally {
      setLoading(false);
    }
  }, [hw]);

  useEffect(() => {
    void refreshIot();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleKnownWrite = useCallback(
    async (offset: number, byte: number) => {
      const addr = `0x${(ERAM_BASE + offset).toString(16).toUpperCase()}`;
      try {
        await hw.writeIotHex(addr, byte.toString(16).padStart(2, '0'));
        const refreshTimeout = window.setTimeout(() => void refreshIot(), 300);
        timeoutRefs.current.push(refreshTimeout);
      } catch (e) {
        setError(`Write failed: ${errToString(e)}`);
      }
    },
    [hw, refreshIot],
  );

  const handleRawRead = useCallback(async () => {
    setRawReadLoading(true);
    setRawReadResult(null);
    try {
      const count = parseInt(rawReadCount, 10);
      const hex = await hw.readEcramRaw(rawReadAddr, count);
      const bytes = hex.match(/.{1,2}/g) ?? [];
      const addrBase = parseInt(rawReadAddr.replace(/^0x/i, ''), 16) || 0;
      const lines: string[] = [];
      for (let i = 0; i < bytes.length; i += 16) {
        const lineAddr = `0x${(addrBase + i).toString(16).toUpperCase().padStart(8, '0')}`;
        const lineHex = bytes.slice(i, i + 16).join(' ');
        const lineAscii = bytes
          .slice(i, i + 16)
          .map((b) => {
            const c = parseInt(b, 16);
            return c >= 0x20 && c < 0x7f ? String.fromCharCode(c) : '.';
          })
          .join('');
        lines.push(`${lineAddr}: ${lineHex.padEnd(47)}  ${lineAscii}`);
      }
      setRawReadResult(lines.join('\n'));
    } catch (e) {
      setRawReadResult(`Error: ${errToString(e)}`);
    } finally {
      setRawReadLoading(false);
    }
  }, [hw, rawReadAddr, rawReadCount]);

  // ── Derived presentation state ────────────────────────────────────────────
  const ecAvailable = ecramMap?.available ?? false;
  const blockedKind = ecramMap?.blocked_kind ?? null;
  const blockedRegions = ecramMap?.blocked_regions ?? [];
  /** The platform's expected state: driver refuses ERAM/SMA2 by address.
   *  Falls back to `blocked_regions` for a backend that predates `blocked_kind`. */
  const driverBlocked =
    !ecAvailable &&
    (blockedKind === 'driver_blocked' || (blockedKind === null && blockedRegions.length > 0));

  const regionState = (region: IotRegionName): RegionState => {
    if (regions[region]) return 'ok';
    const failure = regionFailures[region];
    if (!failure) return 'pending';
    if (failure.code === CODE_DRIVER_BLOCKED || blockedRegions.includes(region)) return 'blocked';
    return 'error';
  };

  /** EC-derived value: real when the block is readable, "—" otherwise. */
  const ecValue = (value: string | number, unit = '') => (ecAvailable ? `${value}${unit}` : '—');

  return (
    <div className="card" style={{ marginTop: 14 }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 12,
          marginBottom: 14,
        }}
      >
        <div>
          <h2 className="card-title" style={{ marginBottom: 2 }}>
            IoT Module
          </h2>
          <div style={{ fontSize: 12, color: MUTED }}>Full ECRAM access via IoTDriver.</div>
        </div>
        <button className="btn-secondary" onClick={() => void refreshIot()} disabled={loading}>
          {loading ? 'Reading…' : 'Refresh'}
        </button>
      </div>

      {/* Red banner: UNEXPECTED failures only (service down, unknown error). */}
      {error && (
        <div
          role="alert"
          style={{
            marginBottom: 14,
            padding: '10px 14px',
            borderRadius: 10,
            background: 'color-mix(in srgb, var(--error) 10%, transparent)',
            border: '1px solid color-mix(in srgb, var(--error) 30%, transparent)',
            fontSize: 13,
            color: 'var(--error)',
          }}
        >
          {error}
        </div>
      )}

      {/* Calm state: the driver blocks these regions by design. */}
      {driverBlocked && (
        <div
          style={{
            marginBottom: 14,
            padding: '10px 14px',
            borderRadius: 10,
            background: 'var(--surface-2)',
            border: '1px solid var(--border)',
          }}
        >
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              flexWrap: 'wrap',
              marginBottom: 6,
            }}
          >
            <span style={{ fontSize: 13, fontWeight: 600 }}>{t('ecrDebug.ecramBlockedTitle')}</span>
            {blockedRegions.map((region) => (
              <Badge key={region} tone="neutral" dot>
                {region}
              </Badge>
            ))}
          </div>
          <p style={{ margin: 0, fontSize: 12, lineHeight: 1.5, color: MUTED }}>
            {t('ecrDebug.ecramBlockedBody')}
          </p>
          {ecramMap?.blocked_reason && (
            <details style={{ marginTop: 6 }}>
              <summary style={{ cursor: 'pointer', fontSize: 11, color: MUTED }}>
                {t('ecrDebug.ecramBlockedDetailLabel')}
              </summary>
              <p
                style={{
                  margin: '6px 0 0',
                  fontSize: 11,
                  lineHeight: 1.5,
                  color: MUTED,
                  fontFamily: 'var(--font-mono)',
                  wordBreak: 'break-word',
                }}
              >
                {ecramMap.blocked_reason}
              </p>
            </details>
          )}
        </div>
      )}

      {ecramMap && (
        <>
          <div className="grid-2" style={{ marginBottom: 14 }}>
            <div className="card" style={{ marginBottom: 0 }}>
              <h2 className="card-title" style={{ fontSize: 13 }}>
                System State
              </h2>
              <div className="stat-row">
                <span className="stat-label">{t('system.ac_connected')}</span>
                <span className="stat-value">
                  {ecAvailable ? (ecramMap.ac_connected ? 'Yes' : 'No') : '—'}
                </span>
              </div>
              <div className="stat-row">
                <span className="stat-label">{t('system.adapter_power')}</span>
                <span className="stat-value">
                  {ecramMap.ac_adapter_w > 0 ? `${ecramMap.ac_adapter_w} W` : '—'}
                  {!ecAvailable && ecramMap.ac_adapter_w > 0 && (
                    <span style={{ fontSize: 10, color: MUTED, marginLeft: 6 }}>WMI</span>
                  )}
                </span>
              </div>
              <div className="stat-row">
                <span className="stat-label">{t('system.battery_current')}</span>
                <span className="stat-value">{ecValue(ecramMap.battery_current_ma, ' mA')}</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">{t('system.battery_voltage')}</span>
                <span className="stat-value">{ecValue(ecramMap.battery_voltage_mv, ' mV')}</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">{t('system.battery_capacity')}</span>
                <span className="stat-value">{ecValue(ecramMap.battery_capacity_mah, ' mAh')}</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">{t('system.charge_limit')}</span>
                <span className="stat-value">{ecValue(ecramMap.charge_threshold_pct, ' %')}</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">{t('system.battery_temp')}</span>
                <span className="stat-value">{ecValue(ecramMap.battery_temp_c, ' °C')}</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">{t('system.cpu_temp')}</span>
                <span className="stat-value">{ecValue(ecramMap.cpu_temp_c, ' °C')}</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">{t('system.cpu_power')}</span>
                <span className="stat-value">{ecValue(ecramMap.cpu_power_w, ' W')}</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">{t('system.fan_1_rpm')}</span>
                <span className="stat-value">{ecValue(ecramMap.fan_rpm)}</span>
              </div>
              {ecAvailable && ecramMap.fan2_rpm > 0 && (
                <div className="stat-row">
                  <span className="stat-label">{t('system.fan_2_rpm')}</span>
                  <span className="stat-value">{ecramMap.fan2_rpm}</span>
                </div>
              )}
            </div>
            <div className="card" style={{ marginBottom: 0 }}>
              <h2 className="card-title" style={{ fontSize: 13 }}>
                Mode / Limits (decoded)
              </h2>
              <div className="stat-row">
                <span className="stat-label">{t('system.performance_profile')}</span>
                <span className="stat-value">
                  {ecAvailable || ecramMap.perf_profile_from_wmi
                    ? `${formatPerfProfile(ecramMap.perf_profile)} (0x${ecramMap.perf_profile
                        .toString(16)
                        .toUpperCase()})`
                    : '—'}
                  {!ecAvailable && ecramMap.perf_profile_from_wmi && (
                    <span style={{ fontSize: 10, color: MUTED, marginLeft: 6 }}>WMI</span>
                  )}
                </span>
              </div>
              <div className="stat-row">
                <span className="stat-label">{t('system.tdp_override')}</span>
                <span className="stat-value">{ecValue(ecramMap.tdp_w, ' W')}</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">{t('system.smart_profile')}</span>
                <span className="stat-value">
                  {ecAvailable ? (ecramMap.smart_mode_profile ?? '—') : '—'}
                </span>
              </div>
              <div className="stat-row">
                <span className="stat-label">SMMT</span>
                <span className="stat-value">
                  {ecAvailable
                    ? `0x${ecramMap.smart_mode_type.toString(16).toUpperCase().padStart(2, '0')}`
                    : '—'}
                </span>
              </div>
              <div className="stat-row">
                <span className="stat-label">SMMD</span>
                <span className="stat-value">
                  {ecAvailable
                    ? `0x${ecramMap.smart_mode_data.toString(16).toUpperCase().padStart(2, '0')}`
                    : '—'}
                </span>
              </div>
              <div className="stat-row">
                <span className="stat-label">QFAN</span>
                <span className="stat-value">
                  {ecAvailable
                    ? `0x${ecramMap.qfan_mode.toString(16).toUpperCase().padStart(2, '0')}`
                    : '—'}
                </span>
              </div>
              <div className="stat-row">
                <span className="stat-label">{t('system.ai_limit_ailm')}</span>
                <span className="stat-value">
                  {ecAvailable ? (ecramMap.ai_limit_enabled ? 'Enabled' : 'Disabled') : '—'}
                </span>
              </div>
              <div className="stat-row">
                <span className="stat-label">{t('system.long_battery_limit')}</span>
                <span className="stat-value">
                  {ecAvailable
                    ? ecramMap.long_battery_limit_enabled
                      ? 'Enabled'
                      : 'Disabled'
                    : '—'}
                </span>
              </div>
              <div className="stat-row">
                <span className="stat-label">{t('system.display_brightness')}</span>
                <span className="stat-value">{ecValue(ecramMap.display_brightness_level)}</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">{t('system.kb_backlight')}</span>
                <span className="stat-value">{ecValue(ecramMap.keyboard_backlight_level)}</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">control_flags[0x1B]</span>
                <span className="stat-value">
                  {ecAvailable
                    ? `0x${ecramMap.control_flags_1b.toString(16).toUpperCase().padStart(2, '0')}`
                    : '—'}
                </span>
              </div>
            </div>
          </div>

          <div className="card" style={{ marginBottom: 14 }}>
            <h2 className="card-title" style={{ fontSize: 13 }}>
              Write Controls (known-safe registers)
            </h2>
            {driverBlocked && (
              <p style={{ margin: '0 0 10px', fontSize: 12, color: MUTED }}>
                {t('ecrDebug.ecramWriteBlockedHint')}
              </p>
            )}
            <div style={{ display: 'grid', gap: 12 }}>
              {ERAM_KNOWN_WRITES.map((reg) => (
                <div
                  key={reg.label}
                  style={{ display: 'flex', gap: 10, alignItems: 'center', flexWrap: 'wrap' }}
                >
                  <span style={{ fontSize: 12, fontWeight: 600, minWidth: 180 }}>
                    {reg.label}
                    <span
                      style={{
                        fontFamily: 'var(--font-mono)',
                        fontSize: 10,
                        color: MUTED,
                        marginLeft: 6,
                      }}
                    >
                      +0x{reg.offset.toString(16).toUpperCase().padStart(2, '0')}
                    </span>
                  </span>
                  {reg.values.map((v) => (
                    <button
                      key={v.label}
                      className="btn-secondary"
                      style={{ fontSize: 11, padding: '4px 10px' }}
                      disabled={!ecAvailable}
                      onClick={() => void handleKnownWrite(reg.offset, v.byte)}
                    >
                      {v.label}
                    </button>
                  ))}
                </div>
              ))}
            </div>
          </div>
        </>
      )}

      <div style={{ display: 'grid', gap: 8, marginBottom: 14 }}>
        {IOT_REGIONS.map((region) => {
          const state = regionState(region);
          return (
            <details
              key={region}
              style={{
                border: '1px solid var(--border)',
                borderRadius: 'var(--r-sm)',
                padding: '8px 12px',
                background: 'var(--surface-2)',
              }}
            >
              <summary
                style={{
                  cursor: 'pointer',
                  fontWeight: 500,
                  fontSize: 13,
                  display: 'flex',
                  justifyContent: 'space-between',
                  alignItems: 'center',
                  gap: 8,
                }}
              >
                <span style={{ display: 'inline-flex', alignItems: 'center', gap: 8 }}>
                  {region}
                  {state === 'blocked' && (
                    <Badge tone="neutral">{t('ecrDebug.ecramRegionBlockedChip')}</Badge>
                  )}
                  {state === 'error' && (
                    <Badge tone="danger">{t('ecrDebug.ecramRegionErrorChip')}</Badge>
                  )}
                </span>
                <span
                  style={{
                    fontFamily: 'var(--font-mono)',
                    fontSize: 11,
                    color: MUTED,
                  }}
                >
                  {state === 'ok' ? formatHexPreview(regions[region] ?? '') : '—'}
                </span>
              </summary>
              <pre
                style={{
                  marginTop: 8,
                  fontSize: 11,
                  whiteSpace: 'pre-wrap',
                  wordBreak: 'break-all',
                  color: state === 'error' ? 'var(--error)' : MUTED,
                }}
              >
                {state === 'ok'
                  ? regions[region]
                  : state === 'blocked'
                    ? t('ecrDebug.ecramRegionBlockedHint')
                    : state === 'error'
                      ? (regionFailures[region]?.message ?? t('ecrDebug.ecramRegionErrorHint'))
                      : '—'}
              </pre>
            </details>
          );
        })}
      </div>

      <details style={{ marginBottom: 10 }}>
        <summary style={{ cursor: 'pointer', fontWeight: 600, fontSize: 13, padding: '8px 0' }}>
          Raw Read — arbitrary address
        </summary>
        <div className="card" style={{ marginTop: 8, marginBottom: 0 }}>
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: '1fr auto auto',
              gap: 8,
              alignItems: 'end',
              marginBottom: 8,
            }}
          >
            <div>
              <div style={{ fontSize: 11, color: MUTED, marginBottom: 4 }}>
                Physical address (hex)
              </div>
              <input
                className="text-input"
                value={rawReadAddr}
                onChange={(e) => setRawReadAddr(e.target.value)}
                placeholder="0xFE0B0300"
              />
            </div>
            <div>
              <div style={{ fontSize: 11, color: MUTED, marginBottom: 4 }}>Bytes (1–256)</div>
              <input
                className="text-input"
                value={rawReadCount}
                onChange={(e) => setRawReadCount(e.target.value)}
                placeholder="16"
                style={{ width: 72 }}
              />
            </div>
            <button
              className="btn-secondary"
              disabled={rawReadLoading}
              onClick={() => void handleRawRead()}
            >
              {rawReadLoading ? 'Reading…' : 'Read'}
            </button>
          </div>
          {rawReadResult && (
            <pre
              style={{
                fontSize: 11,
                whiteSpace: 'pre-wrap',
                wordBreak: 'break-all',
                color: MUTED,
                marginTop: 4,
                fontFamily: 'var(--font-mono)',
              }}
            >
              {rawReadResult}
            </pre>
          )}
        </div>
      </details>

      <details>
        <summary style={{ cursor: 'pointer', fontWeight: 600, fontSize: 13, padding: '8px 0' }}>
          Raw Write — arbitrary address
          <span
            style={{
              fontSize: 11,
              color: 'var(--warning)',
              marginLeft: 8,
              fontWeight: 400,
            }}
          >
            <Icon name="alert-triangle" size={16} /> danger zone
          </span>
        </summary>
        <div className="card" style={{ marginTop: 8, marginBottom: 0 }}>
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: '1fr 1fr auto',
              gap: 8,
              alignItems: 'end',
            }}
          >
            <div>
              <div style={{ fontSize: 11, color: MUTED, marginBottom: 4 }}>
                Physical address (hex)
              </div>
              <input
                className="text-input"
                value={writeAddress}
                onChange={(e) => setWriteAddress(e.target.value)}
                placeholder="0xFE0B0300"
              />
            </div>
            <div>
              <div style={{ fontSize: 11, color: MUTED, marginBottom: 4 }}>Hex data</div>
              <input
                className="text-input"
                value={writeHex}
                onChange={(e) => setWriteHex(e.target.value)}
                placeholder="DEADBEEF"
              />
            </div>
            <button
              className="btn-secondary"
              disabled={loading || !writeHex.trim()}
              onClick={async () => {
                setWriteStatus(null);

                const addrClean = writeAddress.trim().replace(/^0[xX]/, '');
                if (!/^[0-9A-Fa-f]+$/.test(addrClean)) {
                  setWriteStatus('Write failed: invalid address format (must be hex)');
                  return;
                }

                const hexClean = writeHex.replace(/\s+/g, '');
                if (!/^[0-9A-Fa-f]+$/.test(hexClean)) {
                  setWriteStatus('Write failed: invalid hex data (must be 0-9, A-F)');
                  return;
                }

                if (
                  !window.confirm(
                    'WARNING: Writing to a raw physical address can brick your device. Are you sure?',
                  )
                ) {
                  return;
                }

                try {
                  await hw.writeIotHex(writeAddress, writeHex);
                  setWriteStatus('Write OK');
                  const refreshTimeout = window.setTimeout(() => void refreshIot(), 200);
                  timeoutRefs.current.push(refreshTimeout);
                } catch (e) {
                  setWriteStatus(`Write failed: ${errToString(e)}`);
                }
              }}
            >
              Write
            </button>
            {writeStatus && (
              <span
                style={{
                  fontSize: 12,
                  color: writeStatus.startsWith('Write failed') ? 'var(--error)' : 'var(--success)',
                }}
              >
                {writeStatus}
              </span>
            )}
          </div>
        </div>
      </details>
    </div>
  );
}
