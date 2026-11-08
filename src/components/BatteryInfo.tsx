/**
 * Bateria — bloco 2..5 (herói, barra com marca do limite, fase de carga,
 * fonte/potência/tempo) e bloco 9 (detalhes).
 *
 * Os três estados de leitura da spec §5.5 vivem aqui:
 *   1. primeira leitura  → skeleton **com a forma** do conteúdo (CLS 0);
 *   2. dados presentes   → conteúdo completo, "Atualizado há N s" (nada pisca);
 *   3. leitura falhada   → valores mantidos + hairline de topo `--warning`
 *                          + "Sem leitura desde as HH:MM" + retry.
 *   + vazio              → "Este equipamento não reporta bateria".
 *
 * Zero `style={{}}` excepto larguras calculadas (barra, marca do limite), que
 * não têm tradução em classe.
 *
 * Merge f4/battery-poc × f4/tabs-cognitive: a estrutura é a do POC aprovado, e
 * o texto mantém o glossário do tabs-cognitive — "Faltam para os 80%" diz a que
 * nível a estimativa se refere (um tempo sem alvo não responde a nada).
 */

import { useState } from 'react';
import { t, type StringKey } from '../hooks/useI18n';
import { chargePhase, type ChargePhase } from '../lib/batteryState';
import {
  formatCelsius,
  formatDecimal,
  formatDuration,
  formatPercent,
  formatVoltage,
  formatWatts,
  formatWh,
  NO_DATA_LABEL,
} from '../lib/format';
import type { BatteryInfo as BatteryData } from '../types/hardware';
import InfoModal, { InfoRow, InfoSection } from './InfoModal';
import { Badge } from './ui/badge';
import { Card } from './ui/card';
import { Icon } from './ui/icons';
import { IconButton } from './ui/icon-button';
import { BatterySkeleton } from './ui/skeleton';
import { StatRow } from './ui/stat-row';

export type PhaseTone = 'neutral' | 'warn' | 'success' | 'muted';

export interface PhaseMessage {
  key: StringKey;
  vars?: Record<string, string | number>;
  tone: PhaseTone;
}

/**
 * Copy + tom por fase (spec §5.6.2). Exportada para o teste poder fixar a
 * tradução de cada transição sem montar o componente.
 */
export function chargePhaseMessage(
  phase: ChargePhase,
  opts: { care: boolean | null; time: string },
): PhaseMessage {
  const { care, time } = opts;
  switch (phase.kind) {
    case 'discharging':
      return time
        ? { key: 'battery.phase.discharging', vars: { time }, tone: 'neutral' }
        : { key: 'battery.phase.dischargingNoTime', tone: 'neutral' };
    case 'holding':
      return {
        key: 'battery.phase.holding',
        vars: { level: phase.level, limit: phase.limit },
        tone: 'warn',
      };
    case 'resuming':
      return { key: 'battery.phase.resuming', vars: { limit: phase.limit }, tone: 'neutral' };
    case 'full':
      return { key: 'battery.phase.full', tone: 'success' };
    case 'charging':
      if (phase.target >= 100) {
        return care === false
          ? { key: 'battery.phase.careOff', tone: 'muted' }
          : { key: 'battery.phase.noLimit', tone: 'muted' };
      }
      return { key: 'battery.phase.charging', vars: { target: phase.target }, tone: 'neutral' };
    default:
      return { key: 'battery.phase.dischargingNoTime', tone: 'neutral' };
  }
}

/** Banda de saúde explícita — um número sem escala não significa nada. */
export function healthBand(pct: number): {
  key: StringKey;
  tone: 'success' | 'warning' | 'danger';
} {
  if (pct >= 80) return { key: 'battery.healthBand.good', tone: 'success' };
  if (pct >= 60) return { key: 'battery.healthBand.fair', tone: 'warning' };
  return { key: 'battery.healthBand.poor', tone: 'danger' };
}

export interface BatteryInfoProps {
  battery: BatteryData | null;
  /** Limite configurado (0–100); 100 = sem limite. */
  threshold?: number;
  /** Alias antigo de `threshold` — aceite para os chamadores da tab cognitiva. */
  limit?: number;
  /** null = ainda não se sabe (a ler `get_battery_care`). */
  care?: boolean | null;
  /** Segundos desde a última leitura que chegou a este ecrã. */
  updatedAgoSec?: number | null;
  /** "14:32" quando a última leitura falhou; null quando está tudo bem. */
  staleSince?: string | null;
  /** `battery === null` **e** erro: o equipamento não reporta bateria. */
  unavailable?: boolean;
  onRetry?: () => void;
}

