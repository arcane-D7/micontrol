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

// ── MIOT-06 LocalSend ───────────────────────────────────────────────────────
interface LocalSendPeer {
  alias: string;
  fingerprint: string;
  protocol: 'http' | 'https';
  port: number;
  addr: string;
  deviceType?: string | null;
  download: boolean;
}

interface SendReport {
  sessionId: string;
  files: string[];
  total_bytes: number;
}

interface ReceiverStatus {
  running: boolean;
  port: number;
  received_count: number;
}

// ── MIOT-07 scrcpy camera ───────────────────────────────────────────────────
interface ScrcpyStatus {
  binary: string;
  version: string | null;
  state: 'not-installed' | 'running' | 'stopped' | 'error';
  pid: number | null;
  error: string | null;
  installHint: string;
}

// ── MIOT-08 transcription ───────────────────────────────────────────────────
interface TranscriptionStatus {
  binaryInstalled: boolean;
  binaryPath: string | null;
  modelReady: boolean;
  modelDir: string | null;
  installHint: string;
  missing: string[];
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

  // LocalSend (MIOT-06)
  const [peers, setPeers] = useState<LocalSendPeer[]>([]);
  const [discovering, setDiscovering] = useState(false);
  const [sendPath, setSendPath] = useState('');
  const [sendingTo, setSendingTo] = useState<string | null>(null);
  const [sendResult, setSendResult] = useState<string | null>(null);
  const [receiver, setReceiver] = useState<ReceiverStatus | null>(null);

  // scrcpy camera (MIOT-07)
  const [camera, setCamera] = useState<ScrcpyStatus | null>(null);
  const [cameraBusy, setCameraBusy] = useState(false);

  // Transcription (MIOT-08)
  const [transc, setTransc] = useState<TranscriptionStatus | null>(null);
  const [wavPath, setWavPath] = useState('');
  const [transcribing, setTranscribing] = useState(false);
  const [downloadingModel, setDownloadingModel] = useState(false);
  const [transcript, setTranscript] = useState<string | null>(null);
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

  const fetchReceiver = useCallback(async () => {
    try {
      const r = await invoke<ReceiverStatus>('localsend_receiver_status');
      setReceiver(r);
    } catch {
      setReceiver(null);
    }
  }, []);

  const fetchCamera = useCallback(async () => {
    try {
      const s = await invoke<ScrcpyStatus>('scrcpy_status');
      setCamera(s);
    } catch {
      setCamera(null);
    }
  }, []);

  const fetchTranscription = useCallback(async () => {
    try {
      const s = await invoke<TranscriptionStatus>('transcription_status');
      setTransc(s);
    } catch {
      setTransc(null);
    }
  }, []);

  useEffect(() => {
    void fetchStatus();
    void fetchPresence();
    void fetchReceiver();
    void fetchCamera();
    void fetchTranscription();
  }, [fetchStatus, fetchPresence, fetchReceiver, fetchCamera, fetchTranscription]);

  const handleDiscover = async () => {
    setDiscovering(true);
    setSendResult(null);
    try {
      const found = await invoke<LocalSendPeer[]>('localsend_discover', { seconds: 3 });
      setPeers(found);
      if (found.length === 0) setSendResult(t('crossDevice.lsNoPeers'));
    } catch (e) {
      setErrorMsg(String(e));
    } finally {
      setDiscovering(false);
    }
  };

  const handleSend = async (peer: LocalSendPeer) => {
    if (!sendPath.trim()) {
      setSendResult(t('crossDevice.lsFilePath'));
      return;
    }
    setSendingTo(peer.alias);
    try {
      const report = await invoke<SendReport>('localsend_send_files', {
        peer,
        paths: [sendPath],
      });
      setSendResult(
        t('crossDevice.lsSent', { count: report.files.length, bytes: report.total_bytes }),
      );
    } catch (e) {
      setErrorMsg(String(e));
    } finally {
      setSendingTo(null);
    }
  };

  const handleReceiverToggle = async () => {
    try {
      if (receiver?.running) {
        const r = await invoke<ReceiverStatus>('localsend_receiver_stop');
        setReceiver(r);
      } else {
        const r = await invoke<ReceiverStatus>('localsend_receiver_start');
        setReceiver(r);
      }
    } catch (e) {
      setErrorMsg(String(e));
    }
  };

