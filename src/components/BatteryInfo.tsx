import { useState } from 'react';
import { t } from '../hooks/useI18n';
import { batteryState, batteryStateBadge } from '../lib/batteryState';
import type { BatteryInfo as BatteryData } from '../types/hardware';
import InfoModal, { InfoRow, InfoSection } from './InfoModal';

interface Props {
  battery: BatteryData | null;
  /** Active charge limit (0–100). When set, time-to-full says which level it
   *  is counting to — "Faltam para os 80%" answers the question the bare
   *  duration does not (glossary §4.4). */
  limit?: number;
}

function formatTime(minutes: number | null): string {
  if (minutes == null || minutes < 0) return t('common.unknown');
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return h > 0
    ? `${h}${t('battery.hours')} ${m}${t('battery.minutes')}`
    : `${m}${t('battery.minutes')}`;
}

function healthColor(pct: number): string {
  if (pct >= 80) return 'success';
  if (pct >= 60) return 'warning';
  return 'error';
}

function batteryColor(level: number): string {
  if (level > 30) return 'battery';
  if (level > 15) return 'battery warning';
  return 'battery critical';
}

/** mWh → Wh string */
function mwhToWh(mwh: number): string {
  return (mwh / 1000).toFixed(1);
}

/** mWh + voltage_mv → estimated mAh */
function mwhToMah(mwh: number, voltage_mv: number): string | null {
  if (voltage_mv <= 0) return null;
  return Math.round((mwh * 1000) / voltage_mv).toLocaleString();
}

/** A number without a scale is a number without meaning (glossary §4.4). */
function healthWord(pct: number): string {
  if (pct >= 80) return t('battery.healthGood');
  if (pct >= 60) return t('battery.healthFair');
  return t('battery.healthPoor');
}

/**
 * `--success` is tuned for fills (badges, bars), not for 13px text: on the
 * light surface it measures 3.0:1. One step darker clears AA (4.6:1) and
 * stays readable in dark mode too (5.6:1).
 */
const SUCCESS_TEXT = 'oklch(from var(--success) calc(l - 0.1) c h)';

