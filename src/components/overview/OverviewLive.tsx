/**
 * Consumo VIVO — o primeiro bloco do Overview.
 *
 * Quatro tiles: CPU %, GPU %, RAM usada/total e bateria (nível + estado), com
 * a temperatura de CPU/GPU como sub-linha dos tiles de processador.
 *
 * Dados: o sampler que já existe — `get_system_info` (PDH, perna rápida de
 * 2 s do `useHardware`), `get_fan_info` (ESIF, 2 s) e `get_battery_info`
 * (perna lenta de 15 s). Este bloco NÃO abre um segundo sampler no backend:
 * duas leituras concorrentes de `get_fan_info` empilham chamadas abandonadas
 * (STAB-ROUND-1: 2 221 chamadas em 74 min). O que ele mantém é o *refresco*
 * da perna de sistema — a mesma `fastPollSystem` do hook, exposta como
 * `refreshSystem` — agendado por uma cadeia de `setTimeout` com a perna
 * corrida através de `runBounded` (`lib/boundedPoll.ts`), nunca um
 * `setInterval` solto. Como a perna do hook tem guarda de in-flight, um
 * gatilho extra colapsa na mesma leitura, não numa segunda.
 *
 * Honestidade: valor ausente é `—` (`./format.ts`). O commit charge não existe
 * no `SystemInfo` do backend e não foi inventado; a casa não tem primitivo de
 * sparkline e nenhum foi inventado.
 */

import { memo, useEffect } from 'react';
import { runBounded } from '../../lib/boundedPoll';
import { t } from '../../hooks/useI18n';
import type { BatteryInfo, FanInfo, SystemInfo } from '../../types/hardware';
import { Card } from '../ui/card';
import { Icon } from '../ui/icons';
import { EMPTY, fmtNumber } from './format';

/** Mesma cadência da perna rápida do hook (1-2 s). */
const REFRESH_MS = 2000;

interface Props {
  systemInfo: SystemInfo | null;
  fan: FanInfo | null;
  battery: BatteryInfo | null;
  loading?: boolean;
  /**
   * A perna de sistema do `useHardware` (`fastPollSystem`). Opcional para
   * testes e para consumidor sem hook — sem ela o bloco não abre refresco.
   */
  refreshSystem?: () => Promise<void>;
}

function batteryStateLabel(battery: BatteryInfo | null): string | null {
  if (!battery) return null;
  if (battery.is_charging) return t('overview.live.charging');
  if (battery.is_plugged) return t('overview.live.ac');
  return t('overview.live.discharging');
}

function OverviewLiveBase({ systemInfo, fan, battery, loading, refreshSystem }: Props) {
  // Refresco em cadeia: perna → próxima marcação. Sem `setInterval`, sem
  // marcador a correr depois do unmount (o teste fixa o zero de fugas).
  useEffect(() => {
    if (typeof refreshSystem !== 'function') return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const schedule = () => {
      if (cancelled) return;
      timer = setTimeout(tick, REFRESH_MS);
    };
    const tick = () => {
      // O hook salta janelas escondidas (tray); o bloco espelha a regra.
      if (typeof document !== 'undefined' && document.visibilityState !== 'visible') {
        schedule();
        return;
      }
      void runBounded([() => Promise.resolve(refreshSystem())], { limit: 1 }).finally(schedule);
    };
    tick();
    return () => {
      cancelled = true;
      if (timer !== undefined) clearTimeout(timer);
    };
  }, [refreshSystem]);

  const cpuUsage = systemInfo ? systemInfo.cpu_usage : null;
  const gpuUsage = systemInfo ? systemInfo.gpu_usage : null;
  const cpuTemp = fan ? fan.cpu_temp_celsius : null;
  const gpuTemp = fan ? fan.gpu_temp_celsius : null;
  const ramValue = systemInfo
    ? `${fmtNumber(systemInfo.ram_used_gb, 1)} / ${fmtNumber(systemInfo.ram_total_gb, 0)}`
    : EMPTY;
  const batteryLevel = battery ? battery.level : null;
  const batteryState = batteryStateLabel(battery);

  return (
    <Card
      title={t('overview.live.title')}
      icon={<Icon name="bolt" size={16} />}
      loading={loading}
      aria-label={t('overview.live.title')}
    >
      <div className="grid-4" data-testid="live-usage">
        <div className="mc-tile" data-testid="live-cpu">
          <span className="mc-tile__label">{t('overview.live.cpu')}</span>
          <span className="mc-tile__value">
            {fmtNumber(cpuUsage, 0)}
            {cpuUsage != null && <span className="mc-tile__unit">%</span>}
          </span>
          <span className="mc-tile__sub">
            {cpuTemp != null ? fmtNumber(cpuTemp, 1) : EMPTY}
            {cpuTemp != null && <span className="mc-tile__unit">°C</span>}
          </span>
        </div>

        <div className="mc-tile" data-testid="live-gpu">
          <span className="mc-tile__label">{t('overview.live.gpu')}</span>
          <span className="mc-tile__value">
            {fmtNumber(gpuUsage, 0)}
            {gpuUsage != null && <span className="mc-tile__unit">%</span>}
          </span>
          <span className="mc-tile__sub">
            {gpuTemp != null ? fmtNumber(gpuTemp, 1) : EMPTY}
            {gpuTemp != null && <span className="mc-tile__unit">°C</span>}
          </span>
        </div>

        <div className="mc-tile" data-testid="live-ram">
          <span className="mc-tile__label">{t('overview.live.ram')}</span>
          <span className="mc-tile__value mc-tile__value--wide">
            {ramValue}
            {systemInfo && <span className="mc-tile__unit">GB</span>}
          </span>
        </div>

        <div className="mc-tile" data-testid="live-battery">
          <span className="mc-tile__label">{t('overview.live.battery')}</span>
          <span className="mc-tile__value">
            {fmtNumber(batteryLevel, 0)}
            {batteryLevel != null && <span className="mc-tile__unit">%</span>}
          </span>
          <span className="mc-tile__sub">{batteryState ?? EMPTY}</span>
        </div>
      </div>
    </Card>
  );
}

export const OverviewLive = memo(OverviewLiveBase);
export default OverviewLive;