  const handleCameraToggle = async () => {
    setCameraBusy(true);
    try {
      if (camera?.state === 'running') {
        await invoke('scrcpy_stop');
      } else {
        await invoke('scrcpy_start');
      }
      const s = await invoke<ScrcpyStatus>('scrcpy_status');
      setCamera(s);
    } catch (e) {
      setErrorMsg(String(e));
    } finally {
      setCameraBusy(false);
    }
  };

  const handleDownloadModel = async () => {
    setDownloadingModel(true);
    try {
      await invoke('transcription_download_model');
      const s = await invoke<TranscriptionStatus>('transcription_status');
      setTransc(s);
    } catch (e) {
      setErrorMsg(String(e));
    } finally {
      setDownloadingModel(false);
    }
  };

  const handleTranscribe = async () => {
    if (!wavPath.trim()) return;
    setTranscribing(true);
    setTranscript(null);
    try {
      const text = await invoke<string>('transcribe_audio', { wavPath });
      setTranscript(text);
    } catch (e) {
      setErrorMsg(String(e));
    } finally {
      setTranscribing(false);
    }
  };

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

      {/* LocalSend — LAN file transfer (MIOT-06) */}
      <div className="card" style={{ marginBottom: 16 }}>
        <h3>📡 {t('crossDevice.lsTitle')}</h3>
        <p className="text-muted" style={{ marginBottom: 12 }}>
          {t('crossDevice.lsDesc')}
        </p>

