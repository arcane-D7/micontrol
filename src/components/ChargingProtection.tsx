import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { t } from '../hooks/useI18n';
import { useToast } from '../contexts/ToastContext';
import { getUserFriendlyMessage, parseErrorResponse, type TranslateFn } from '../types/error';
import InfoModal, { InfoRow, InfoSection } from './InfoModal';

// `t` from useI18n types keys as a literal union; error.ts' TranslateFn takes
// any string — wrap it (matches the pattern used across the codebase).
const translate: TranslateFn = (key) => t(key as never);

interface Props {
  /** Current charging threshold (0-100, where 100 = no limit) */
  threshold: number;
  /** Callback to change the threshold via parent state */
  onThresholdChange: (threshold: number) => Promise<void>;
  /**
   * Live battery snapshot. Optional so the card still renders (and the tests
   * still mount it) without hardware; when present it powers the charging
   * state line — the app creates the state, so the app must name it
   * (audit §4.3 #2).
   */
  battery?: { level: number; is_charging: boolean; is_plugged: boolean } | null;
}

const LEVELS = [40, 50, 60, 70, 80] as const;

/** Local wall-clock HH:MM — the confirmation must say *when*, not just *what*. */
function clockTime(): string {
  const d = new Date();
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

/**
 * Unified charging protection card.
 *
 * Merges Battery Care (EC 0xA4 master toggle) and Charging Threshold
 * (EC 0xA7 charge limit) into a single, cohesive UI.
 *
 * - When Battery Care is ON, the threshold buttons are active and the EC
 *   respects the configured charge limit.
 * - When Battery Care is OFF, the threshold buttons are disabled and the
 *   EC charges to 100% regardless of the threshold register.
 */
export default function ChargingProtection({ threshold, onThresholdChange, battery }: Props) {
  const [batteryCare, setBatteryCare] = useState<boolean | null>(null);
  const [savingCare, setSavingCare] = useState(false);
  const [savingThreshold, setSavingThreshold] = useState(false);
  const [showInfo, setShowInfo] = useState(false);
  /** Last applied change, named with its value and the time it landed. */
  const [confirmation, setConfirmation] = useState<string | null>(null);
  const { addToast } = useToast();

  // threshold === 100 means "no limit" (disabled)
  const thresholdEnabled = threshold !== 100;
  const [lastLevel, setLastLevel] = useState<number>(
    LEVELS.includes(threshold as (typeof LEVELS)[number]) ? threshold : 80,
  );

  const fetchBatteryCare = useCallback(async () => {
    try {
      const care = await invoke<boolean>('get_battery_care');
      setBatteryCare(care);
    } catch {
      setBatteryCare(null);
    }
  }, []);

  useEffect(() => {
    void fetchBatteryCare();
  }, [fetchBatteryCare]);

  // ── Battery Care toggle (master switch) ──────────────────────────
  const handleToggleBatteryCare = async () => {
    if (batteryCare === null) return;
    const newState = !batteryCare;
    setSavingCare(true);
    try {
      await invoke('set_battery_care', { enabled: newState });
      setBatteryCare(newState);

      // When enabling battery care, also set the threshold to the last level
      // When disabling, set threshold to 100 (no limit)
      if (newState) {
        await onThresholdChange(lastLevel);
      } else {
        await onThresholdChange(100);
      }

      // Name the change, its value and its effect (audit §4.3 #5: a toast
      // that cannot name the delta should not exist).
      const message = t(newState ? 'charging.appliedCareOn' : 'charging.appliedCareOff', {
        time: clockTime(),
        limit: lastLevel,
      });
      setConfirmation(message);
      addToast({ message, type: 'info' });
    } catch (e) {
      addToast({
        message: `${t('charging.error')}: ${getUserFriendlyMessage(parseErrorResponse(e), translate)}`,
        type: 'error',
        onRetry: () => void handleToggleBatteryCare(),
      });
    } finally {
      setSavingCare(false);
    }
  };

  /**
   * Turn the protection on *and* apply the level in one interaction.
   *
   * Before, the 40–80% buttons were visible but dead (`pointerEvents: none`)
   * whenever protection was off — the user's first click did nothing and the
   * app read as broken (audit §4.3 #1). One click now does both, and the
   * confirmation says exactly what happened.
   */
  const enableCareAndApply = async (level: number) => {
    setSavingCare(true);
    try {
      await invoke('set_battery_care', { enabled: true });
      setBatteryCare(true);
      await onThresholdChange(level);
      setLastLevel(level);

      const message = t('charging.appliedCareOn', { time: clockTime(), limit: level });
      setConfirmation(message);
      addToast({ message, type: 'success' });
    } catch (e) {
      addToast({
        message: `${t('charging.error')}: ${getUserFriendlyMessage(parseErrorResponse(e), translate)}`,
        type: 'error',
        onRetry: () => void enableCareAndApply(level),
      });
    } finally {
      setSavingCare(false);
    }
  };

  // ── Threshold level change ──────────────────────────────────────
  const handleThresholdChange = async (level: number) => {
    // Protection off (or not readable) → switch it on *and* apply the level.
    // A click must never do nothing (audit §4.3 #1).
    if (!careEnabled) {
      await enableCareAndApply(level);
      return;
    }
    setSavingThreshold(true);
    try {
      await onThresholdChange(level);
      if (level !== 100) setLastLevel(level);
      const message = t('charging.appliedLimit', { level, time: clockTime() });
      setConfirmation(message);
      addToast({ message, type: 'success' });
    } catch (e) {
      addToast({
        message: `${t('charging.error')}: ${getUserFriendlyMessage(parseErrorResponse(e), translate)}`,
        type: 'error',
        onRetry: () => void handleThresholdChange(level),
      });
    } finally {
      setSavingThreshold(false);
    }
  };

  const careEnabled = batteryCare === true;
  // Protection off no longer blocks the level buttons — a click turns it on
  // and applies the level (see enableCareAndApply). Only an in-flight write
  // makes them inert.
  const saving = savingCare || savingThreshold;
  const disabled = saving;

  /**
   * The charging state the hardware is actually in, in user language.
   * 96% with an 80% limit used to read as "AC connected · 96%" and nothing
   * else — the app created a state it never named (audit §4.3 #2).
   */
  const chargingState = (() => {
    if (!careEnabled) return null;
    if (!thresholdEnabled) return t('charging.noLimit');
    if (!battery) return null;
    if (!battery.is_plugged) return t('charging.state.unplugged');
    if (battery.level > threshold)
      return t('charging.state.paused', { level: battery.level, limit: threshold });
    return t('charging.state.chargingTo', { limit: threshold });
  })();

  const showWhy =
    careEnabled && thresholdEnabled && !!battery && battery.is_plugged && battery.level > threshold;

  return (
    <div className="card">
      {/* Card header with info button */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          marginBottom: 12,
        }}
      >
        <h2 className="card-title" style={{ margin: 0 }}>
          {t('charging.title')}
        </h2>
        <button
          onClick={() => setShowInfo(true)}
          title={t('charging.infoModal.title')}
          style={{
            background: 'none',
            border: 'none',
            cursor: 'pointer',
            color: 'var(--text-dim)',
            fontSize: 16,
            lineHeight: 1,
            padding: '2px 4px',
            borderRadius: 'var(--r-xs)',
            transition: 'color var(--t-fast)',
          }}
          onMouseEnter={(e) => (e.currentTarget.style.color = 'var(--text)')}
          onMouseLeave={(e) => (e.currentTarget.style.color = 'var(--text-dim)')}
        >
          ⓘ
        </button>
      </div>

      <p className="page-subtitle" style={{ marginBottom: 16 }}>
        {t('charging.subtitle')}
      </p>

      {/* Battery Care master toggle */}
      <div className="stat-row" style={{ marginBottom: 16 }}>
        <span className="stat-label" id="battery-care-label">
          {t('battery.batteryCare')}
        </span>
        <label className="toggle-switch">
          <input
            type="checkbox"
            checked={careEnabled}
            disabled={savingCare || batteryCare === null}
            onChange={() => void handleToggleBatteryCare()}
            aria-label={t('battery.batteryCare')}
          />
          <span className="toggle-track" />
          <span className="toggle-knob" />
        </label>
      </div>

      {/* What the hardware is doing right now — named, not implied. */}
      <div className="charge-state" role="status" aria-live="polite">
        {batteryCare === null
          ? t('common.loading')
          : (chargingState ?? (careEnabled ? t('charging.noLimit') : t('battery.batteryCareOff')))}
      </div>

      {showWhy && battery && (
        <details className="charge-why">
          <summary>{t('common.why')}</summary>
          <p>{t('charging.state.why', { level: battery.level, limit: threshold })}</p>
        </details>
      )}

      {/* Confirmation of the last action: value + time + effect. */}
      {confirmation && (
        <div className="charge-confirm" role="status" aria-live="polite">
          <span className="charge-confirm-icon" aria-hidden="true">
            ✓
          </span>
          <span>{confirmation}</span>
        </div>
      )}

      {/* Threshold level buttons — always clickable; when the protection is
          off a click turns it on and applies the level in one step. */}
      <div style={{ marginBottom: 8 }}>
        <div className="stat-label" style={{ marginBottom: 10 }}>
          {t('charging.threshold')}
        </div>
        <div className={`threshold-options ${careEnabled ? '' : 'is-muted'}`}>
          {LEVELS.map((level) => (
            <button
              key={level}
              className={`threshold-btn ${threshold === level ? 'active' : ''}`}
              onClick={() => void handleThresholdChange(level)}
              disabled={disabled}
              aria-pressed={threshold === level}
            >
              {level}%
              {level === 80 && <span className="threshold-badge">{t('charging.recommended')}</span>}
            </button>
          ))}
        </div>
        {batteryCare === false && <p className="charge-hint">{t('charging.enableHint')}</p>}
      </div>

      {/* Show "no limit" indicator when threshold is 100 */}
      {careEnabled && !thresholdEnabled && (
        <div style={{ marginTop: 4, fontSize: 12, color: 'var(--text-muted)' }}>
          {t('charging.noLimit')}
        </div>
      )}

      {saving && (
        <div style={{ marginTop: 12, fontSize: 12, color: 'var(--text-muted)' }}>
          {t('charging.pending')}
        </div>
      )}

      {/* The engineering detail, one disclosure away — never in the main path. */}
      <details className="charge-why" style={{ marginTop: 12 }}>
        <summary>{t('common.technicalTerm')}</summary>
        <p>{t('charging.technicalTerm')}</p>
      </details>

      {/* Info modal */}
      <InfoModal
        open={showInfo}
        onClose={() => setShowInfo(false)}
        title={t('charging.infoModal.title')}
      >
        <InfoRow label={t('charging.infoModal.functionLabel')}>
          {t('charging.infoModal.functionDesc')}
        </InfoRow>
        <InfoRow label={t('charging.infoModal.requiresLabel')}>
          {t('charging.infoModal.requiresDesc')}
        </InfoRow>
        <InfoRow label={t('charging.infoModal.behaviorLabel')}>
          {t('charging.infoModal.behaviorDesc')}
        </InfoRow>
        <InfoSection>
          <div
            style={{
              background: 'oklch(from var(--warning, #ff9800) l c h / 0.12)',
              border: '1px solid oklch(from var(--warning, #ff9800) l c h / 0.3)',
              borderRadius: 'var(--r-sm)',
              padding: '10px 12px',
              fontSize: 12,
              color: 'var(--text-muted)',
              lineHeight: 1.6,
            }}
          >
            <strong style={{ color: 'var(--text)' }}>{t('charging.infoModal.warningLabel')}</strong>{' '}
            {t('charging.infoModal.warningDesc')}
          </div>
        </InfoSection>
      </InfoModal>
    </div>
  );
}
