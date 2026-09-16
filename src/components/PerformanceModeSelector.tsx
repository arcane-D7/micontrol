import { useState } from 'react';
import { t } from '../hooks/useI18n';
import type { PerformanceMode } from '../types/hardware';
import { useToast } from '../contexts/ToastContext';

interface Props {
  current: PerformanceMode;
  onChange: (mode: PerformanceMode) => Promise<void>;
  disabled?: boolean;
  /** Whether the user has configured an AI API key (gates Smart modes) */
  aiApiKeySet?: boolean;
  /** Navigate to Settings tab to configure API key */
  onOpenSettings?: () => void;
  /**
   * While `true`, the UI shows a pending state on the selected mode and
   * disables the other buttons — the async confirmation from the backend is
   * awaited (no physical timers).
   */
  applying?: boolean;
}

type ModeCategory = 'economy' | 'performance' | 'overpower';

/** Modes grouped by category, in the display order the user requested:
 *  Economy (battery-saving) first, then Performance, then Overpower
 *  (experimental, hardware-stressing — gated behind a confirmation modal). */
const MODE_GROUPS: Array<{ category: ModeCategory; modes: typeof MODES }> = [
  {
    category: 'economy',
    modes: [
      { key: 'silence', icon: '🔇', labelKey: 'silence', descKey: 'silence', detailKey: 'silence' },
      {
        key: 'long_battery',
        icon: '🍃',
        labelKey: 'longBattery',
        descKey: 'longBattery',
        detailKey: 'longBattery',
      },
    ],
  },
  {
    category: 'performance',
    modes: [
      { key: 'balance', icon: '⚖️', labelKey: 'balance', descKey: 'balance', detailKey: 'balance' },
      { key: 'turbo', icon: '⚡', labelKey: 'turbo', descKey: 'turbo', detailKey: 'turbo' },
      {
        key: 'decepticon',
        icon: '🌡️',
        labelKey: 'decepticon',
        descKey: 'decepticon',
        detailKey: 'decepticon',
      },
      { key: 'smart', icon: '🧠', labelKey: 'smart', descKey: 'smart', detailKey: 'smart' },
      {
        key: 'smart_acceleration',
        icon: '🚀',
        labelKey: 'smartAcceleration',
        descKey: 'smartAcceleration',
        detailKey: 'smartAcceleration',
      },
      {
        key: 'smart_adaptive',
        icon: '🎯',
        labelKey: 'smartAdaptive',
        descKey: 'smartAdaptive',
        detailKey: 'smartAdaptive',
      },
    ],
  },
  {
    category: 'overpower',
    modes: [
      {
        key: 'overdrive',
        icon: '🔥',
        labelKey: 'overdrive',
        descKey: 'overdrive',
        detailKey: 'overdrive',
      },
      {
        key: 'overdrive_high',
        icon: '🌋',
        labelKey: 'overdriveHigh',
        descKey: 'overdriveHigh',
        detailKey: 'overdriveHigh',
      },
      {
        key: 'overdrive_max',
        icon: '☠️',
        labelKey: 'overdriveMax',
        descKey: 'overdriveMax',
        detailKey: 'overdriveMax',
      },
    ],
  },
];

const MODES: Array<{
  key: PerformanceMode;
  icon: string;
  labelKey: keyof (typeof import('../i18n/en.json'))['performance']['modes'];
  descKey: keyof (typeof import('../i18n/en.json'))['performance']['descriptions'];
  detailKey: keyof (typeof import('../i18n/en.json'))['performance']['techDetails']['modes'];
  requiresAi?: true;
}> = MODE_GROUPS.flatMap((g) => g.modes);

/** Hardware constants per mode (not translated — numbers / proper nouns).
 *  S60c: only VERIFIED data is shown in the button spec row. `sustW` comes
 *  from the nominal PL1 setpoints (TDP_SETPOINTS — the same values the
 *  Performance Monitor uses as its cap reference when RAPL reads 0 on
 *  Panther Lake). `peakW` is the documented burst value from MODE_SPECS.tdp.
 *  NO temperature is shown: per-mode peak temps were never measured on this
 *  machine (the S60b tempC column was removed for that reason — the old
 *  techDetails numbers were written for a different EC/firmware state and
 *  presenting them as measurements was wrong). Live temperature is shown in
 *  the Performance Monitor from the ESIF/ACPI sensor chain instead. */
