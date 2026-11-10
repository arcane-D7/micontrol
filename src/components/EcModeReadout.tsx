import { t, type StringKey } from '../hooks/useI18n';
import type { FanInfo } from '../types/hardware';

/**
 * The EC's **active** performance mode — the machine's real mode.
 *
 * Everything else in the performance UI is a stored preference
 * (`HKLM\SOFTWARE\MI\PerformanceMode`, `HKLM\SOFTWARE\MI\FanControl`) or a
 * Windows overlay GUID. On this platform those can disagree with the EC for
 * hours: the owner's registry said "Balance" while the EC sat at QFAN=9
 * (Smart Adaptive, the aggressive profile). Showing the preference alone is
 * therefore not a report of machine state, which is why this line exists next
 * to the selector.
 *
 * Honesty rules baked in:
 * - no reading → `—` plus the reason, never a guessed mode;
 * - the firmware's ACPI read only names five modes, so every other value is
 *   shown as "not nameable (raw 0)" instead of being mapped to something
 *   convenient like "Silence".
 */
const LABEL_KEYS: Record<NonNullable<FanInfo['ec_active']>, StringKey> = {
  turbo: 'fan.ecTurbo',
  decepticon: 'fan.ecDecepticon',
  overdrive: 'fan.ecOverdrive',
  smart_adaptive: 'fan.ecSmartAdaptive',
  smart: 'fan.ecSmart',
  unreported: 'fan.ecUnreported',
};

export default function EcModeReadout({ fan }: { fan: FanInfo | null }) {
  const mode = fan?.ec_active ?? null;
  const raw = fan?.ec_active_raw ?? null;
  const known = mode != null && fan != null;

  return (
    <div style={{ marginBottom: 16 }}>
      <div className="stat-label" style={{ marginBottom: 6 }}>
        {t('fan.ecActive')}
      </div>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
        <span style={{ fontSize: 18, fontWeight: 600 }}>
          {known && mode ? t(LABEL_KEYS[mode]) : t('fan.ecUnavailable')}
        </span>
        {known && raw != null && (
          <span style={{ fontSize: 12, color: 'var(--text-muted)', fontVariantNumeric: 'tabular-nums' }}>
            raw {raw}
          </span>
        )}
      </div>
      <p style={{ fontSize: 11, color: 'var(--text-muted)', margin: '6px 0 0' }}>
        {t('fan.ecActiveHint')}
      </p>
      {mode === 'unreported' && (
        <p style={{ fontSize: 11, color: 'var(--text-muted)', margin: '4px 0 0' }}>
          {t('fan.ecUnreportedHint')}
        </p>
      )}
    </div>
  );
}