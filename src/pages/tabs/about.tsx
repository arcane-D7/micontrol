import { PageHeader } from './PageHeader';
import { invoke } from '@tauri-apps/api/core';
import { t } from '../../hooks/useI18n';
import AppUpdateBanner from '../../components/AppUpdateBanner';
import BridgeUpdateCard from '../../components/BridgeUpdateCard';
import AutoUpdateCard from '../../components/AutoUpdateCard';
import type { AppUpdateState, AppUpdateInfo } from '../../hooks/useAutoUpdate';

const APP_VERSION = typeof __APP_VERSION__ !== 'undefined' ? __APP_VERSION__ : '0.0.0';

interface Props {
  appUpdateState?: AppUpdateState;
  appUpdateInfo?: AppUpdateInfo | null;
  appUpdateProgress?: number;
  appUpdateError?: string;
  onCheckAppUpdate?: () => void;
  onInstallAppUpdate?: () => void;
  onDismissAppUpdate?: () => void;
}

export default function AboutTab({
  appUpdateState = 'idle',
  appUpdateInfo = null,
  appUpdateProgress = 0,
  appUpdateError = '',
  onCheckAppUpdate,
  onInstallAppUpdate,
  onDismissAppUpdate,
}: Props) {
  return (
    <>
      <PageHeader title={t('about.title')} />

      {/* App self-update banner */}
      {onCheckAppUpdate && onInstallAppUpdate && onDismissAppUpdate && (
        <AppUpdateBanner
          state={appUpdateState}
          updateInfo={appUpdateInfo}
          progress={appUpdateProgress}
          errorMsg={appUpdateError}
          onCheck={onCheckAppUpdate}
          onInstall={onInstallAppUpdate}
          onDismiss={onDismissAppUpdate}
        />
      )}

      <div className="card">
        <div className="grid-2">
          <div>
            <div className="stat-row">
              <span className="stat-label">{t('about.appName')}</span>
              <span className="stat-value">MiControl</span>
            </div>
            <div className="stat-row">
              <span className="stat-label">{t('about.version')}</span>
              <span className="stat-value">{APP_VERSION}</span>
            </div>
            <div className="stat-row">
              <span className="stat-label">{t('about.device')}</span>
              <span className="stat-value">Xiaomi Laptop Pro</span>
            </div>
          </div>
          <div>
            <div className="stat-row">
              <span className="stat-label">{t('about.author')}</span>
              <span className="stat-value">MiControl Contributors</span>
            </div>
            <div className="stat-row">
              <span className="stat-label">{t('about.license')}</span>
              <span className="stat-value">MIT License</span>
            </div>
            <div className="stat-row">
              <span className="stat-label">{t('about.github')}</span>
              <span className="stat-value">GitHub Repository</span>
            </div>
          </div>
        </div>
        <p style={{ marginTop: 16, fontSize: 12, color: 'var(--color-text-muted)' }}>
          {t('about.description')}
        </p>
      </div>
      <div className="card">
        <div className="card-title">{t('about.drivers')}</div>
        <div className="grid-2">
          <div className="stat-row">
            <span className="stat-label">{t('about.driversList.virtualControlHID')}</span>
          </div>
          <div className="stat-row">
            <span className="stat-label">{t('about.driversList.iotDriver')}</span>
          </div>
        </div>
      </div>

      {/* S45-001: silent self-update via the privileged bridge service */}
      <BridgeUpdateCard />

      {/* S49: Buy me a coffee — optional thank-you donation */}
      <div className="card">
        <div className="card-title">{t('about.support.title')}</div>
        <div style={{ display: 'flex', alignItems: 'flex-start', gap: 14 }}>
          <span
            aria-hidden="true"
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              justifyContent: 'center',
              width: 44,
              height: 44,
              borderRadius: 12,
              background: 'oklch(from var(--warning) l c h / 0.15)',
              fontSize: 22,
              flexShrink: 0,
            }}
          >
            ☕
          </span>
          <div style={{ flex: 1, minWidth: 0 }}>
            <p
              className="text-sm"
              style={{ color: 'var(--color-text-muted)', marginBottom: 12, lineHeight: 1.6 }}
            >
              {t('about.support.desc')}
            </p>
            <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap' }}>
              <button
                type="button"
                className="btn-primary btn-sm"
                onClick={() =>
                  void invoke('open_external_url', {
                    url: 'https://buymeacoffee.com/micontrol',
                  }).catch((e) => console.error('[support] open failed:', e))
                }
              >
                ☕ {t('about.support.bmc')}
              </button>
              <button
                type="button"
                className="btn-ghost btn-sm"
                onClick={() =>
                  void invoke('open_external_url', {
                    url: 'https://github.com/arcane-D7/micontrol',
                  }).catch((e) => console.error('[support] open failed:', e))
                }
              >
                ⭐ {t('about.support.star')}
              </button>
            </div>
            <p
              className="text-xs"
              style={{
                color: 'var(--color-text-dim)',
                marginTop: 12,
                marginBottom: 0,
                lineHeight: 1.5,
              }}
            >
              {t('about.support.note')}
            </p>
          </div>
        </div>
      </div>

      {/* S45-002: auto-update toggle (off by default) + hidden dev beta feed */}
      <AutoUpdateCard />
    </>
  );
}
