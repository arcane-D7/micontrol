/**
 * Bateria — o resumo que a tab Bateria tem em detalhe.
 *
 * O que o Overview mostrava antes era só "carregador ligado". Aqui entram
 * nível, fase de carga, saúde, desgaste, capacidade real vs. de fábrica,
 * ciclos, potência do adaptador, tensão, tempo estimado, limite e modo care.
 *
 * A fase e a banda de saúde vêm de `lib/batteryState` e de `BatteryInfo` — as
 * mesmas funções que a tab Bateria usa, para os dois ecrãs nunca dizerem coisas
 * diferentes sobre o mesmo estado.
 *
 * Honestidade: `cycle_count` só é mostrado quando o firmware o reporta (> 0);
 * este equipamento devolve 0, que significa "não sei", não "zero ciclos".
 */

import { t } from '../../hooks/useI18n';
import { chargePhase } from '../../lib/batteryState';
import { formatDuration } from '../../lib/format';
import type { BatteryInfo } from '../../types/hardware';
import { chargePhaseMessage, healthBand } from '../BatteryInfo';
import { Badge, type BadgeTone } from '../ui/badge';
import { Card } from '../ui/card';
import { Icon } from '../ui/icons';
import { StatGroup, StatRow } from '../ui/stat-row';
import { EMPTY, fmtNumber, fmtUnit } from './format';

interface Props {
  battery: BatteryInfo | null;
  /** Limite de carga configurado; 100 = sem limite. */
  threshold: number;
  /** null enquanto `get_battery_care` não respondeu. */
  care: boolean | null;
  loading: boolean;
}

const PHASE_TONE: Record<string, BadgeTone> = {
  neutral: 'neutral',
  warn: 'warning',
  success: 'success',
  muted: 'neutral',
};

export function OverviewBattery({ battery, threshold, care, loading }: Props) {
  if (!battery) {
    return (
      <Card
        title={t('overview.power.title')}
        icon={<Icon name="battery" size={16} />}
        loading={loading}
      >
        <p className="mc-group__note">
          {loading ? t('overview.power.reading') : t('overview.power.unavailable')}
        </p>
      </Card>
    );
  }

  const phase = chargePhase({
    level: battery.level,
    isCharging: battery.is_charging,
    isPlugged: battery.is_plugged,
    limit: threshold,
    minutesRemaining: battery.time_remaining_minutes,
  });
  const phaseMessage = chargePhaseMessage(phase, {
    care,
    time: formatDuration(battery.time_remaining_minutes),
  });
  const band = healthBand(battery.health_percent);
  const wear = Math.max(0, 100 - battery.health_percent);
  const cycles = battery.cycle_count > 0 ? battery.cycle_count : null;

  return (
    <Card
      title={t('overview.power.title')}
      icon={<Icon name="battery" size={16} />}
      loading={loading}
      meta={
        <Badge tone={PHASE_TONE[phaseMessage.tone] ?? 'neutral'} dot>
          {t(phaseMessage.key, phaseMessage.vars)}
        </Badge>
      }
    >
      <div className="mc-group">
        <p className="mc-lead">
          {fmtNumber(battery.level)}%
          <span className="mc-lead__sub">
            {battery.device_name} · {battery.manufacturer}
          </span>
        </p>
        <StatGroup>
          <StatRow
            label={t('overview.power.health')}
            value={
              <Badge tone={band.tone} dot>
                {t(band.key)}
              </Badge>
            }
            hint={fmtUnit(battery.health_percent, '%', 1)}
          />
          <StatRow
            label={t('overview.power.wear')}
            value={fmtNumber(wear, 1)}
            unit="%"
            hint={t('overview.power.wearHint')}
          />
          <StatRow
            label={t('overview.power.capacity')}
            value={`${fmtNumber(battery.full_capacity_mwh / 1000, 1)} / ${fmtNumber(
              battery.designed_capacity_mwh / 1000,
              1,
            )}`}
            unit="Wh"
            hint={t('overview.power.capacityHint')}
          />
          <StatRow
            label={t('overview.power.cycles')}
            value={cycles != null ? fmtNumber(cycles) : EMPTY}
            empty={cycles == null}
            hint={cycles == null ? t('overview.power.cyclesHint') : undefined}
          />
        </StatGroup>
      </div>

      <div className="mc-group">
        <h3 className="mc-group__title">{t('overview.power.charging')}</h3>
        <StatGroup>
          <StatRow
            label={t('overview.power.adapter')}
            value={
              battery.ac_input_power_mw != null
                ? fmtNumber(battery.ac_input_power_mw / 1000, 0)
                : EMPTY
            }
            unit={battery.ac_input_power_mw != null ? 'W' : undefined}
            empty={battery.ac_input_power_mw == null}
            hint={battery.ac_input_power_mw == null ? t('overview.power.adapterHint') : undefined}
            mono
          />
          <StatRow
            label={t('overview.power.voltage')}
            value={battery.voltage_mv > 0 ? fmtNumber(battery.voltage_mv / 1000, 2) : EMPTY}
            unit={battery.voltage_mv > 0 ? 'V' : undefined}
            empty={battery.voltage_mv <= 0}
            mono
          />
          <StatRow
            label={t('overview.power.timeLeft')}
            value={
              battery.time_remaining_minutes != null
                ? formatDuration(battery.time_remaining_minutes)
                : EMPTY
            }
            empty={battery.time_remaining_minutes == null}
            hint={
              battery.time_remaining_minutes == null ? t('overview.power.timeLeftHint') : undefined
            }
          />
          <StatRow
            label={t('overview.power.limit')}
            value={threshold >= 100 ? t('overview.power.noLimit') : fmtNumber(threshold)}
            unit={threshold >= 100 ? undefined : '%'}
          />
          <StatRow
            label={t('overview.power.care')}
            value={
              care == null ? (
                EMPTY
              ) : care ? (
                <Badge tone="success" dot>
                  {t('overview.power.careOn')}
                </Badge>
              ) : (
                <Badge tone="neutral" dot>
                  {t('overview.power.careOff')}
                </Badge>
              )
            }
            empty={care == null}
          />
        </StatGroup>
      </div>
    </Card>
  );
}

export default OverviewBattery;
