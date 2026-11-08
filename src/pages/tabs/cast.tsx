import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import ScreenCast from '../../components/ScreenCast';

export default function CastTab() {
  return (
    <>
      <PageHeader title={t('cast.title')} />
      <ScreenCast />
    </>
  );
}
