import { t } from '../../hooks/useI18n';
import { PageHeader } from './PageHeader';
import WiFiManager from '../../components/WiFiManager';

export default function WiFiTab() {
  return (
    <>
      <PageHeader title={t('wifi.title')} />
      <WiFiManager />
    </>
  );
}