export default function BatteryInfo({ battery, limit }: Props) {
  const [showInfo, setShowInfo] = useState(false);

  if (!battery) {
    return (
      <div className="card">
        <h2 className="card-title">{t('battery.title')}</h2>
        <div className="skeleton" style={{ height: 120 }} />
      </div>
    );
  }

  const state = batteryState(battery.is_charging, battery.is_plugged);
  const designedMah = mwhToMah(battery.designed_capacity_mwh, battery.voltage_mv);
  const fullMah = mwhToMah(battery.full_capacity_mwh, battery.voltage_mv);

  return (
    <div className="card">
      {/* Card header with info button */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          marginBottom: 16,
        }}
      >
        <h2 className="card-title" style={{ margin: 0 }}>
          {t('battery.title')}
        </h2>
        <button
          onClick={() => setShowInfo(true)}
          title={t('battery.infoModal.title')}
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

      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          marginBottom: 6,
        }}
      >
        <span className={`badge ${batteryStateBadge(state)}`}>
          {t(`battery.${state}` as Parameters<typeof t>[0])}
        </span>
        <span style={{ fontWeight: 700, fontSize: 18, lineHeight: 1 }}>{battery.level}%</span>
      </div>
      <div className="progress-bar" style={{ marginBottom: 8 }}>
        <div
          className={`progress-fill ${batteryColor(battery.level)}`}
          style={{ width: `${battery.level}%` }}
        />
      </div>
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          marginBottom: 16,
        }}
        title={t('battery.healthHint')}
      >
        <span className={`badge ${healthColor(battery.health_percent)}`}>
          {t('battery.health')}
        </span>
        <span style={{ fontSize: 13, color: 'var(--text-muted)', fontWeight: 500 }}>
          {battery.health_percent.toFixed(0)}% ({healthWord(battery.health_percent)})
        </span>
      </div>

      {/* Power source is shown explicitly: "plugged in but not charging"
          (charge threshold reached) is a real state and must not read as
          "Discharging" nor imply that current is flowing. */}
      <div className="stat-row">
        <span className="stat-label">{t('battery.powerSource')}</span>
        <span
          className="stat-value"
          style={{ color: battery.is_plugged ? SUCCESS_TEXT : undefined }}
        >
          {battery.is_plugged ? t('battery.acConnected') : t('battery.acOnBattery')}
        </span>
      </div>
      <div className="stat-row">
        <span className="stat-label">{t('battery.manufacturer')}</span>
        <span className="stat-value">{battery.manufacturer}</span>
      </div>
      <div className="stat-row">
        <span className="stat-label">{t('battery.designedCapacity')}</span>
        <span className="stat-value">
          {mwhToWh(battery.designed_capacity_mwh)} {t('battery.wh')}
          {designedMah && (
            <span style={{ color: 'var(--text-muted)', fontSize: 11, marginLeft: 5 }}>
              (≈{designedMah} {t('battery.mah')})
            </span>
          )}
        </span>
      </div>
      <div className="stat-row">
        <span className="stat-label">{t('battery.fullCapacity')}</span>
        <span className="stat-value">
          {mwhToWh(battery.full_capacity_mwh)} {t('battery.wh')}
          {fullMah && (
            <span style={{ color: 'var(--text-muted)', fontSize: 11, marginLeft: 5 }}>
              (≈{fullMah} {t('battery.mah')})
            </span>
          )}
        </span>
      </div>
      {/* Cycle count intentionally NOT shown: the COSMX BX70 firmware never
          reports it (WMI CycleCount and powercfg both return 0), so displaying
          it would present a false "0 cycles" as real data. */}
      {battery.voltage_mv > 0 && (
        <div className="stat-row">
          <span className="stat-label">{t('battery.voltage')}</span>
          <span className="stat-value" style={{ fontFamily: 'var(--font-mono)' }}>
            {(battery.voltage_mv / 1000).toFixed(2)} {t('battery.voltageUnit')}
          </span>
        </div>
      )}
      {/* S48-002: slow-charger hint next to the charging voltage/power block.
          While actively charging, an adapter under 45 W stretches a full
          charge into many hours — warn the user inline (matching the OSD). */}
      {battery.is_charging &&
        battery.ac_input_power_mw != null &&
        battery.ac_input_power_mw < 45_000 && (
          <div
            className="badge warning"
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 6,
              margin: '0 0 12px',
              width: '100%',
              justifyContent: 'center',
            }}
            title={t('battery.slowChargerHint')}
          >
            ⚠ {t('battery.slowCharger')}
          </div>
        )}
      {battery.temperature_celsius != null && (
        <div className="stat-row">
          <span className="stat-label">{t('battery.temperature')}</span>
          <span className="stat-value">
            {battery.temperature_celsius.toFixed(1)} {t('battery.celsius')}
          </span>
        </div>
      )}
      {battery.ac_input_power_mw != null && battery.is_plugged && (
        <div className="stat-row">
          <span className="stat-label">{t('battery.acInputPower')}</span>
          <span
            className="stat-value"
            style={{
              fontFamily: 'var(--font-mono)',
              color: SUCCESS_TEXT,
              fontWeight: 600,
            }}
          >
            ⚡ {(battery.ac_input_power_mw / 1000).toFixed(0)} W
          </span>
        </div>
      )}
      {battery.charge_rate_mw !== 0 && (
        <div className="stat-row">
          <span className="stat-label">
            {battery.charge_rate_mw > 0 ? t('battery.chargeRate') : t('battery.dischargeRate')}
          </span>
          <span
            className="stat-value"
            style={{
              color: battery.charge_rate_mw > 0 ? SUCCESS_TEXT : undefined,
              fontWeight: battery.charge_rate_mw > 0 ? 600 : undefined,
              fontFamily: 'var(--font-mono)',
            }}
          >
            {battery.charge_rate_mw > 0 && '⚡ '}
            {(Math.abs(battery.charge_rate_mw) / 1000).toFixed(1)} W
          </span>
        </div>
      )}
      {battery.is_charging && battery.time_to_full_minutes != null && (
        <div className="stat-row">
          <span className="stat-label">
            {limit ? t('battery.timeToFullAt', { limit }) : t('battery.timeToFull')}
          </span>
          <span className="stat-value">{formatTime(battery.time_to_full_minutes)}</span>
        </div>
      )}
      {!battery.is_charging && !battery.is_plugged && battery.time_remaining_minutes != null && (
        <div className="stat-row">
          <span className="stat-label">{t('battery.timeRemaining')}</span>
          <span className="stat-value">{formatTime(battery.time_remaining_minutes)}</span>
        </div>
      )}

      {/* Info modal */}
      <InfoModal
        open={showInfo}
        onClose={() => setShowInfo(false)}
        title={t('battery.infoModal.title')}
      >
        <InfoRow label={t('battery.infoModal.levelLabel')}>
          {t('battery.infoModal.levelDesc')}
        </InfoRow>
        <InfoRow label={t('battery.infoModal.healthLabel')}>
          {t('battery.infoModal.healthDesc')}
        </InfoRow>
        <InfoSection title={t('battery.infoModal.capacitySection')}>
          <InfoRow label={t('battery.infoModal.capacityLabel')}>
            {t('battery.infoModal.capacityDesc')}
          </InfoRow>
        </InfoSection>
        <InfoSection title={t('battery.infoModal.powerSection')}>
          <InfoRow label={t('battery.infoModal.chargeRateLabel')}>
            {t('battery.infoModal.chargeRateDesc')}
          </InfoRow>
          <InfoRow label={t('battery.infoModal.timeLabel')}>
            {t('battery.infoModal.timeDesc')}
          </InfoRow>
        </InfoSection>
      </InfoModal>
    </div>
  );
}
