import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { IotDeviceInfo } from '../types/hardware';
import { Icon } from './ui/icons';
import { t, type StringKey } from '../hooks/useI18n';

/** All 16 EC commands available via the EC command protocol. */
const EC_COMMANDS: { id: string; name: string; descKey: StringKey }[] = [
  { id: '0x01', name: 'GetBindStatus', descKey: 'iot.cmd.bindStatus' },
  { id: '0x02', name: 'SetBindStatus', descKey: 'iot.cmd.setBindStatus' },
  { id: '0x03', name: 'ResetDevice', descKey: 'iot.cmd.resetDevice' },
  { id: '0x04', name: 'WriteWiFiItem', descKey: 'iot.cmd.writeWifiItem' },
  { id: '0x05', name: 'EmptyWiFiItems', descKey: 'iot.cmd.emptyWifiItems' },
  { id: '0x06', name: 'DeleteWiFiItem', descKey: 'iot.cmd.deleteWifiItem' },
  { id: '0x07', name: 'ReadWiFiStatus', descKey: 'iot.cmd.readWifiStatus' },
  { id: '0x08', name: 'ReadWiFiCount', descKey: 'iot.cmd.readWifiCount' },
  { id: '0x09', name: 'GetWiFiByIndex', descKey: 'iot.cmd.getWifiByIndex' },
  { id: '0x0A', name: 'GetFwVersion', descKey: 'iot.cmd.getFwVersion' },
  { id: '0x0B', name: 'GetModel', descKey: 'iot.cmd.getModel' },
  { id: '0x0C', name: 'ConnectWiFi', descKey: 'iot.cmd.connectWifi' },
  { id: '0x0D', name: 'GetDeviceID', descKey: 'iot.cmd.getDeviceId' },
  { id: '0x0E', name: 'SendLaptopStatus (SUSPEND)', descKey: 'iot.cmd.sendLaptopStatusSuspend' },
  { id: '0x0F', name: 'SendLaptopStatus (SHUTDOWN)', descKey: 'iot.cmd.sendLaptopStatusShutdown' },
  { id: '0x10', name: 'SendLaptopStatus (WIN_READY)', descKey: 'iot.cmd.sendLaptopStatusWinReady' },
];

