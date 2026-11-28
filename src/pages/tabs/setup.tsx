import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import HardwareDiscovery from '../../components/HardwareDiscovery';
import IotModulePanel from '../../components/IotModulePanel';
import type { Hardware } from './shared';

interface Props {
  hw: Hardware;
}

export default function SetupTab({ hw }: Props) {
  return (
    <>
      <PageHeader title={t('discovery.title')} subtitle={t('discovery.subtitle')} />
      <HardwareDiscovery
        profile={hw.hardwareProfile}
        loading={hw.loadingDiscovery}
        onRescan={hw.runHardwareDiscovery}
        onInstallDriver={hw.installDriver}
      />
      <IotModulePanel hw={hw} />
    </>
  );
}
