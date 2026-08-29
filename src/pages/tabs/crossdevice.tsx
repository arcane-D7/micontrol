import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import ToggleRow from '../../components/ToggleRow';

// ── Types matching Rust structs ──────────────────────────────────────────────

interface PhoneLinkStatus {
  installed: boolean;
  paired: boolean;
  device_name: string | null;
  package_version: string | null;
  running: boolean;
}

interface PresenceStatus {
  enabled: boolean;
  state: 'near' | 'far' | 'unknown';
  last_rssi_dbm: number | null;
  last_seen_seconds_ago: number | null;
  phone: string | null;
  scan_ok: boolean;
}

interface PresenceConfig {
  enabled: boolean;
  phone_mac: string | null;
  phone_name: string | null;
  rssi_lock_dbm: number;
  lock_enabled: boolean;
}

// ── Component ────────────────────────────────────────────────────────────────

export default function CrossDeviceTab() {
  const [status, setStatus] = useState<PhoneLinkStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  // BLE presence (MIOT-05)
  const [presence, setPresence] = useState<PresenceStatus | null>(null);
  const [presenceCfg, setPresenceCfg] = useState<PresenceConfig>({
    enabled: false,
    phone_mac: null,
    phone_name: null,
    rssi_lock_dbm: -70,
    lock_enabled: false,
  });
  const [scanning, setScanning] = useState(false);
  const [presenceSaved, setPresenceSaved] = useState(false);
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

  // Load BLE presence config + status on mount (best-effort).
  const fetchPresence = useCallback(async () => {
    try {
      const [cfg, st] = await Promise.all([
        invoke<PresenceConfig>('get_presence_config'),
        invoke<PresenceStatus>('get_presence_status'),
      ]);
      setPresenceCfg({
        enabled: cfg.enabled,
        phone_mac: cfg.phone_mac ?? null,
        phone_name: cfg.phone_name ?? null,
        rssi_lock_dbm: cfg.rssi_lock_dbm,
        lock_enabled: cfg.lock_enabled,
      });
      setPresence(st);
    } catch (e) {
      // Presence monitor may be disabled/unavailable — leave defaults.
      console.debug('Presence status unavailable:', e);
    }
  }, []);

  useEffect(() => {
    void fetchStatus();
    void fetchPresence();
  }, [fetchStatus, fetchPresence]);

  const handleSavePresence = async () => {
    try {
      await invoke('set_presence_config', { config: presenceCfg });
      setPresenceSaved(true);
      setTimeout(() => setPresenceSaved(false), 2500);
      const st = await invoke<PresenceStatus>('get_presence_status');
      setPresence(st);
    } catch (e) {
      setErrorMsg(String(e));
    }
  };

  const handleScanNow = async () => {
    setScanning(true);
    try {
      const st = await invoke<PresenceStatus>('scan_presence_now');
      setPresence(st);
    } catch (e) {
      setErrorMsg(String(e));
    } finally {
      setScanning(false);
    }
  };

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

      {/* BLE Presence (MIOT-05) */}
      <div className="card" style={{ marginBottom: 16 }}>
        <h3>🛰️ {t('crossDevice.presenceTitle')}</h3>
        <p className="text-muted" style={{ marginBottom: 12 }}>
          {t('crossDevice.presenceDesc')}
        </p>

        <ToggleRow
          label={t('crossDevice.presenceEnabled')}
          checked={presenceCfg.enabled}
          onChange={(v) => {
            setPresenceCfg((c) => ({ ...c, enabled: v }));
            setPresenceSaved(false);
          }}
        />

        {presenceCfg.enabled && (
          <div style={{ marginTop: 14, display: 'flex', flexDirection: 'column', gap: 12 }}>
            <div>
              <div style={{ fontSize: 13, marginBottom: 4 }}>
                {t('crossDevice.presencePhoneMac')}
              </div>
              <input
                className="text-input"
                type="text"
                placeholder="AA:BB:CC:DD:EE:FF"
                value={presenceCfg.phone_mac ?? ''}
                onChange={(e) => {
                  setPresenceCfg((c) => ({ ...c, phone_mac: e.target.value || null }));
                  setPresenceSaved(false);
                }}
                style={{ width: '100%' }}
              />
              <p className="text-muted" style={{ fontSize: 12, marginTop: 4 }}>
                {t('crossDevice.presencePhoneHint')}
              </p>
            </div>

            <div>
              <div style={{ fontSize: 13, marginBottom: 4 }}>
                {t('crossDevice.presencePhoneName')}
              </div>
              <input
                className="text-input"
                type="text"
                placeholder={t('crossDevice.presencePhoneName')}
                value={presenceCfg.phone_name ?? ''}
                onChange={(e) => {
                  setPresenceCfg((c) => ({ ...c, phone_name: e.target.value || null }));
                  setPresenceSaved(false);
                }}
                style={{ width: '100%' }}
              />
            </div>

            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                gap: 8,
              }}
            >
              <span style={{ fontSize: 13 }}>{t('crossDevice.presenceThreshold')}</span>
              <input
                className="text-input"
                type="number"
                value={presenceCfg.rssi_lock_dbm}
                onChange={(e) => {
                  setPresenceCfg((c) => ({ ...c, rssi_lock_dbm: Number(e.target.value) }));
                  setPresenceSaved(false);
                }}
                style={{ width: 120 }}
              />
            </div>

            <ToggleRow
              label={t('crossDevice.presenceLock')}
              checked={presenceCfg.lock_enabled}
              onChange={(v) => {
                setPresenceCfg((c) => ({ ...c, lock_enabled: v }));
                setPresenceSaved(false);
              }}
            />

            {/* Live status */}
            {presence && (
              <div className="info-grid" style={{ marginTop: 6 }}>
                <div className="info-row">
                  <span className="info-label">{t('crossDevice.presenceState')}</span>
                  <span
                    className={`info-value ${
                      presence.state === 'near'
                        ? 'status-ok'
                        : presence.state === 'unknown'
                          ? 'text-muted'
                          : 'status-warn'
                    }`}
                  >
                    {presence.state === 'near'
                      ? t('crossDevice.presenceNear')
                      : presence.state === 'far'
                        ? t('crossDevice.presenceFar')
                        : t('crossDevice.presenceUnknown')}
                  </span>
                </div>
                {presence.last_rssi_dbm !== null && (
                  <div className="info-row">
                    <span className="info-label">{t('crossDevice.presenceLastRssi')}</span>
                    <span className="info-value">{presence.last_rssi_dbm} dBm</span>
                  </div>
                )}
              </div>
            )}

            <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', marginTop: 4 }}>
              <button className="btn btn-primary" onClick={handleSavePresence}>
                💾 {t('crossDevice.presenceSave')}
              </button>
              <button className="btn btn-secondary" onClick={handleScanNow} disabled={scanning}>
                📡 {scanning ? '…' : t('crossDevice.presenceScanNow')}
              </button>
            </div>

            {presenceSaved && (
              <p className="status-ok" style={{ fontSize: 13 }}>
                ✓ {t('crossDevice.presenceSaved')}
              </p>
            )}

            {presenceCfg.enabled && !presenceCfg.phone_mac && !presenceCfg.phone_name && (
              <p className="text-muted" style={{ fontSize: 12 }}>
                {t('crossDevice.presenceNoPhone')}
              </p>
            )}
          </div>
        )}
      </div>

      {/* Info Card */}
      <div className="card">
        <h3>ℹ️ {t('crossDevice.supportedDevices')}</h3>
        <p className="text-muted">{t('crossDevice.installHint')}</p>
      </div>
    </>
  );
}
