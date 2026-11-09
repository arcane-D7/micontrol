/**
 * Tab Overview — "um resumo de toda a aplicação".
 *
 * Antes: cartão de sistema + cartão de bateria + selector de desempenho. Ficava
 * de fora tudo o que identifica a máquina (modelo, CPU, GPU, NPU, RAM, disco,
 * ecrã) e o estado que as outras 20 tabs conhecem.
 *
 * Agora: cinco blocos de leitura + atalhos.
 *   1. Hardware     — identidade, CPU, GPU, NPU (com o TOPS honestamente em `—`)
 *   2. Temperatura  — CPU/GPU/SSD/ambiente, com a fonte à vista
 *   3. Memória e armazenamento — módulos, velocidade, NVMe, espaço, saúde
 *   4. Bateria      — nível, fase, saúde, desgaste, ciclos, adaptador, tensão
 *   5. Estado do sistema + Agora — Windows/uptime/perfil/OS Turbo, ecrã/rede/IoT
 *   + "Ir para…" — cada bloco abre a tab onde se altera
 *
 * Dados: `useHardware` (vivo, polling existente) + `useOverview` (especificações
 * estáticas, lidas uma vez). Nada aqui abre polling novo pesado.
 *
 * O `AiAdvisor` fica **fora** do scope `.mc-hairline`: é um cartão do sistema
 * antigo e herdar tokens hairline mudaria a sua cara.
 */

import { memo } from 'react';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import { useBatteryCare } from '../../hooks/useBatteryCare';
import { useOverview } from '../../hooks/useOverview';
import { OverviewBattery } from '../../components/overview/OverviewBattery';
import {
  OverviewHardware,
  OverviewMemoryStorage,
} from '../../components/overview/OverviewHardware';
import { OverviewJump } from '../../components/overview/OverviewJump';
import { OverviewNow } from '../../components/overview/OverviewNow';
import { OverviewSystem } from '../../components/overview/OverviewSystem';
import { OverviewThermal } from '../../components/overview/OverviewThermal';
import { Icon } from '../../components/ui/icons';
import AiAdvisor from '../../components/AiAdvisor';

import type { Hardware, AiSettings } from './shared';

interface Props {
  hw: Hardware;
  ai: AiSettings;
  onOpenSettings: () => void;
  /** Muda de tab (atalhos "Ir para…"). */
  onNavigate: (tab: string) => void;
}

function OverviewTab({ hw, ai, onOpenSettings, onNavigate }: Props) {
  const {
    snapshot,
    osTurbo,
    wifi,
    iot,
    services,
    loading: overviewLoading,
    refresh,
  } = useOverview();
  const { care } = useBatteryCare();

  const failed = !overviewLoading && snapshot == null;

  return (
    <>
      <PageHeader title={t('overview.title')} subtitle={t('overview.subtitle')} />

      <div className="mc-hairline mc-overview">
        {failed && (
          <p className="mc-note mc-note--warning" role="status">
            <Icon name="alert-triangle" size={14} />
            {t('overview.specsFailed')}
            <span className="mc-note__spacer" />
            <button type="button" className="mc-linkbtn" onClick={() => void refresh()}>
              {t('overview.retry')}
            </button>
          </p>
        )}

        <div className="grid-2">
          <OverviewHardware snapshot={snapshot} loading={overviewLoading} />
          <div className="mc-stack">
            <OverviewThermal fan={hw.fan} loading={hw.loading} />
            <OverviewBattery
              battery={hw.battery}
              threshold={hw.chargingThreshold}
              care={care}
              loading={hw.loading}
            />
          </div>
        </div>

        <div className="grid-2">
          <OverviewMemoryStorage snapshot={snapshot} loading={overviewLoading} />
          <OverviewSystem
            snapshot={snapshot}
            perfMode={hw.performanceMode}
            osTurbo={osTurbo}
            services={services}
            loading={overviewLoading}
          />
        </div>

        <OverviewNow
          snapshot={snapshot}
          display={hw.display}
          wifi={wifi}
          iot={iot}
          loading={overviewLoading}
        />

        <OverviewJump onNavigate={onNavigate} />
      </div>

      <AiAdvisor hw={hw} ai={ai} onOpenSettings={onOpenSettings} />
    </>
  );
}

export default memo(OverviewTab);
