import { memo } from 'react';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import UpdateManager from '../../components/UpdateManager';
import type { Hardware } from './shared';
import type { AppUpdateState, AppUpdateInfo } from '../../hooks/useAutoUpdate';

interface Props {
  hw: Hardware;
  appUpdateState: AppUpdateState;
  appUpdateInfo: AppUpdateInfo | null;
  appUpdateProgress: number;
  appUpdateError: string;
  onCheckAppUpdate: () => void;
  onInstallAppUpdate: () => void;
  onDismissAppUpdate: () => void;
}

function UpdatesTab({
  hw,
  appUpdateState,
  appUpdateInfo,
  appUpdateProgress,
  appUpdateError,
  onCheckAppUpdate,
  onInstallAppUpdate,
  onDismissAppUpdate,
}: Props) {
  return (
    <>
      <PageHeader title={t('updates.title')} subtitle={t('updates.subtitle')} />
      <UpdateManager
        updateStatus={hw.updateStatus}
        loadingUpdate={hw.loadingUpdate}
        onRefreshUpdate={hw.refreshUpdateStatus}
        appUpdateState={appUpdateState}
        appUpdateInfo={appUpdateInfo}
        appUpdateProgress={appUpdateProgress}
        appUpdateError={appUpdateError}
        onCheckAppUpdate={onCheckAppUpdate}
        onInstallAppUpdate={onInstallAppUpdate}
        onDismissAppUpdate={onDismissAppUpdate}
      />
    </>
  );
}

export default memo(UpdatesTab);