const MODE_SPECS: Record<
  PerformanceMode,
  {
    tdp: string;
    fan: string;
    windowsOverlay: string;
    accentColor: string;
    peakW: string;
    sustW: string;
  }
> = {
  silence: {
    tdp: '~65 W burst (~34 s) / ~35 W sust.',
    fan: 'Max RPM (thermal)',
    windowsOverlay: 'Power Saver',
    accentColor: 'var(--info)',
    peakW: '~65 W',
    sustW: '~7 W',
  },
  balance: {
    tdp: '~60 W burst (~11 s) / ~35 W sust.',
    fan: 'Adaptive 2 000–4 500 RPM',
    windowsOverlay: 'Balanced',
    accentColor: 'var(--success)',
    peakW: '~60 W',
    sustW: '~15 W',
  },
  turbo: {
    tdp: '~62 W burst (~5 s) / ~15 W sust.',
    fan: 'Aggressive 4 000–5 500 RPM',
    windowsOverlay: 'Best Performance',
    accentColor: 'var(--warning)',
    peakW: '~62 W',
    sustW: '~25 W',
  },
  smart: {
    tdp: '~62 W burst (~5 s) / ~15 W sust. (AI)',
    fan: 'Variable — follows load',
    windowsOverlay: 'Balanced',
    accentColor: 'var(--accent)',
    peakW: '~62 W',
    sustW: '~15 W',
  },
  long_battery: {
    tdp: '~60 W burst (~2 s) / ~42 W sust.',
    fan: 'Moderate 2 000–3 500 RPM',
    windowsOverlay: 'Power Saver',
    accentColor: 'var(--success)',
    peakW: '~60 W',
    sustW: '~6 W',
  },
  decepticon: {
    tdp: '~35-40 W flat (no burst phase)',
    fan: 'Steady ~3 500 RPM',
    windowsOverlay: 'Best Performance',
    accentColor: 'var(--error)',
    peakW: '~40 W',
    sustW: '~35 W',
  },
  smart_acceleration: {
    tdp: '~65 W burst (~12 s) / ~38 W sust. (AI)',
    fan: 'Reactive — spikes on demand',
    windowsOverlay: 'Balanced',
    accentColor: 'var(--accent)',
    peakW: '~65 W',
    sustW: '~20 W',
  },
  overdrive: {
    tdp: '~65-83 W burst (~17 s) / ~50 W sust.',
    fan: 'Max 5 000–5 500 RPM',
    windowsOverlay: 'Best Performance',
    accentColor: '#ff6a00',
    peakW: '~83 W',
    sustW: '~50 W',
  },
  overdrive_high: {
    tdp: '~62-67 W sustained (uncapped PL1)',
    fan: 'Max 5 000–5 500 RPM',
    windowsOverlay: 'Best Performance',
    accentColor: '#ff3300',
    peakW: '~67 W',
    sustW: '~62 W',
  },
  overdrive_max: {
    tdp: '~60-77 W sustained (uncapped PL1)',
    fan: 'Max 5 000–5 500+ RPM',
    windowsOverlay: 'Best Performance',
    accentColor: '#cc0000',
    peakW: '~77 W',
    sustW: '~60 W',
  },
  smart_adaptive: {
    tdp: '~65-73 W burst (~13 s) / ~35 W sust.',
    fan: 'Variable — EC-controlled',
    windowsOverlay: 'Balanced',
    accentColor: '#00b4d8',
    peakW: '~73 W',
    sustW: '~35 W',
  },
};

