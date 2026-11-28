import { PageHeader } from './PageHeader';
import EcrDebugPanel from '../../components/EcrDebugPanel';
import { t } from '../../hooks/useI18n';

export default function EcrDebugTab() {
  return (
    <>
      <PageHeader title={t('ecrDebug.title')} />
      <EcrDebugPanel />
    </>
  );
}
