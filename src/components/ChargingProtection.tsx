import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { t } from '../hooks/useI18n';
import { useToast } from '../contexts/ToastContext';
import InfoModal, { InfoRow, InfoSection } from './InfoModal';

interface Props {
  /** Current charging threshold (0-100, where 100 = no limit) */
  threshold: number;
  /** Callback to change the threshold via parent state */
  onThresholdChange: (threshold: number) => Promise<void>;
}

const LEVELS = [40, 50, 60, 70, 80] as const;

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
export default function ChargingProtection({ threshold, onThresholdChange }: Props) {
  const [batteryCare, setBatteryCare] = useState<boolean | null>(null);
  const [savingCare, setSavingCare] = useState(false);
  const [savingThreshold, setSavingThreshold] = useState(false);
  const [showInfo, setShowInfo] = useState(false);
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

      addToast({
        message: newState ? t('charging.limitEnabled') : t('charging.limitDisabled'),
        type: 'info',
      });
    } catch (e) {
      addToast({
        message: `${t('charging.error')}: ${String(e)}`,
        type: 'error',
        onRetry: () => void handleToggleBatteryCare(),
      });
    } finally {
      setSavingCare(false);
    }
  };

  // ── Threshold level change ──────────────────────────────────────
  const handleThresholdChange = async (level: number) => {
    setSavingThreshold(true);
    try {
      await onThresholdChange(level);
      if (level !== 100) setLastLevel(level);
      addToast({ message: t('charging.applied'), type: 'success' });
    } catch (e) {
      addToast({
        message: `${t('charging.error')}: ${String(e)}`,
        type: 'error',
        onRetry: () => void handleThresholdChange(level),
      });
    } finally {
      setSavingThreshold(false);
    }
  };

  const careEnabled = batteryCare === true;
  const disabled = savingCare || savingThreshold || batteryCare === null || !careEnabled;

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
        <div className="card-title" style={{ margin: 0 }}>
          {t('charging.title')}
        </div>
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
        <span className="stat-label">{t('battery.batteryCare')}</span>
        <label className="toggle-switch">
          <input
            type="checkbox"
            checked={careEnabled}
            disabled={savingCare || batteryCare === null}
            onChange={() => void handleToggleBatteryCare()}
          />
          <span className="toggle-track" />
          <span className="toggle-knob" />
        </label>
      </div>

      {/* Status indicator */}
      <div style={{ marginBottom: 12, fontSize: 12, color: 'var(--text-muted)' }}>
        {batteryCare === null
          ? t('common.loading')
          : careEnabled
            ? thresholdEnabled
              ? t('charging.limitEnabled')
              : t('charging.noLimit')
            : t('battery.batteryCareOff')}
      </div>

      {/* Threshold level buttons — only interactive when battery care is enabled */}
      <div
        style={{
          marginBottom: 8,
          opacity: careEnabled ? 1 : 0.4,
          pointerEvents: careEnabled ? 'auto' : 'none',
        }}
      >
        <div className="stat-label" style={{ marginBottom: 10 }}>
          {t('charging.threshold')}
        </div>
        <div className="threshold-options">
          {LEVELS.map((level) => (
            <button
              key={level}
              className={`threshold-btn ${threshold === level ? 'active' : ''}`}
              onClick={() => void handleThresholdChange(level)}
              disabled={disabled}
            >
              {level}%
              {level === 80 && <span className="threshold-badge">{t('charging.recommended')}</span>}
            </button>
          ))}
        </div>
      </div>

      {/* Show "no limit" indicator when threshold is 100 */}
      {careEnabled && !thresholdEnabled && (
        <div style={{ marginTop: 4, fontSize: 12, color: 'var(--text-muted)' }}>
          {t('charging.noLimit')}
        </div>
      )}

      {(savingCare || savingThreshold) && (
        <div style={{ marginTop: 12, fontSize: 12, color: 'var(--text-muted)' }}>
          {t('charging.applying')}
        </div>
      )}

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