        {/* Receiver (this PC) */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 12 }}>
          <span style={{ fontSize: 13 }}>{t('crossDevice.lsReceiverTitle')}</span>
          <button
            className={`btn ${receiver?.running ? 'btn-primary' : 'btn-secondary'}`}
            onClick={handleReceiverToggle}
          >
            {receiver?.running ? t('crossDevice.lsReceiverStop') : t('crossDevice.lsReceiverStart')}
          </button>
          {receiver?.running && (
            <span className="status-ok" style={{ fontSize: 13 }}>
              {t('crossDevice.lsReceiverRunning', { port: receiver.port })}
            </span>
          )}
          {receiver && !receiver.running && (
            <span className="text-muted" style={{ fontSize: 13 }}>
              {t('crossDevice.lsReceiverStopped')}
            </span>
          )}
        </div>
        {receiver && receiver.received_count > 0 && (
          <p className="text-muted" style={{ fontSize: 12, marginBottom: 8 }}>
            {t('crossDevice.lsReceived')}: {receiver.received_count}
          </p>
        )}

        {/* Discover + send */}
        <div style={{ display: 'flex', gap: 8, alignItems: 'center', marginBottom: 12 }}>
          <button className="btn btn-secondary" onClick={handleDiscover} disabled={discovering}>
            {discovering ? t('crossDevice.lsDiscovering') : `🔎 ${t('crossDevice.lsDiscover')}`}
          </button>
          {peers.length > 0 && (
            <span className="text-muted" style={{ fontSize: 12 }}>
              {peers.length} {peers.length === 1 ? 'device' : 'devices'}
            </span>
          )}
        </div>

        {peers.map((peer) => (
          <div
            key={peer.fingerprint}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 12,
              marginBottom: 8,
              padding: 10,
              borderRadius: 8,
              border: '1px solid var(--border)',
            }}
          >
            <span style={{ fontSize: 22 }}>📱</span>
            <div style={{ flex: 1 }}>
              <div style={{ fontWeight: 600 }}>{peer.alias}</div>
              <div className="text-muted" style={{ fontSize: 12 }}>
                {peer.deviceType ?? t('crossDevice.lsPeerType')} · {peer.protocol} · {peer.addr}:
                {peer.port}
              </div>
            </div>
            <input
              type="text"
              className="text-input"
              placeholder={t('crossDevice.lsFilePathPlaceholder')}
              value={sendPath}
              onChange={(e) => setSendPath(e.target.value)}
              style={{ width: 260, fontSize: 12 }}
            />
            <button
              className="btn btn-primary"
              onClick={() => void handleSend(peer)}
              disabled={sendingTo !== null}
            >
              {sendingTo === peer.alias ? t('crossDevice.lsSending') : t('crossDevice.lsSend')}
            </button>
          </div>
        ))}

        {sendResult && (
          <p className="status-ok" style={{ fontSize: 13 }}>
            ✓ {sendResult}
          </p>
        )}
      </div>

      {/* scrcpy — phone camera (MIOT-07) */}
      <div className="card" style={{ marginBottom: 16 }}>
        <h3>📷 {t('crossDevice.cameraTitle')}</h3>
        <p className="text-muted" style={{ marginBottom: 12 }}>
          {t('crossDevice.cameraDesc')}
        </p>

        {camera?.state === 'not-installed' ? (
          <div className="alert alert-warn" style={{ marginBottom: 8 }}>
            {t('crossDevice.cameraNotInstalled')}{' '}
            <span className="text-muted" style={{ fontSize: 12 }}>
              {t('crossDevice.cameraInstallHint')}
            </span>
          </div>
        ) : (
          <>
            <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
              <button
                className={`btn ${camera?.state === 'running' ? 'btn-primary' : 'btn-secondary'}`}
                onClick={handleCameraToggle}
                disabled={cameraBusy}
              >
                {camera?.state === 'running'
                  ? t('crossDevice.cameraStop')
                  : t('crossDevice.cameraStart')}
              </button>
              {camera?.state === 'running' && camera.pid != null && (
                <span className="status-ok" style={{ fontSize: 13 }}>
                  {t('crossDevice.cameraRunning', { pid: camera.pid })}
                </span>
              )}
              {camera?.state === 'stopped' && (
                <span className="text-muted" style={{ fontSize: 13 }}>
                  {t('crossDevice.cameraStopped')}
                </span>
              )}
            </div>
            {camera?.state === 'error' && camera.error && (
              <p className="text-muted" style={{ fontSize: 12, marginTop: 8 }}>
                {camera.error}
              </p>
            )}
            {camera?.version && (
              <p className="text-muted" style={{ fontSize: 12, marginTop: 8 }}>
                {t('crossDevice.cameraVersion')}: {camera.version}
              </p>
            )}
          </>
        )}
      </div>

      {/* On-device transcription (MIOT-08) */}
      <div className="card" style={{ marginBottom: 16 }}>
        <h3>🎙️ {t('crossDevice.transcribeTitle')}</h3>
        <p className="text-muted" style={{ marginBottom: 12 }}>
          {t('crossDevice.transcribeDesc')}
        </p>

        {transc && !transc.binaryInstalled && (
          <div className="alert alert-warn" style={{ marginBottom: 8 }}>
            {t('crossDevice.transcribeNotInstalled')}{' '}
            <span className="text-muted" style={{ fontSize: 12 }}>
              {t('crossDevice.transcribeInstallHint')}
            </span>
          </div>
        )}

        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 12,
            flexWrap: 'wrap',
            marginBottom: 10,
          }}
        >
          <span style={{ fontSize: 13 }}>{t('crossDevice.transcribePath')}</span>
          <input
            type="text"
            className="text-input"
            placeholder={t('crossDevice.transcribePathPlaceholder')}
            value={wavPath}
            onChange={(e) => setWavPath(e.target.value)}
            style={{ flex: 1, minWidth: 200, fontSize: 12 }}
          />
          <button
            className="btn btn-primary"
            onClick={handleTranscribe}
            disabled={transcribing || !wavPath.trim() || !transc?.modelReady}
          >
            {transcribing
              ? t('crossDevice.transcribeTranscribing')
              : t('crossDevice.transcribeBtn')}
          </button>
          <button
            className="btn btn-secondary"
            onClick={handleDownloadModel}
            disabled={downloadingModel || transc?.modelReady}
          >
            {downloadingModel
              ? t('crossDevice.transcribeDownloading')
              : transc?.modelReady
                ? `✓ ${t('crossDevice.transcribeModelReady')}`
                : t('crossDevice.transcribeDownloadModel')}
          </button>
        </div>
        {transc && !transc.modelReady && (
          <p className="text-muted" style={{ fontSize: 12 }}>
            {t('crossDevice.transcribeModelMissing')}
          </p>
        )}

        {transcript !== null && (
          <div
            className="alert alert-success"
            style={{ marginTop: 10, whiteSpace: 'pre-wrap', fontSize: 13 }}
          >
            <strong>{t('crossDevice.transcribeResult')}:</strong>{' '}
            {transcript || t('crossDevice.transcribeNoResult')}
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
