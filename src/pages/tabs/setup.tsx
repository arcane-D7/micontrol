import { useState, useEffect, useCallback, useRef } from 'react';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import HardwareDiscovery from '../../components/HardwareDiscovery';
import type { useHardware } from '../../hooks/useHardware';
import type { IotRegionName } from '../../types/hardware';
import type { Hardware } from './shared';

type HardwareInstance = ReturnType<typeof useHardware>;

const IOT_REGIONS: IotRegionName[] = ['ERAM', 'SMA2', 'IOT_STATUS', 'IOT_SENSORS'];

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

function IotModulePanel({ hw }: { hw: HardwareInstance }) {
  const [ecramMap, setEcramMap] = useState<import('../../hooks/useHardware').EramMap | null>(null);
  const [regions, setRegions] = useState<
    Partial<Record<import('../../hooks/useHardware').IotRegionName, string>>
  >({});
  const [loading, setLoading] = useState(false);
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
      const [map, ...hexes] = await Promise.all([
        hw.getEcramMap(),
        ...IOT_REGIONS.map((region) => hw.getIotRegionHex(region)),
      ]);
      setEcramMap(map);
      setRegions({ ERAM: hexes[0], SMA2: hexes[1], IOT_STATUS: hexes[2], IOT_SENSORS: hexes[3] });
    } catch (e) {
      setError(String(e));
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
        setError(`Write failed: ${String(e)}`);
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
      setRawReadResult(`Error: ${String(e)}`);
    } finally {
      setRawReadLoading(false);
    }
  }, [hw, rawReadAddr, rawReadCount]);

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
          <div className="card-title" style={{ marginBottom: 2 }}>
            IoT Module
          </div>
          <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>
            Full ECRAM access via IoTDriver.
          </div>
        </div>
        <button className="btn-secondary" onClick={() => void refreshIot()} disabled={loading}>
          {loading ? 'Reading…' : 'Refresh'}
        </button>
      </div>

      {error && (
        <div
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

      {ecramMap && (
        <>
          <div className="grid-2" style={{ marginBottom: 14 }}>
            <div className="card" style={{ marginBottom: 0 }}>
              <div className="card-title" style={{ fontSize: 13 }}>
                System State
              </div>
              <div className="stat-row">
                <span className="stat-label">AC connected</span>
                <span className="stat-value">{ecramMap.ac_connected ? 'Yes' : 'No'}</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">Adapter power</span>
                <span className="stat-value">{ecramMap.ac_adapter_w} W</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">Battery current</span>
                <span className="stat-value">{ecramMap.battery_current_ma} mA</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">Battery voltage</span>
                <span className="stat-value">{ecramMap.battery_voltage_mv} mV</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">Battery capacity</span>
                <span className="stat-value">{ecramMap.battery_capacity_mah} mAh</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">Charge limit</span>
                <span className="stat-value">{ecramMap.charge_threshold_pct} %</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">Battery temp</span>
                <span className="stat-value">{ecramMap.battery_temp_c} °C</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">CPU temp</span>
                <span className="stat-value">{ecramMap.cpu_temp_c} °C</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">CPU power</span>
                <span className="stat-value">{ecramMap.cpu_power_w} W</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">Fan 1 RPM</span>
                <span className="stat-value">{ecramMap.fan_rpm}</span>
              </div>
              {ecramMap.fan2_rpm > 0 && (
                <div className="stat-row">
                  <span className="stat-label">Fan 2 RPM</span>
                  <span className="stat-value">{ecramMap.fan2_rpm}</span>
                </div>
              )}
            </div>
            <div className="card" style={{ marginBottom: 0 }}>
              <div className="card-title" style={{ fontSize: 13 }}>
                Mode / Limits (decoded)
              </div>
              <div className="stat-row">
                <span className="stat-label">Performance profile</span>
                <span className="stat-value">
                  {formatPerfProfile(ecramMap.perf_profile)} (0x
                  {ecramMap.perf_profile.toString(16).toUpperCase()})
                </span>
              </div>
              <div className="stat-row">
                <span className="stat-label">TDP override</span>
                <span className="stat-value">{ecramMap.tdp_w} W</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">Smart profile</span>
                <span className="stat-value">{ecramMap.smart_mode_profile ?? '—'}</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">SMMT</span>
                <span className="stat-value">
                  0x{ecramMap.smart_mode_type.toString(16).toUpperCase().padStart(2, '0')}
                </span>
              </div>
              <div className="stat-row">
                <span className="stat-label">SMMD</span>
                <span className="stat-value">
                  0x{ecramMap.smart_mode_data.toString(16).toUpperCase().padStart(2, '0')}
                </span>
              </div>
              <div className="stat-row">
                <span className="stat-label">QFAN</span>
                <span className="stat-value">
                  0x{ecramMap.qfan_mode.toString(16).toUpperCase().padStart(2, '0')}
                </span>
              </div>
              <div className="stat-row">
                <span className="stat-label">AI limit (AILM)</span>
                <span className="stat-value">
                  {ecramMap.ai_limit_enabled ? 'Enabled' : 'Disabled'}
                </span>
              </div>
              <div className="stat-row">
                <span className="stat-label">Long battery limit</span>
                <span className="stat-value">
                  {ecramMap.long_battery_limit_enabled ? 'Enabled' : 'Disabled'}
                </span>
              </div>
              <div className="stat-row">
                <span className="stat-label">Display brightness</span>
                <span className="stat-value">{ecramMap.display_brightness_level}</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">KB backlight</span>
                <span className="stat-value">{ecramMap.keyboard_backlight_level}</span>
              </div>
              <div className="stat-row">
                <span className="stat-label">control_flags[0x1B]</span>
                <span className="stat-value">
                  0x{ecramMap.control_flags_1b.toString(16).toUpperCase().padStart(2, '0')}
                </span>
              </div>
            </div>
          </div>

          <div className="card" style={{ marginBottom: 14 }}>
            <div className="card-title" style={{ fontSize: 13 }}>
              Write Controls (known-safe registers)
            </div>
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
                        color: 'var(--text-muted)',
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
        {IOT_REGIONS.map((region) => (
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
              }}
            >
              <span>{region}</span>
              <span
                style={{ fontFamily: 'var(--font-mono)', fontSize: 11, color: 'var(--text-muted)' }}
              >
                {formatHexPreview(regions[region] ?? '')}
              </span>
            </summary>
            <pre
              style={{
                marginTop: 8,
                fontSize: 11,
                whiteSpace: 'pre-wrap',
                wordBreak: 'break-all',
                color: 'var(--text-muted)',
              }}
            >
              {regions[region] ?? 'No data'}
            </pre>
          </details>
        ))}
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
              <div style={{ fontSize: 11, color: 'var(--text-muted)', marginBottom: 4 }}>
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
              <div style={{ fontSize: 11, color: 'var(--text-muted)', marginBottom: 4 }}>
                Bytes (1–256)
              </div>
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
                color: 'var(--text-muted)',
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
          <span style={{ fontSize: 11, color: 'var(--warning)', marginLeft: 8, fontWeight: 400 }}>
            ⚠ danger zone
          </span>
        </summary>
        <div className="card" style={{ marginTop: 8, marginBottom: 0 }}>
          <div style={{ fontSize: 12, color: 'var(--text-muted)', marginBottom: 10 }}>
            Direct write to any physical address via IOCTL 0x22E004.
          </div>
          <div
            style={{
              fontSize: 12,
              color: 'var(--warning)',
              background: 'var(--bg-hover)',
              border: '1px solid var(--warning)',
              borderRadius: 'var(--r-xs)',
              padding: '8px 10px',
              marginBottom: 10,
            }}
          >
            ⚠ DANGER: Writing to a raw physical address can permanently brick your device.
            Double-check the address and data before proceeding.
          </div>
          <div style={{ display: 'grid', gap: 8 }}>
            <input
              className="text-input"
              value={writeAddress}
              onChange={(e) => setWriteAddress(e.target.value)}
              placeholder="0xFE0B0300"
            />
            <textarea
              className="text-input"
              value={writeHex}
              onChange={(e) => setWriteHex(e.target.value)}
              placeholder="hex bytes e.g. 01 02 03 04"
              rows={3}
              style={{ resize: 'vertical', fontFamily: 'var(--font-mono)', fontSize: 12 }}
            />
            <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
              <button
                className="btn-secondary"
                disabled={loading || !writeHex.trim()}
                onClick={async () => {
                  setWriteStatus(null);

                  // Validate address format (hex, with or without 0x prefix)
                  const addrClean = writeAddress.trim().replace(/^0[xX]/, '');
                  if (!/^[0-9A-Fa-f]+$/.test(addrClean)) {
                    setWriteStatus('Write failed: invalid address format (must be hex)');
                    return;
                  }

                  // Validate hex data format (whitespace stripped)
                  const hexClean = writeHex.replace(/\s+/g, '');
                  if (!/^[0-9A-Fa-f]+$/.test(hexClean)) {
                    setWriteStatus('Write failed: invalid hex data (must be 0-9, A-F)');
                    return;
                  }

                  // Confirmation dialog — raw physical writes can brick the device
                  if (
                    !window.confirm(
                      '⚠️ WARNING: Writing to a raw physical address can brick your device. Are you sure?',
                    )
                  ) {
                    return;
                  }

                  try {
                    await hw.writeIotHex(writeAddress, writeHex);
                    setWriteStatus('Write OK');
                    const writeRefreshTimeout = window.setTimeout(() => void refreshIot(), 200);
                    timeoutRefs.current.push(writeRefreshTimeout);
                  } catch (e) {
                    setWriteStatus(`Write failed: ${String(e)}`);
                  }
                }}
              >
                Write
              </button>
              {writeStatus && (
                <span
                  style={{
                    fontSize: 12,
                    color: writeStatus.startsWith('Write failed')
                      ? 'var(--error)'
                      : 'var(--success)',
                  }}
                >
                  {writeStatus}
                </span>
              )}
            </div>
          </div>
        </div>
      </details>
    </div>
  );
}

interface Props {
  hw: Hardware;
}

export default function SetupTab({ hw }: Props) {
  return (
    <>
      <PageHeader title={t('discovery.title')} subtitle={t('discovery.subtitle')} />
      <HardwareDiscovery
        profile={hw.hardwareProfile}
        loading={hw.loadingDiscovery}
        onRescan={hw.runHardwareDiscovery}
        onInstallDriver={hw.installDriver}
      />
      <IotModulePanel hw={hw as unknown as HardwareInstance} />
    </>
  );
}
