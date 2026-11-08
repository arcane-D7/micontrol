import { memo } from 'react';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import SystemInfoCard from '../../components/SystemInfoCard';
import BatteryInfoCard from '../../components/BatteryInfo';
import PerformanceModeSelector from '../../components/PerformanceModeSelector';
import AiAdvisor from '../../components/AiAdvisor';
import type { Hardware, AiSettings } from './shared';

interface Props {
  hw: Hardware;
  ai: AiSettings;
  onOpenSettings: () => void;
}

function OverviewTab({ hw, ai, onOpenSettings }: Props) {
  return (
    <>
      <PageHeader title={t('overview.title')} />
      <div className="grid-2">
        <SystemInfoCard info={hw.systemInfo} getProcessList={hw.getProcessList} />
        {/* O limite entra aqui para a linha de fase não mentir: sem ele, uma
            bateria em pausa aos 96% com limite de 80% diria "carrega até 100%".
            O wrapper `mc-hairline` é o scope dos tokens do POC — fora da tab
            Bateria o cartão herdaria tokens que ainda não existem em :root. */}
        <div className="mc-hairline">
          <BatteryInfoCard battery={hw.battery} threshold={hw.chargingThreshold} />
        </div>
      </div>
      <div className="card">
        <h2 className="card-title">{t('nav.performance')}</h2>
        <PerformanceModeSelector
          current={hw.performanceMode}
          onChange={hw.setPerformanceMode}
          disabled={hw.loading}
          applying={hw.perfApplying}
        />
      </div>
      <AiAdvisor hw={hw} ai={ai} onOpenSettings={onOpenSettings} />
    </>
  );
}

export default memo(OverviewTab);
