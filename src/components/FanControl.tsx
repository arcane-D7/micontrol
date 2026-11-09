import { useState } from 'react';
import { t } from '../hooks/useI18n';
import type { FanInfo, FanModeResult } from '../types/hardware';
import { useToast } from '../contexts/ToastContext';
import { Icon } from './ui/icons';
import { InfoHint } from './ui/info-hint';

interface Props {
  fan: FanInfo | null;
  onModeChange: (
    mode: 'auto' | 'fixed' | 'off',
    speedPercent?: number,
  ) => Promise<FanModeResult | void>;
}

function FanSpeedVisual({ speedPct }: { speedPct: number }) {
  const segs = 12;
  const active = Math.round((speedPct / 100) * segs);
  return (
    <div className="fan-segments">
      {Array.from({ length: segs }, (_, i) => (
        <div
          key={i}
          className={`fan-seg ${i < active ? 'active' : ''}`}
          style={{ height: `${40 + (i / segs) * 60}%` }}
        />
      ))}
    </div>
  );
}

export default function FanControl({ fan, onModeChange }: Props) {
  const [speed, setSpeed] = useState(fan?.requested_speed_percent ?? 50);
  const [mode, setMode] = useState<'auto' | 'fixed' | 'off'>(fan?.mode ?? 'auto');
  const [lastResult, setLastResult] = useState<FanModeResult | null>(null);
  const { addToast } = useToast();

  const handleModeChange = async (m: 'auto' | 'fixed' | 'off') => {
    const previous = mode;
    setMode(m);
    try {
      const result = await onModeChange(m, m === 'fixed' ? speed : undefined);
      setLastResult(result ?? null);
      // The command reports what actually changed. When the platform cannot
      // honour the request (no RPM control, no fan stop) the toast carries the
      // caveat instead of a bare "applied".
      addToast({
        message: result?.warning ?? t('fan.applied'),
        type: result?.warning ? 'info' : 'success',
      });
    } catch (e) {
      // Nothing was applied — don't leave the button highlighted as if it were.
      setMode(previous);
      addToast({
        message: `${t('fan.error')}: ${String(e)}`,
        type: 'error',
        onRetry: () => void handleModeChange(m),
      });
    }
  };

  if (!fan) {
    return (
      <div className="card">
        <h2 className="card-title">{t('fan.title')}</h2>
        <div className="skeleton" style={{ height: 120 }} />
      </div>
    );
  }

  // `speed_rpm === null` means the platform exposes no tachometer — not that
  // the fan has stopped. Nothing measured is shown as a number.
  const hasRpmReading = fan.speed_rpm != null;

  return (
    <div className="card">
      <h2 className="card-title mc-label-with-hint">
        <span>{t('fan.title')}</span>
        <InfoHint
          label={t('hint.fanModes')}
          buttonLabel={t('hint.about', { name: t('fan.title') })}
        />
      </h2>

      <div className="grid-3" style={{ marginBottom: 20 }}>
        {(['auto', 'fixed', 'off'] as const).map((m) => (
          <button
            key={m}
            className={`mode-btn ${mode === m ? 'active' : ''}`}
            onClick={() => void handleModeChange(m)}
            style={{ padding: '12px 8px' }}
          >
            <span className="mode-btn-icon">
              {m === 'auto' ? (
                <Icon name="refresh" size={16} />
              ) : m === 'fixed' ? (
                <Icon name="wrench" size={16} />
              ) : (
                <Icon name="volume-x" size={16} />
              )}
            </span>
            <span className="mode-btn-name">{t(`fan.modes.${m}` as Parameters<typeof t>[0])}</span>
          </button>
        ))}
      </div>

      {mode === 'fixed' && (
        <>
          <div className="slider-row" style={{ marginBottom: 8 }}>
            <span className="slider-label">{t('fan.speed')}</span>
            <input
              type="range"
              min={20}
              max={100}
              step={5}
              value={speed}
              onChange={(e) => setSpeed(Number(e.target.value))}
              onMouseUp={() => void handleModeChange('fixed')}
              onTouchEnd={() => void handleModeChange('fixed')}
            />
            <span className="slider-value">{speed}%</span>
          </div>
          {/* The slider selects an EC fan-profile band; it does not pin an
              exact RPM on hardware without an IGCL fan handle. */}
          <p style={{ fontSize: 11, color: 'var(--text-muted)', margin: '0 0 16px' }}>
            {t('fan.fixedProfileNote')}
          </p>
        </>
      )}

      {lastResult?.warning && (
        <p
          style={{
            fontSize: 11,
            color: 'var(--text-muted)',
            margin: '0 0 16px',
            padding: '8px 10px',
            background: 'var(--surface-2)',
            borderRadius: 'var(--r-sm)',
          }}
        >
          <Icon name="info" size={16} /> {lastResult.warning}
        </p>
      )}

      <div className="grid-2">
        <div>
          <div className="stat-label" style={{ marginBottom: 8 }}>
            {t('fan.current')}
          </div>
          {hasRpmReading ? (
            <>
              <FanSpeedVisual speedPct={fan.speed_percent ?? 0} />
              <div style={{ marginTop: 4, fontSize: 13, fontWeight: 600 }}>
                {fan.speed_rpm?.toLocaleString()} {t('fan.rpm')}
              </div>
            </>
          ) : (
            <>
              <div
                className="card-value"
                style={{ fontSize: 20, color: 'var(--text-dim)' }}
                title={t('fan.noSensorHint')}
              >
                N/A
              </div>
              <div style={{ marginTop: 4, fontSize: 11, color: 'var(--text-muted)' }}>
                {t('fan.noSensor')}
              </div>
            </>
          )}
        </div>
        <div>
          <div className="stat-label" style={{ marginBottom: 8 }}>
            {t('fan.temperature')}
          </div>
          <div className="card-value" style={{ fontSize: 22 }}>
            {fan.gpu_temp_celsius != null ? `${fan.gpu_temp_celsius}°` : '—'}
          </div>
          {fan.ambient_temp_celsius != null && (
            <div style={{ fontSize: 11, color: 'var(--text-muted)' }}>
              {t('fan.ambient')}: {fan.ambient_temp_celsius.toFixed(1)}°
            </div>
          )}
        </div>
      </div>

      <p style={{ fontSize: 11, color: 'var(--color-warning)', marginTop: 12 }}>
        <Icon name="alert-triangle" size={16} /> {t('fan.warning')}
      </p>
    </div>
  );
}
