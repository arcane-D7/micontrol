import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import StartupManager from '../../components/StartupManager';

export default function StartupTab() {
  return (
    <>
      <PageHeader title={t('startup.title')} subtitle={t('startup.subtitle')} />
      <StartupManager />
    </>
  );
}
