import { t } from '../hooks/useI18n';
import type { AppUpdateState, AppUpdateInfo } from '../hooks/useAutoUpdate';

interface Props {
  state: AppUpdateState;
  updateInfo: AppUpdateInfo | null;
  progress: number;
  errorMsg: string;
  onCheck: () => void;
  onInstall: () => void;
  onDismiss: () => void;
}

export default function AppUpdateBanner({
  state,
  updateInfo,
  progress,
  errorMsg,
  onCheck,
  onInstall,
  onDismiss,
}: Props) {
  if (state === 'idle' || state === 'checking') {
    return (
      <div className="card" style={{ marginBottom: 16 }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <div>
            <div className="card-title" style={{ margin: 0, marginBottom: 4 }}>
              {t('updates.appVersion')}
            </div>
            <div style={{ fontSize: '0.85rem', color: 'var(--text-dim)' }}>
              {state === 'checking' ? t('updates.checkingForUpdates') : t('updates.upToDate')}
            </div>
          </div>
          <button
            className="btn-secondary"
            onClick={onCheck}
            disabled={state === 'checking'}
            style={{ fontSize: 12, padding: '6px 14px' }}
          >
            {state === 'checking' ? t('common.loading') : t('updates.checkNow')}
          </button>
        </div>
      </div>
    );
  }

  if (state === 'error') {
    return (
      <div className="card" style={{ marginBottom: 16, borderColor: 'var(--danger)' }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <div>
            <div
              className="card-title"
              style={{ margin: 0, marginBottom: 4, color: 'var(--danger)' }}
            >
              {t('updates.updateError')}
            </div>
            <div style={{ fontSize: '0.85rem', color: 'var(--text-dim)' }}>{errorMsg}</div>
          </div>
          <div style={{ display: 'flex', gap: 8 }}>
            <button
              className="btn-secondary"
              onClick={onCheck}
              style={{ fontSize: 12, padding: '6px 14px' }}
            >
              {t('updates.checkNow')}
            </button>
            <button
              className="btn-secondary"
              onClick={onDismiss}
              style={{ fontSize: 12, padding: '6px 14px' }}
            >
              {t('common.close')}
            </button>
          </div>
        </div>
      </div>
    );
  }

  if (state === 'available') {
    return (
      <div
        className="card"
        style={{
          marginBottom: 16,
          borderColor: 'oklch(from var(--primary) l c h / 0.4)',
          background: 'oklch(from var(--primary) l c h / 0.06)',
        }}
      >
        <div
          style={{
            display: 'flex',
            alignItems: 'flex-start',
            justifyContent: 'space-between',
            gap: 16,
          }}
        >
          <div style={{ flex: 1, minWidth: 0 }}>
            <div className="card-title" style={{ margin: 0, marginBottom: 6 }}>
              🎉 {t('updates.newVersionAvailable')} — v{updateInfo?.version}
            </div>
            {updateInfo?.body && (
              <div
                style={{
                  fontSize: '0.82rem',
                  color: 'var(--text-dim)',
                  maxHeight: 120,
                  overflow: 'auto',
                  whiteSpace: 'pre-wrap',
                  lineHeight: 1.5,
                }}
              >
                {updateInfo.body}
              </div>
            )}
          </div>
          <div style={{ display: 'flex', gap: 8, flexShrink: 0 }}>
            <button
              className="btn-primary"
              onClick={onInstall}
              style={{ fontSize: 13, padding: '8px 20px' }}
            >
              {t('updates.downloadAndInstall')}
            </button>
            <button
              className="btn-secondary"
              onClick={onDismiss}
              style={{ fontSize: 12, padding: '8px 14px' }}
            >
              {t('common.close')}
            </button>
          </div>
        </div>
      </div>
    );
  }

  if (state === 'downloading') {
    return (
      <div className="card" style={{ marginBottom: 16 }}>
        <div className="card-title" style={{ marginBottom: 12 }}>
          {t('updates.downloading')} {progress}%
        </div>
        <div
          style={{
            width: '100%',
            height: 8,
            borderRadius: 99,
            background: 'var(--surface-2)',
            overflow: 'hidden',
          }}
        >
          <div
            style={{
              width: `${progress}%`,
              height: '100%',
              borderRadius: 99,
              background: 'var(--primary)',
              transition: 'width 0.3s ease',
            }}
          />
        </div>
      </div>
    );
  }

  if (state === 'ready') {
    return (
      <div
        className="card"
        style={{
          marginBottom: 16,
          borderColor: 'oklch(from var(--success) l c h / 0.4)',
          background: 'oklch(from var(--success) l c h / 0.06)',
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <div>
            <div
              className="card-title"
              style={{ margin: 0, marginBottom: 4, color: 'var(--success)' }}
            >
              ✓ {t('updates.downloadComplete')}
            </div>
            <div style={{ fontSize: '0.85rem', color: 'var(--text-dim)' }}>
              {t('updates.restarting')}
            </div>
          </div>
        </div>
      </div>
    );
  }

  // installing state
  return (
    <div className="card" style={{ marginBottom: 16 }}>
      <div className="card-title" style={{ marginBottom: 8 }}>
        {t('updates.installing')}
      </div>
      <div className="loading-spinner" role="status">
        {t('common.loading')}
      </div>
    </div>
  );
}
