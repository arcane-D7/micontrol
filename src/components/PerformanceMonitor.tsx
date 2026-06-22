import { t } from '../hooks/useI18n';
import type { FanInfo, SystemInfo, PerformanceMode, PerformanceResult } from '../hooks/useHardware';

interface Props {
  fan: FanInfo | null;
  systemInfo: SystemInfo | null;
  currentMode: PerformanceMode;
  lastResult: PerformanceResult | null;
}

// Nominal PL1 (sustained) TDP setpoints per mode, used when RAPL measurement
// is unavailable. Panther Lake firmware returns 0 from the ACPI Power Meter.
const TDP_SETPOINTS: Record<string, number> = {
  silence: 7,
  balance: 15,
  turbo: 25,
  decepticon: 35,
  smart: 15,
  long_battery: 6,
  smart_acceleration: 20,
};

function tempColor(temp: number): string {
  if (temp >= 90) return 'var(--error)';
  if (temp >= 75) return 'var(--warning)';
  return 'var(--success)';
}

function tdpColor(w: number): string {
  if (w >= 35) return 'var(--error)';
  if (w >= 20) return 'var(--warning)';
  return 'var(--info)';
}

// Temperature bar: 40°C = 0%, 100°C = 100%
const tempPct = (t: number) => Math.min(Math.max(((t - 40) / 60) * 100, 0), 100);

interface MetricProps {
  label: string;
  value: React.ReactNode;
  sub?: React.ReactNode;
  bar?: { pct: number; cls: string };
}

function Metric({ label, value, sub, bar }: MetricProps) {
  return (
    <div>
      <div style={{ fontSize: 11, color: 'var(--text-muted)', marginBottom: 6 }}>{label}</div>
      <div style={{ fontSize: 24, fontWeight: 700, fontFamily: 'var(--font-mono)', lineHeight: 1 }}>
        {value}
      </div>
      {sub && <div style={{ fontSize: 11, color: 'var(--text-muted)', marginTop: 4 }}>{sub}</div>}
      {bar && (
        <div className="progress-bar" style={{ marginTop: 8 }}>
          <div
            className={`progress-fill ${bar.cls}`}
            style={{ width: `${bar.pct}%`, transition: 'width 600ms ease' }}
          />
        </div>
      )}
    </div>
  );
}

export default function PerformanceMonitor({ fan, systemInfo, currentMode, lastResult }: Props) {
  if (!fan && !systemInfo) return null;

  // Build METHOD_LABELS inside component so t() reflects current locale
  const METHOD_LABELS: Record<string, string> = {
    'hq_wmi+registry+overlay': t('performance.monitor.methodLabels.hqWmi'),
    'vhf+registry+overlay': t('performance.monitor.methodLabels.vhf'),
    'registry+overlay': t('performance.monitor.methodLabels.overlay'),
  };

  const cpuTemp = fan?.cpu_temp_celsius ?? 0;
  const gpuTemp = fan?.gpu_temp_celsius ?? 0;
  const fanRpm = fan?.speed_rpm ?? 0;
  const tdp = fan?.tdp_watts ?? null;
  const cpuLoad = systemInfo?.cpu_usage ?? null;
  const gpuLoad = systemInfo?.gpu_usage ?? null;

  // Setpoint for current mode (used as cap reference and fallback when RAPL is null)
  const setpoint = TDP_SETPOINTS[currentMode] ?? null;
  const tdpEstimated = tdp === null ? setpoint : null;
  const tdpDisplay = tdp ?? tdpEstimated;
  const tdpIsEstimated = tdp === null && tdpDisplay !== null;

  const tdpLabel = tdpIsEstimated
    ? t('performance.monitor.tdpEstLabel')
    : t('performance.monitor.tdpRealLabel');
  const tdpValueStr =
    tdpDisplay !== null
      ? tdpIsEstimated
        ? `~${tdpDisplay.toFixed(0)} W`
        : `${tdpDisplay.toFixed(1)} W`
      : null;
  // Below the real TDP, show the configured setpoint for context
  const tdpSub =
    !tdpIsEstimated && setpoint !== null
      ? t('performance.monitor.tdpCapLine').replace('{value}', String(setpoint))
      : undefined;

  return (
    <div className="card">
      <div className="card-title">{t('performance.monitor.title')}</div>

      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(4, 1fr)',
          gap: '10px 14px',
          marginBottom: lastResult ? 16 : 0,
        }}
      >
        {/* CPU Temp + CPU Load */}
        <Metric
          label={t('performance.monitor.cpuTemp')}
          value={<span style={{ color: tempColor(cpuTemp) }}>{cpuTemp.toFixed(0)}°C</span>}
          sub={
            cpuLoad !== null ? (
              <span style={{ color: 'var(--info)' }}>
                {cpuLoad.toFixed(0)}% {t('performance.monitor.cpuLoad')}
              </span>
            ) : undefined
          }
          bar={{ pct: tempPct(cpuTemp), cls: 'temp' }}
        />

        {/* GPU Temp + GPU Load */}
        <Metric
          label={t('performance.monitor.gpuTemp')}
          value={<span style={{ color: tempColor(gpuTemp) }}>{gpuTemp.toFixed(0)}°C</span>}
          sub={
            gpuLoad !== null ? (
              <span style={{ color: 'var(--info)' }}>
                {gpuLoad.toFixed(0)}% {t('performance.monitor.gpuLoad')}
              </span>
            ) : undefined
          }
          bar={{ pct: tempPct(gpuTemp), cls: 'gpu' }}
        />

        {/* TDP */}
        <Metric
          label={tdpLabel}
          value={
            tdpValueStr !== null ? (
              <span style={{ color: tdpColor(tdpDisplay!) }}>{tdpValueStr}</span>
            ) : (
              <span style={{ color: 'var(--text-dim)', fontSize: 16 }}>—</span>
            )
          }
          sub={tdpSub}
          bar={
            tdpDisplay !== null
              ? { pct: Math.min((tdpDisplay / 45) * 100, 100), cls: 'ram' }
              : undefined
          }
        />

        {/* Fan */}
        <Metric
          label={t('performance.monitor.fan')}
          value={
            fanRpm > 0 ? (
              <span style={{ color: 'var(--text)' }}>{fanRpm.toLocaleString()}</span>
            ) : (
              <span style={{ color: 'var(--text-dim)', fontSize: 16 }}>N/A</span>
            )
          }
          sub={fanRpm > 0 ? t('performance.monitor.rpm') : t('performance.monitor.noWmiSensor')}
        />
      </div>

      {/* Activation method confirmation */}
      {lastResult && (
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 10,
            padding: '10px 14px',
            background: 'var(--surface-2)',
            borderRadius: 'var(--r-sm)',
            marginTop: 16,
            borderLeft: `3px solid ${lastResult.success ? 'var(--success)' : 'var(--error)'}`,
          }}
        >
          <span className={`badge ${lastResult.success ? 'success' : 'error'}`}>
            {lastResult.success
              ? `✓ ${t('performance.monitor.modeActive')}`
              : `✗ ${t('performance.monitor.failed')}`}
          </span>
          <span style={{ fontSize: 12, color: 'var(--text-muted)' }}>
            {t('performance.monitor.via')}{' '}
            <span style={{ color: 'var(--text)', fontWeight: 500 }}>
              {METHOD_LABELS[lastResult.method] ?? lastResult.method}
            </span>
          </span>
        </div>
      )}
    </div>
  );
}