export default function PerformanceModeSelector({
  current,
  onChange,
  disabled,
  aiApiKeySet = false,
  onOpenSettings,
  applying = false,
}: Props) {
  const spec = MODE_SPECS[current];
  const showSmartDiff = current === 'smart' || current === 'smart_acceleration';
  const { addToast } = useToast();
  // S60: overpower modes require an explicit confirmation modal — they run
  // the hardware at its limits and can shorten its useful life.
  const [pendingOverpower, setPendingOverpower] = useState<PerformanceMode | null>(null);

  async function applyMode(key: PerformanceMode) {
    try {
      await onChange(key);
      addToast({ message: t('performance.applied'), type: 'success' });
    } catch (e) {
      addToast({
        message: `${t('performance.error')}: ${String(e)}`,
        type: 'error',
        onRetry: () => applyMode(key),
      });
    }
  }

  async function handleModeChange(key: PerformanceMode) {
    if (MODE_GROUPS.find((g) => g.category === 'overpower')!.modes.some((m) => m.key === key)) {
      setPendingOverpower(key);
      return;
    }
    await applyMode(key);
  }

  function confirmOverpower() {
    const key = pendingOverpower;
    setPendingOverpower(null);
    if (key) void applyMode(key);
  }

  const pendingSpec = pendingOverpower ? MODE_SPECS[pendingOverpower] : null;
  const pendingMode = MODES.find((m) => m.key === pendingOverpower);

  return (
    <div>
      {MODE_GROUPS.map((group, gi) => (
        <div key={group.category}>
          {gi > 0 && (
            <div className="mode-group-divider" role="separator" aria-orientation="horizontal" />
          )}
          <div className="mode-group-header">
            {t(`performance.groups.${group.category}` as Parameters<typeof t>[0])}
          </div>
          <div className="mode-grid">
            {group.modes.map((m) => {
              const aiLocked = !!m.requiresAi && !aiApiKeySet;
              const isCurrent = current === m.key;
              const showPending = applying && isCurrent;
              const isOverpower = group.category === 'overpower';
              return (
                <button
                  key={m.key}
                  className={`mode-btn ${isCurrent ? 'active' : ''} ${showPending ? 'applying' : ''} ${aiLocked ? 'ai-locked' : ''} ${isOverpower ? 'overpower' : ''}`}
                  onClick={() => {
                    if (aiLocked) {
                      onOpenSettings?.();
                      return;
                    }
                    void handleModeChange(m.key);
                  }}
                  disabled={(disabled || applying) && !aiLocked}
                  title={
                    aiLocked
                      ? t('performance.techDetails.aiLockedMsg')
                      : t(`performance.descriptions.${m.descKey}` as Parameters<typeof t>[0])
                  }
                >
                  <span className="mode-btn-icon">{showPending ? '⏳' : m.icon}</span>
                  <span className="mode-btn-name">
                    {t(`performance.modes.${m.labelKey}` as Parameters<typeof t>[0])}
                    {showPending && (
                      <span
                        style={{
                          marginLeft: 4,
                          fontSize: 10,
                          color: 'var(--text-dim)',
                          verticalAlign: 'middle',
                        }}
                      >
                        …
                      </span>
                    )}
                    {aiLocked && (
                      <span
                        style={{
                          marginLeft: 4,
                          fontSize: 10,
                          color: 'var(--text-dim)',
                          verticalAlign: 'middle',
                        }}
                        title={t('performance.techDetails.aiLockedMsg')}
                      >
                        🔒
                      </span>
                    )}
                  </span>
                  {/* S60b: measured power/thermal spec row — peak TDP,
                      sustained TDP and expected peak temperature. */}
                  <span className="mode-btn-specs">
                    <span className="mode-btn-spec">
                      <span className="mode-btn-spec-label">
                        {t('performance.techDetails.peak')}
                      </span>
                      <span className="mode-btn-spec-value">{MODE_SPECS[m.key].peakW}</span>
                    </span>
                    <span className="mode-btn-spec">
                      <span className="mode-btn-spec-label">
                        {t('performance.techDetails.sustained')}
                      </span>
                      <span className="mode-btn-spec-value">{MODE_SPECS[m.key].sustW}</span>
                    </span>
                  </span>
                  <span className="mode-btn-desc">
                    {aiLocked
                      ? t('performance.techDetails.requiresApiKey')
                      : t(`performance.descriptions.${m.descKey}` as Parameters<typeof t>[0])}
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      ))}

      {/* Technical details for the active mode */}
      {spec && (
        <div
          style={{
            marginTop: 16,
            padding: '16px 18px',
            background: 'var(--surface-2)',
            borderRadius: 'var(--r-sm)',
            borderLeft: `3px solid ${spec.accentColor}`,
          }}
        >
          <div
            style={{
              fontSize: 11,
              fontWeight: 600,
              color: 'var(--text-dim)',
              textTransform: 'uppercase',
              letterSpacing: '0.08em',
              marginBottom: 12,
            }}
          >
            {t('performance.techDetails.title')}
          </div>

          {/* Spec row: TDP / Fan / Windows overlay */}
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: '1fr 1fr 1fr',
              gap: '10px 16px',
              marginBottom: 12,
            }}
          >
            <div>
              <div style={{ fontSize: 10, color: 'var(--text-muted)', marginBottom: 2 }}>
                {t('performance.techDetails.tdp')}
              </div>
              <div
                style={{
                  fontSize: 13,
                  fontWeight: 600,
                  color: 'var(--text)',
                  fontFamily: 'var(--font-mono)',
                }}
              >
                {spec.tdp}
              </div>
            </div>
            <div>
              <div style={{ fontSize: 10, color: 'var(--text-muted)', marginBottom: 2 }}>
                {t('performance.techDetails.fanBehavior')}
              </div>
              <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text)' }}>{spec.fan}</div>
            </div>
            <div>
              <div style={{ fontSize: 10, color: 'var(--text-muted)', marginBottom: 2 }}>
                {t('performance.techDetails.windowsOverlay')}
              </div>
              <div style={{ fontSize: 13, fontWeight: 600, color: spec.accentColor }}>
                {spec.windowsOverlay}
              </div>
            </div>
          </div>

          {/* Detailed description — now pulled from i18n */}
          <div style={{ fontSize: 12, color: 'var(--text-muted)', lineHeight: 1.6 }}>
            {t(
              `performance.techDetails.modes.${MODES.find((m) => m.key === current)?.detailKey ?? 'balance'}` as Parameters<
                typeof t
              >[0],
            )}
          </div>

          {/* Windows overlay note */}
          <div
            style={{
              marginTop: 10,
              fontSize: 11,
              color: 'var(--text-dim)',
              display: 'flex',
              alignItems: 'flex-start',
              gap: 6,
            }}
          >
            <span style={{ flexShrink: 0 }} aria-hidden="true">
              ℹ️
            </span>
            <span>{t('performance.techDetails.overlayNote')}</span>
          </div>

          {/* Smart vs Smart Acceleration comparison */}
          {showSmartDiff && (
            <div
              style={{
                marginTop: 12,
                padding: '10px 14px',
                background: 'var(--surface-3, var(--surface))',
                borderRadius: 'var(--r-sm)',
                borderLeft: '3px solid var(--accent)',
                fontSize: 12,
                color: 'var(--text-muted)',
                lineHeight: 1.55,
              }}
            >
              <span style={{ fontWeight: 600, color: 'var(--accent)' }}>
                Smart vs Smart Acceleration —{' '}
              </span>
              {t('performance.techDetails.smartDiff')}
            </div>
          )}
        </div>
      )}

      {/* S60: overpower confirmation modal — experimental modes that push the
          hardware to its limits and can shorten its useful life. */}
      {pendingOverpower && pendingMode && (
        <div
          className="overpower-modal-backdrop"
          role="dialog"
          aria-modal="true"
          aria-labelledby="overpower-modal-title"
          onClick={() => setPendingOverpower(null)}
        >
          <div
            className="overpower-modal"
            onClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => {
              if (e.key === 'Escape') setPendingOverpower(null);
            }}
          >
            <div className="overpower-modal-icon" aria-hidden="true">
              ⚠️
            </div>
            <h2 id="overpower-modal-title">{t('performance.overpowerModal.title')}</h2>
            <p className="overpower-modal-mode">
              {pendingMode.icon}{' '}
              {t(`performance.modes.${pendingMode.labelKey}` as Parameters<typeof t>[0])}
              {pendingSpec && <span className="overpower-modal-spec"> · {pendingSpec.tdp}</span>}
            </p>
            <p>{t('performance.overpowerModal.body')}</p>
            <div className="overpower-modal-actions">
              <button
                type="button"
                className="overpower-modal-cancel"
                onClick={() => setPendingOverpower(null)}
              >
                {t('performance.overpowerModal.cancel')}
              </button>
              <button type="button" className="overpower-modal-confirm" onClick={confirmOverpower}>
                {t('performance.overpowerModal.confirm')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
