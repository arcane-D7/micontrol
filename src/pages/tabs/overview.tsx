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
            bateria em pausa aos 96% com limite de 80% diria "carrega até 100%". */}
        <BatteryInfoCard battery={hw.battery} threshold={hw.chargingThreshold} />
      </div>
      <div className="card">
        <div className="card-title">{t('nav.performance')}</div>
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
