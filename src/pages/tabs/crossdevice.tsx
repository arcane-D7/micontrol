import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';

// ── Types matching Rust structs ──────────────────────────────────────────────

interface PhoneLinkStatus {
  installed: boolean;
  paired: boolean;
  device_name: string | null;
  package_version: string | null;
  running: boolean;
}

// ── Component ────────────────────────────────────────────────────────────────

export default function CrossDeviceTab() {
  const [status, setStatus] = useState<PhoneLinkStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const fetchStatus = useCallback(async () => {
    try {
      const s = await invoke<PhoneLinkStatus>('get_phone_link_status');
      setStatus(s);
    } catch (e) {
      setStatus(null);
      setErrorMsg(String(e));
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    void fetchStatus();
  }, [fetchStatus]);

  const handleLaunch = async () => {
    try {
      await invoke('launch_phone_link');
    } catch (e) {
      setErrorMsg(String(e));
    }
  };

  const handleOpenSettings = async () => {
    try {
      await invoke('open_phone_link_settings');
    } catch (e) {
      setErrorMsg(String(e));
    }
  };

  const handleFeature = async (feature: string) => {
    try {
      await invoke('launch_phone_link_feature', { feature });
    } catch (e) {
      setErrorMsg(String(e));
    }
  };

  if (loading) {
    return (
      <>
        <PageHeader title={t('crossDevice.title')} subtitle={t('crossDevice.subtitle')} />
        <div className="loading-spinner">{t('common.loading')}</div>
      </>
    );
  }

  const features = [
    {
      id: 'Phone',
      icon: '📞',
      labelKey: 'crossDevice.featureCalls',
      descKey: 'crossDevice.featureCallsDesc',
    },
    {
      id: 'Messages',
      icon: '💬',
      labelKey: 'crossDevice.featureMessages',
      descKey: 'crossDevice.featureMessagesDesc',
    },
    {
      id: 'Photos',
      icon: '📸',
      labelKey: 'crossDevice.featurePhotos',
      descKey: 'crossDevice.featurePhotosDesc',
    },
    {
      id: 'ScreenMirror',
      icon: '📱',
      labelKey: 'crossDevice.featureScreenMirror',
      descKey: 'crossDevice.featureScreenMirrorDesc',
    },
    {
      id: 'Apps',
      icon: '🎯',
      labelKey: 'crossDevice.featureApps',
      descKey: 'crossDevice.featureAppsDesc',
    },
  ];

  return (
    <>
      <PageHeader title={t('crossDevice.title')} subtitle={t('crossDevice.subtitle')} />

      {/* Error message */}
      {errorMsg && (
        <div className="alert alert-error" style={{ marginBottom: 16 }}>
          ⚠ {errorMsg}
        </div>
      )}

      {/* Status Card */}
      <div className="card" style={{ marginBottom: 16 }}>
        <h3>{t('crossDevice.status')}</h3>
        {status ? (
          <div className="info-grid">
            <div className="info-row">
              <span className="info-label">{t('crossDevice.installed')}</span>
              <span className={`info-value ${status.installed ? 'status-ok' : 'status-warn'}`}>
                {status.installed ? t('crossDevice.installed') : t('crossDevice.notInstalled')}
              </span>
            </div>
            {status.installed && (
              <>
                <div className="info-row">
                  <span className="info-label">{t('crossDevice.paired')}</span>
                  <span className={`info-value ${status.paired ? 'status-ok' : 'status-warn'}`}>
                    {status.paired ? t('crossDevice.paired') : t('crossDevice.notPaired')}
                  </span>
                </div>
                {status.device_name && (
                  <div className="info-row">
                    <span className="info-label">{t('crossDevice.deviceName')}</span>
                    <span className="info-value">{status.device_name}</span>
                  </div>
                )}
                {status.package_version && (
                  <div className="info-row">
                    <span className="info-label">{t('crossDevice.version')}</span>
                    <span className="info-value">{status.package_version}</span>
                  </div>
                )}
                <div className="info-row">
                  <span className="info-label">{t('crossDevice.running')}</span>
                  <span className={`info-value ${status.running ? 'status-ok' : 'text-muted'}`}>
                    {status.running ? t('crossDevice.running') : t('crossDevice.notRunning')}
                  </span>
                </div>
              </>
            )}
          </div>
        ) : (
          <p className="text-muted">{t('errors.unknownError')}</p>
        )}

        {/* Status hints */}
        {status && !status.installed && (
          <p className="text-muted" style={{ marginTop: 8 }}>
            {t('crossDevice.notInstalledDesc')}
          </p>
        )}
        {status && status.installed && !status.paired && (
          <p className="text-muted" style={{ marginTop: 8 }}>
            {t('crossDevice.notPairedDesc')}
          </p>
        )}
      </div>

      {/* Quick Actions */}
      {status?.installed && (
        <div className="card" style={{ marginBottom: 16 }}>
          <h3>{t('crossDevice.features')}</h3>
          <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', marginBottom: 12 }}>
            <button className="btn btn-primary" onClick={handleLaunch}>
              📱 {t('crossDevice.launchApp')}
            </button>
            <button className="btn btn-secondary" onClick={handleOpenSettings}>
              ⚙️ {t('crossDevice.openSettings')}
            </button>
          </div>

          {/* Feature grid */}
          <div
            className="feature-grid"
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fill, minmax(200px, 1fr))',
              gap: 12,
            }}
          >
            {features.map((f) => (
              <button
                key={f.id}
                className="feature-card btn btn-secondary"
                onClick={() => handleFeature(f.id)}
                style={{
                  display: 'flex',
                  flexDirection: 'column',
                  alignItems: 'flex-start',
                  padding: 16,
                  textAlign: 'left',
                  gap: 4,
                }}
              >
                <span style={{ fontSize: 24 }}>{f.icon}</span>
                <span style={{ fontWeight: 600 }}>{t(f.labelKey as Parameters<typeof t>[0])}</span>
                <span className="text-muted" style={{ fontSize: 12 }}>
                  {t(f.descKey as Parameters<typeof t>[0])}
                </span>
              </button>
            ))}
          </div>
        </div>
      )}

      {/* Info Card */}
      <div className="card">
        <h3>ℹ️ {t('crossDevice.supportedDevices')}</h3>
        <p className="text-muted">{t('crossDevice.installHint')}</p>
      </div>
    </>
  );
}