function levelTone(level: number): string {
  if (level > 30) return '';
  return level > 15 ? ' mc-bar__fill--low' : ' mc-bar__fill--critical';
}

export default function BatteryInfo({
  battery,
  threshold,
  limit: limitAlias,
  care = null,
  updatedAgoSec = null,
  staleSince = null,
  unavailable = false,
  onRetry = () => {},
}: BatteryInfoProps) {
  const [showInfo, setShowInfo] = useState(false);

  if (!battery) {
    if (unavailable) {
      return (
        <Card title={t('battery.status')}>
          <div className="mc-empty">
            <Icon name="battery" size={24} className="mc-empty__icon" />
            <div>
              <div className="mc-empty__title">{t('battery.unavailable')}</div>
              <div className="mc-empty__desc">{t('battery.unavailableHint')}</div>
            </div>
          </div>
        </Card>
      );
    }
    return (
      <Card title={t('battery.status')}>
        <BatterySkeleton label={t('battery.reading')} />
      </Card>
    );
  }

  /** `threshold` é o nome canónico; `limit` sobrevive como alias de compat. */
  const configured = threshold ?? limitAlias ?? 100;
  const limit = care === false ? 100 : configured;

  const phase = chargePhase({
    level: battery.level,
    isCharging: battery.is_charging,
    isPlugged: battery.is_plugged,
    limit,
    minutesRemaining: battery.time_remaining_minutes,
  });
  const message = chargePhaseMessage(phase, {
    care,
    time: formatDuration(battery.time_remaining_minutes),
  });
  const toneClass =
    message.tone === 'warn'
      ? ' mc-hero__phase--warn'
      : message.tone === 'success'
        ? ' mc-hero__phase--success'
        : message.tone === 'muted'
          ? ' mc-hero__phase--muted'
          : '';

  const power = battery.is_plugged
    ? `${t('battery.plugConnected')}${
        battery.ac_input_power_mw ? ` · ${formatWatts(battery.ac_input_power_mw)}` : ''
      }`
    : t('battery.onBattery');

  const eta =
    phase.kind === 'discharging'
      ? formatDuration(phase.minutes)
      : phase.kind === 'charging' && battery.time_to_full_minutes != null
        ? formatDuration(battery.time_to_full_minutes)
        : '';

  // Glossário §4.4: um tempo sem alvo não responde a "quando é que chega aos
  // 80%?". O rótulo diz a que nível a estimativa se refere.
  const etaTarget =
    phase.kind === 'charging' && eta !== '' && limit < 100
      ? t('battery.timeToFullAt', { limit })
      : '';

  // A taxa de descarga não se lê em mais nenhum sítio do cartão.
  const dischargeRate =
    !battery.is_charging && battery.charge_rate_mw < 0
      ? formatWatts(Math.abs(battery.charge_rate_mw))
      : '';

  return (
    <Card
      title={t('battery.status')}
      stale={Boolean(staleSince)}
      action={
        <>
          {updatedAgoSec != null && (
            <span className="mc-card__meta">
              {updatedAgoSec <= 1
                ? t('battery.updatedNow')
                : t('battery.updatedAgo', { s: updatedAgoSec })}
            </span>
          )}
          <IconButton
            icon="info"
            label={t('battery.infoModal.title')}
            onClick={() => setShowInfo(true)}
          />
        </>
      }
    >
      <div className="mc-hero">
        <Icon name="battery" size={24} level={battery.level} className="mc-hero__icon" />
        <div className="mc-hero__main">
          <div className="mc-hero__row">
            <span className="mc-hero__value">{formatPercent(battery.level)}</span>
            {phase.kind === 'holding' && (
              <Badge tone="warning" dot>
                {t('battery.phase.holdingBadge')}
              </Badge>
            )}
            {phase.kind === 'full' && (
              <Badge tone="success" dot>
                {t('battery.phase.fullBadge')}
              </Badge>
            )}
          </div>

          <div
            className="mc-bar"
            role="img"
            aria-label={t('battery.levelBarLabel', { level: battery.level })}
          >
            <div
              className={`mc-bar__fill${levelTone(battery.level)}`}
              style={{ width: `${Math.min(100, Math.max(0, battery.level))}%` }}
            />
            {limit < 100 && (
              <span
                className="mc-bar__mark"
                style={{ left: `calc(${limit}% - 1px)` }}
                title={t('battery.limitMark', { limit })}
              />
            )}
          </div>

          <div className={`mc-hero__phase${toneClass}`} data-testid="charge-phase">
            {t(message.key, message.vars)}
          </div>

          <div className="mc-hero__meta">
            <span className="mc-hero__meta-item">
              <Icon name="plug" size={16} />
              {power}
            </span>
            {eta && (
              <span className="mc-hero__meta-item">
                <Icon name="bolt" size={16} />
                {etaTarget ? `${etaTarget}: ${eta}` : eta}
              </span>
            )}
            {dischargeRate && (
              <span className="mc-hero__meta-item">
                <Icon name="bolt" size={16} />
                {t('battery.dischargeRate')} {dischargeRate}
              </span>
            )}
          </div>
        </div>
      </div>

      {battery.is_charging &&
        battery.ac_input_power_mw != null &&
        battery.ac_input_power_mw < 45_000 && (
          <div className="mc-note mc-note--warning" title={t('battery.slowChargerHint')}>
            {t('battery.slowCharger')} — {formatWatts(battery.ac_input_power_mw)}
          </div>
        )}

      {staleSince && (
        <div className="mc-note mc-note--warning" role="status">
          <Icon name="info" size={16} />
          <span>{t('battery.stale', { time: staleSince })}</span>
          <span className="mc-note__spacer" />
          <IconButton icon="refresh" label={t('battery.retry')} onClick={onRetry} />
        </div>
      )}

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
    </Card>
  );
}