export default function IotDeviceCard() {
  const [info, setInfo] = useState<IotDeviceInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [retryCount, setRetryCount] = useState(0);
  const [ensuringService, setEnsuringService] = useState(false);
  const [showCommands, setShowCommands] = useState(false);

  // Hard cap on automatic retries so a missing bridge/service does not keep
  // polling forever (throttle: 3 tries total, then stop and wait for the user).
  const MAX_RETRIES = 3;

  const loadInfo = useCallback(async () => {
    try {
      const data = await invoke<IotDeviceInfo>('get_iot_device_info');
      // Defensive: ensure all string fields are actually strings, not objects
      // (prevents [object Object] if backend returns unexpected shapes)
      if (data) {
        if (typeof data.device_status !== 'string' && data.device_status !== null) {
          data.device_status = String(data.device_status);
        }
        if (typeof data.model !== 'string' && data.model !== null) {
          data.model = String(data.model);
        }
        if (typeof data.fw_version !== 'string' && data.fw_version !== null) {
          data.fw_version = String(data.fw_version);
        }
      }
      setInfo(data);
    } catch (e) {
      console.error('Failed to load IoT device info:', e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadInfo();
  }, [loadInfo]);

  // When pipe is not available, try to start the ecram_service bridge once,
  // then retry a few times (bounded), then give up and wait for a manual tap.
  useEffect(() => {
    if (info?.pipe_available === false && !ensuringService && retryCount < MAX_RETRIES) {
      setEnsuringService(true);
      invoke('ensure_iot_service')
        .then(() => {
          // Wait a moment for the pipe to become available, then reload
          setTimeout(() => {
            void loadInfo();
            setEnsuringService(false);
          }, 3000);
        })
        .catch((e) => {
          console.warn('Failed to ensure IoT service:', e);
          setEnsuringService(false);
        });
    }
  }, [info?.pipe_available, ensuringService, retryCount, loadInfo]);

  // Bounded auto-retry every 5 seconds (stops after MAX_RETRIES).
  useEffect(() => {
    if (info?.pipe_available === false && retryCount < MAX_RETRIES) {
      const timer = setTimeout(() => {
        setRetryCount((c) => c + 1);
        void loadInfo();
      }, 5000);
      return () => clearTimeout(timer);
    }
  }, [info?.pipe_available, retryCount, loadInfo]);

  const retryNow = () => {
    setRetryCount(0);
    setLoading(true);
    void loadInfo();
  };

  if (loading) {
    return (
      <div className="card">
        <h2 className="card-title">
          <Icon name="plug" size={16} /> {t('iot.device.title')}
        </h2>
        <p className="page-subtitle">{t('iot.device.loading')}</p>
      </div>
    );
  }

  if (!info?.pipe_available) {
    return (
      <div className="card">
        <h2 className="card-title">
          <Icon name="plug" size={16} /> {t('iot.device.title')}
        </h2>
        <p className="page-subtitle" style={{ color: 'var(--text-dim)' }}>
          {t('iot.device.unavailable')}
        </p>
        <div style={{ marginTop: 12, fontSize: 12, color: 'var(--text-muted)' }}>
          <div>
            {t('iot.device.expectedPipe')}{' '}
            <code>{'\\\\.\\pipe\\LOCAL\\IoTService_IPC_Broker'}</code>
          </div>
          <div style={{ marginTop: 4 }}>
            {t('iot.device.statusNotFound')}
            {retryCount > 0 &&
              ` ${t('iot.device.retryCounter', { count: retryCount, max: MAX_RETRIES })}`}
          </div>
          <div style={{ marginTop: 8, lineHeight: 1.5 }}>
            {ensuringService
              ? t('iot.device.startingService')
              : retryCount >= MAX_RETRIES
                ? t('iot.device.unreachable')
                : t('iot.device.notRunning')}
          </div>
        </div>
        <div style={{ display: 'flex', gap: 8, marginTop: 12 }}>
          <button className="btn btn-secondary" style={{ flex: 1 }} onClick={retryNow}>
            <Icon name="refresh" size={16} /> {t('iot.device.tryAgain')}
          </button>
          <button className="btn btn-secondary" style={{ flex: 1 }} onClick={() => void loadInfo()}>
            <Icon name="refresh" size={16} /> {t('common.refresh')}
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="card">
      <h2 className="card-title">
        <Icon name="plug" size={16} /> {t('iot.device.title')}
      </h2>
      <p className="page-subtitle">{t('iot.device.subtitle')}</p>

      {/* ── Device Info ─────────────────────────────────────────── */}
      <div style={{ marginTop: 12 }}>
        {info.model && (
          <div className="stat-row">
            <span className="stat-label">{t('iot.device.model')}</span>
            <span className="stat-value">{info.model}</span>
          </div>
        )}
        {info.fw_version && (
          <div className="stat-row">
            <span className="stat-label">{t('iot.device.firmware')}</span>
            <span className="stat-value">{info.fw_version}</span>
          </div>
        )}
        {info.device_id !== null && (
          <div className="stat-row">
            <span className="stat-label">{t('iot.device.deviceId')}</span>
            <span className="stat-value">{info.device_id}</span>
          </div>
        )}
        {info.device_status && (
          <div className="stat-row">
            <span className="stat-label">{t('iot.device.status')}</span>
            <span className="stat-value">{info.device_status}</span>
          </div>
        )}
        {info.bind_status && (
          <div className="stat-row">
            <span className="stat-label">{t('iot.device.cloudBinding')}</span>
            <span
              className="stat-value"
              style={{ color: info.bind_status.bound ? 'var(--success)' : 'var(--text-dim)' }}
            >
              {info.bind_status.bound ? (
                <>
                  <Icon name="check" size={16} />{' '}
                  {t('iot.device.bound', { uid: info.bind_status.uid ?? '' })}
                </>
              ) : (
                t('iot.device.notBound')
              )}
            </span>
          </div>
        )}
        {info.wifi_status && (
          <div className="stat-row">
            <span className="stat-label">{t('iot.device.iotWifi')}</span>
            <span className="stat-value">
              {info.wifi_status.ssid || t('iot.device.notConnected')}
            </span>
          </div>
        )}
        {info.wifi_network_count !== null && (
          <div className="stat-row">
            <span className="stat-label">{t('iot.device.savedNetworks')}</span>
            <span className="stat-value">{info.wifi_network_count}</span>
          </div>
        )}
      </div>

      {/* ── Explanatory Notes ───────────────────────────────────── */}
      <div
        style={{
          marginTop: 16,
          padding: '10px 12px',
          borderRadius: 8,
          background: 'var(--bg-elevated, rgba(255,255,255,0.04))',
          border: '1px solid var(--border-color, rgba(255,255,255,0.08))',
          fontSize: 12,
          lineHeight: 1.6,
          color: 'var(--text-muted)',
        }}
      >
        <div style={{ fontWeight: 600, marginBottom: 6, color: 'var(--text)' }}>
          <Icon name="info" size={16} /> {t('iot.device.about')}
        </div>
        <p style={{ margin: '0 0 8px 0' }}>{t('iot.device.noteCloud')}</p>
        <p style={{ margin: '0 0 8px 0' }}>{t('iot.device.noteWifi')}</p>
        <p style={{ margin: 0 }}>{t('iot.device.noteStatus')}</p>
      </div>

      {/* ── EC Command Protocol ─────────────────────────────────── */}
      <div style={{ marginTop: 12 }}>
        <button
          className="btn btn-secondary"
          onClick={() => setShowCommands((s) => !s)}
          style={{ width: '100%', fontSize: 12 }}
        >
          {showCommands ? <Icon name="chevron-down" size={12} /> : <Icon name="play" size={12} />}{' '}
          {t('iot.device.protocol')}
        </button>
        {showCommands && (
          <div
            style={{
              marginTop: 8,
              borderRadius: 8,
              overflow: 'hidden',
              border: '1px solid var(--border-color, rgba(255,255,255,0.08))',
            }}
          >
            <table
              style={{
                width: '100%',
                borderCollapse: 'collapse',
                fontSize: 11,
              }}
            >
              <thead>
                <tr
                  style={{
                    background: 'var(--bg-elevated, rgba(255,255,255,0.06))',
                    textAlign: 'left',
                  }}
                >
                  <th style={{ padding: '6px 8px', width: 48 }}>{t('iot.device.colId')}</th>
                  <th style={{ padding: '6px 8px' }}>{t('iot.device.colCommand')}</th>
                  <th style={{ padding: '6px 8px' }}>{t('iot.device.colDescription')}</th>
                </tr>
              </thead>
              <tbody>
                {EC_COMMANDS.map((cmd) => (
                  <tr
                    key={cmd.id}
                    style={{
                      borderTop: '1px solid var(--border-color, rgba(255,255,255,0.06))',
                    }}
                  >
                    <td
                      style={{
                        padding: '4px 8px',
                        fontFamily: 'monospace',
                        color: 'var(--text-dim)',
                      }}
                    >
                      {cmd.id}
                    </td>
                    <td style={{ padding: '4px 8px', fontWeight: 500 }}>{cmd.name}</td>
                    <td style={{ padding: '4px 8px', color: 'var(--text-muted)' }}>
                      {t(cmd.descKey)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        <p
          style={{
            marginTop: 6,
            fontSize: 11,
            color: 'var(--text-dim)',
            lineHeight: 1.5,
          }}
        >
          {t('iot.device.protocolNote')}
        </p>
      </div>

      <button
        className="btn btn-secondary"
        onClick={loadInfo}
        style={{ marginTop: 12, width: '100%' }}
      >
        <Icon name="refresh" size={16} /> {t('common.refresh')}
      </button>
    </div>
  );
}
