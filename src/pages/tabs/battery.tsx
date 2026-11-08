import { memo } from 'react';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import BatteryInfoCard from '../../components/BatteryInfo';
import ChargingProtection from '../../components/ChargingProtection';
import type { Hardware } from './shared';

interface Props {
  hw: Hardware;
}

function BatteryTab({ hw }: Props) {
  return (
    <>
      <PageHeader title={t('battery.title')} />

      <BatteryInfoCard
        battery={hw.battery}
        limit={hw.chargingThreshold !== 100 ? hw.chargingThreshold : undefined}
      />
      <ChargingProtection
        threshold={hw.chargingThreshold}
        onThresholdChange={hw.setChargingThreshold}
        battery={hw.battery}
      />
    </>
  );
}
export default memo(BatteryTab);