export interface BatteryDetailsProps {
  battery: BatteryData;
}

/** Bloco 9 — as 5 linhas de "a bateria está boa?". */
export function BatteryDetails({ battery }: BatteryDetailsProps) {
  const band = healthBand(battery.health_percent);
  const healthPct = battery.designed_capacity_mwh
    ? Math.round((battery.full_capacity_mwh / battery.designed_capacity_mwh) * 100)
    : null;

  // Estimativa mAh a partir dos mWh e da tensão atual (o backend só dá mWh).
  const designedMah =
    battery.voltage_mv > 0
      ? formatDecimal((battery.designed_capacity_mwh * 1000) / battery.voltage_mv, 0)
      : null;

  return (
    <Card
      title={t('battery.details')}
      action={
        battery.manufacturer ? (
          <span className="mc-card__meta">{battery.manufacturer}</span>
        ) : undefined
      }
    >
      <StatRow
        icon={<Icon name="shield" size={20} />}
        label={t('battery.factoryCapacity')}
        value={formatWh(battery.designed_capacity_mwh)}
        hint={
          designedMah != null ? (
            <span className="mc-stat__hint">
              ≈ {designedMah} {t('battery.mah')}
            </span>
          ) : undefined
        }
        mono
      />
      <StatRow
        icon={<Icon name="battery" size={20} />}
        label={t('battery.maxCapacity')}
        value={formatWh(battery.full_capacity_mwh)}
        hint={
          <>
            <Badge tone={band.tone} dot>
              {t(band.key)}
            </Badge>
            {healthPct != null && (
              <span className="mc-stat__hint">
                {t('battery.healthOfFactory', { pct: healthPct })}
              </span>
            )}
          </>
        }
        mono
      />
      <StatRow
        icon={<Icon name="bolt" size={20} />}
        label={t('battery.voltage')}
        value={formatVoltage(battery.voltage_mv)}
        mono
        dim={battery.voltage_mv <= 0}
      />
      <StatRow
        icon={<Icon name="thermometer" size={20} />}
        label={t('battery.temperature')}
        value={formatCelsius(battery.temperature_celsius)}
        mono
        dim={battery.temperature_celsius == null}
      />
      {battery.is_plugged && (
        <StatRow
          icon={<Icon name="plug" size={20} />}
          label={t('battery.chargerSupplying')}
          value={battery.ac_input_power_mw ? formatWatts(battery.ac_input_power_mw) : NO_DATA_LABEL}
          hint={!battery.ac_input_power_mw ? t('battery.notExposed') : undefined}
          mono
          dim={!battery.ac_input_power_mw}
        />
      )}
    </Card>
  );
}