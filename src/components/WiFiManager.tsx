import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useToast } from '../contexts/ToastContext';
import { t } from '../hooks/useI18n';

interface WifiNetwork {
  ssid: string;
  signal: number;
  security: string;
  connected: boolean;
}

interface WifiStatus {
  connected: boolean;
  ssid: string | null;
  signal: number | null;
  interface: string | null;
}

export default function WiFiManager() {
  const [networks, setNetworks] = useState<WifiNetwork[]>([]);
  const [status, setStatus] = useState<WifiStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [connecting, setConnecting] = useState(false);
  const [newPassword, setNewPassword] = useState('');
  const [selectedSsid, setSelectedSsid] = useState<string | null>(null);
  const { addToast } = useToast();

  const loadData = useCallback(async () => {
    try {
      const [wifiStatus, scanResults] = await Promise.all([
        invoke<WifiStatus>('wifi_status'),
        invoke<WifiNetwork[]>('wifi_scan'),
      ]);
      setStatus(wifiStatus);
      setNetworks(scanResults);
    } catch (e) {
      console.error('Failed to load WiFi data:', e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadData();
  }, [loadData]);

  const handleConnect = async (ssid: string) => {
    setSelectedSsid(ssid);
    setConnecting(true);
    try {
      await invoke('wifi_connect', { ssid, password: newPassword || null });
      addToast(`Connected to "${ssid}"`, 'success');
      setNewPassword('');
      loadData();
    } catch (e) {
      addToast(`Connect error: ${String(e)}`, 'error');
    } finally {
      setConnecting(false);
    }
  };

  const handleDisconnect = async () => {
    try {
      await invoke('wifi_disconnect');
      addToast('Disconnected', 'info');
      loadData();
    } catch (e) {
      addToast(`Disconnect error: ${String(e)}`, 'error');
    }
  };

  const handleRefresh = () => {
    setLoading(true);
    loadData();
  };

  if (loading) {
    return (
      <div className="card">
        <div className="card-title">📶 {t('wifi.title')}</div>
        <p className="page-subtitle">{t('common.loading')}</p>
      </div>
    );
  }

  return (
    <div className="card">
      <div className="card-title">📶 {t('wifi.title')}</div>
      <p className="page-subtitle">
        {status?.connected && status.ssid
          ? `${t('wifi.connectedTo')}: ${status.ssid}`
          : t('wifi.notConnected')}
      </p>

      {/* Current connection */}
      {status?.connected && (
        <div
          style={{
            marginTop: 12,
            padding: 10,
            background: 'var(--bg-hover)',
            borderRadius: 'var(--r-xs)',
          }}
        >
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <div>
              <div style={{ fontWeight: 600, fontSize: 14 }}>{status.ssid}</div>
              <div style={{ fontSize: 12, color: 'var(--text-dim)' }}>
                {status.signal !== null && `${t('wifi.signal')}: ${status.signal}%`}
              </div>
            </div>
            <button className="btn btn-secondary" onClick={handleDisconnect}>
              {t('wifi.disconnect')}
            </button>
          </div>
        </div>
      )}

      {/* Network list */}
      <div style={{ marginTop: 16 }}>
        <div style={{ fontWeight: 600, marginBottom: 8, color: 'var(--text-dim)', fontSize: 13 }}>
          {t('wifi.availableNetworks')}
        </div>
        {networks.length === 0 ? (
          <p style={{ color: 'var(--text-dim)', fontSize: 13 }}>{t('wifi.noNetworks')}</p>
        ) : (
          networks.map((n) => (
            <div
              key={n.ssid}
              className="stat-row"
              style={{
                padding: '8px 10px',
                marginBottom: 4,
                borderRadius: 'var(--r-xs)',
                cursor: 'pointer',
              }}
              onClick={() => setSelectedSsid(selectedSsid === n.ssid ? null : n.ssid)}
            >
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 13, fontWeight: 500 }}>
                  {n.ssid}
                  {n.connected && (
                    <span style={{ color: 'var(--success)', marginLeft: 8 }}>
                      ● {t('wifi.connected')}
                    </span>
                  )}
                </div>
                <div style={{ fontSize: 11, color: 'var(--text-dim)', marginTop: 2 }}>
                  {t('wifi.signal')}: {n.signal}% • {n.security || 'Open'}
                </div>
              </div>
              {selectedSsid === n.ssid && !n.connected && (
                <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                  <input
                    type="password"
                    placeholder={t('wifi.password')}
                    value={newPassword}
                    onChange={(e) => setNewPassword(e.target.value)}
                    onClick={(e) => e.stopPropagation()}
                    style={{
                      padding: '4px 8px',
                      borderRadius: 'var(--r-xs)',
                      border: '1px solid var(--border)',
                      background: 'var(--bg)',
                      color: 'var(--text)',
                      width: 140,
                      fontSize: 12,
                    }}
                  />
                  <button
                    className="btn btn-primary"
                    onClick={(e) => {
                      e.stopPropagation();
                      handleConnect(n.ssid);
                    }}
                    disabled={connecting}
                    style={{ padding: '4px 10px', fontSize: 12 }}
                  >
                    {connecting ? '...' : t('wifi.connect')}
                  </button>
                </div>
              )}
            </div>
          ))
        )}
      </div>

      <button
        className="btn btn-secondary"
        onClick={handleRefresh}
        style={{ marginTop: 12, width: '100%' }}
      >
        🔄 {t('wifi.refresh')}
      </button>
    </div>
  );
}
