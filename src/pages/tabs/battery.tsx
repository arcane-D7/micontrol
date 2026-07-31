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

      <BatteryInfoCard battery={hw.battery} />
      <ChargingProtection
        threshold={hw.chargingThreshold}
        onThresholdChange={hw.setChargingThreshold}
      />
    </>
  );
}
export default memo(BatteryTab);
