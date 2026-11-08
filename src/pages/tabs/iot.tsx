import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import IotDeviceCard from '../../components/IotDeviceCard';

export default function IotTab() {
  return (
    <>
      <PageHeader title={t('nav.iot')} />
      <IotDeviceCard />
    </>
  );
}
