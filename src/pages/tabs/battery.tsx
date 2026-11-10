/**
 * Tab Bateria — POC hairline (spec §5.2/§5.3, 9 blocos).
 *
 * 1. Título + subtítulo            → PageHeader
 * 2-5. Herói, barra + marca do limite, fase de carga, fonte/potência/tempo
 *                                  → BatteryInfo
 * 6-8. Proteção (switch), limite (segmented), linha derivada
 *                                  → ChargingProtection
 * 9. Detalhes (5 linhas)           → BatteryDetails
 *
 * A tab é o único sítio que sabe o limite **e** o estado da proteção, e é isso
 * que permite a pergunta que a app hoje não responde: "porque é que não chega
 * aos 100%?".
 */

import { memo, useCallback, useEffect, useRef, useState } from 'react';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import BatteryInfoCard, { BatteryDetails } from '../../components/BatteryInfo';
import ChargingProtection from '../../components/ChargingProtection';
import { useBatteryCare } from '../../hooks/useBatteryCare';
import type { Hardware } from './shared';

interface Props {
  hw: Hardware;
}

/** Última leitura da bateria que chegou a este ecrã (estado 2: "há N s"). */
function useBatteryFreshness(hasBattery: boolean) {
  const lastReadRef = useRef<number | null>(null);
  const [updatedAgoSec, setUpdatedAgoSec] = useState<number | null>(null);
  const [lastReadClock, setLastReadClock] = useState<string | null>(null);

  // Reinicia em cada leitura nova (o poll lento é de 15 s).
  useEffect(() => {
    if (!hasBattery) return;
    lastReadRef.current = Date.now();
    setUpdatedAgoSec(0);
    setLastReadClock(
      new Date().toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' }),
    );
  }, [hasBattery]);

  // Um tick por segundo, e só para o contador — não re-renderiza dados.
  useEffect(() => {
    const id = setInterval(() => {
      if (lastReadRef.current == null) return;
      setUpdatedAgoSec(Math.round((Date.now() - lastReadRef.current) / 1000));
    }, 1000);
    return () => clearInterval(id);
  }, []);

  return { updatedAgoSec, lastReadClock };
}

function BatteryTab({ hw }: Props) {
  const { care, limitState, pending: carePending, setCare, reload: reloadCare } = useBatteryCare();
  const { updatedAgoSec, lastReadClock } = useBatteryFreshness(hw.battery != null);
  const { setChargingThreshold, refreshBattery } = hw;

  // Estado 3: há valores mas o último poll falhou → mantêm-se e diz-se desde quando.
  const staleSince = hw.battery != null && hw.error != null ? lastReadClock : null;
  // Vazio: nunca houve leitura e a carga inicial já terminou com erro.
  const unavailable = hw.battery == null && !hw.loading && hw.error != null;

  /** Um gesto: liga a proteção e aplica o limite; se o limite falhar, desliga. */
  const enableCare = useCallback(
    async (limit: number) => {
      await setCare(true);
      try {
        await setChargingThreshold(limit);
        await reloadCare();
      } catch (e) {
        await setCare(false).catch(() => {});
        throw e;
      }
    },
    [setCare, setChargingThreshold, reloadCare],
  );

  /** Desligar a proteção implica "sem limite" (100) — é o que o EC faz. */
  const disableCare = useCallback(async () => {
    await setCare(false);
    await setChargingThreshold(100);
    await reloadCare();
  }, [setCare, setChargingThreshold, reloadCare]);

  /** Aplicar um nível com a proteção já ligada — relê a verdade do firmware. */
  const applyThreshold = useCallback(
    async (limit: number) => {
      await setChargingThreshold(limit);
      await reloadCare();
    },
    [setChargingThreshold, reloadCare],
  );

  const retry = useCallback(() => {
    void refreshBattery();
  }, [refreshBattery]);

  return (
    <div className="mc-hairline">
      <PageHeader title={t('battery.title')} subtitle={t('battery.subtitle')} />

      <div className="mc-stack">
        <BatteryInfoCard
          battery={hw.battery}
          threshold={hw.chargingThreshold}
          care={care}
          limitApplied={limitState?.applied ?? null}
          updatedAgoSec={updatedAgoSec}
          staleSince={staleSince}
          unavailable={unavailable}
          onRetry={retry}
        />

        {hw.battery && (
          <ChargingProtection
            threshold={hw.chargingThreshold}
            level={hw.battery.level}
            care={care}
            carePending={carePending}
            onThresholdChange={applyThreshold}
            onEnableCare={enableCare}
            onDisableCare={disableCare}
            limitState={limitState}
          />
        )}

        {hw.battery && <BatteryDetails battery={hw.battery} />}
      </div>
    </div>
  );
}
export default memo(BatteryTab);
