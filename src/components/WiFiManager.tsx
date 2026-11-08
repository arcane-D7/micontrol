import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useToast } from '../contexts/ToastContext';
import { t } from '../hooks/useI18n';
import { Icon } from './ui/icons';

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

/// The backend creates WPA2-PSK, WPA3-SAE or open profiles — an empty password
/// can only fail for a secured network, so the UI blocks it.
function requiresPassword(security: string): boolean {
  return security !== 'Open' && security !== 'Unknown';
}

/// 802.1X/EAP and legacy WPA/TKIP cannot be provisioned by MiControl: the
/// backend refuses them with an explicit reason, and the UI says so up front
/// instead of offering a Connect button that cannot work.
function isUnsupported(security: string): boolean {
  return /enterprise|802\.1x|legacy/i.test(security);
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
    void loadData();
  }, [loadData]);

  // The password box is shared by every row, so switching networks must not
  // carry the previous network's passphrase over to the next one.
  useEffect(() => {
    setNewPassword('');
  }, [selectedSsid]);

  const handleConnect = async (ssid: string, security: string) => {
    setSelectedSsid(ssid);
    setConnecting(true);
    try {
      // `security` is the label the scan reported. The backend uses it to build
      // a matching profile (WPA2-PSK vs WPA3-SAE vs open) and refuses 802.1X
      // with a real reason instead of failing to associate.
      await invoke('wifi_connect', {
        ssid,
        password: newPassword || null,
        security,
      });
      addToast({ message: `Connected to "${ssid}"`, type: 'success' });
      setNewPassword('');
      void loadData();
    } catch (e) {
      addToast({
        message: `Connect error: ${String(e)}`,
        type: 'error',
        onRetry: () => handleConnect(ssid, security),
      });
    } finally {
      setConnecting(false);
    }
  };

  const handleDisconnect = async () => {
    try {
      await invoke('wifi_disconnect');
      addToast({ message: 'Disconnected', type: 'info' });
      void loadData();
    } catch (e) {
      addToast({
        message: `Disconnect error: ${String(e)}`,
        type: 'error',
        onRetry: handleDisconnect,
      });
    }
  };

  const handleRefresh = () => {
    setLoading(true);
    void loadData();
  };

  if (loading) {
    return (
      <div className="card">
        <h2 className="card-title">
          <Icon name="wifi" size={16} /> {t('wifi.title')}
        </h2>
        <p className="page-subtitle">{t('common.loading')}</p>
      </div>
    );
  }

  return (
    <div className="card">
      <h2 className="card-title">
        <Icon name="wifi" size={16} /> {t('wifi.title')}
      </h2>
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
          networks.map((n) => {
            const isHidden = n.ssid === '<Hidden Network>';
            return (
              <div
                key={n.ssid + (n.connected ? '-conn' : '')}
                className="stat-row"
                style={{
                  padding: '8px 10px',
                  marginBottom: 4,
                  borderRadius: 'var(--r-xs)',
                  cursor: isHidden ? 'default' : 'pointer',
                  opacity: isHidden ? 0.6 : 1,
                }}
                onClick={() =>
                  !isHidden && setSelectedSsid(selectedSsid === n.ssid ? null : n.ssid)
                }
              >
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 13, fontWeight: 500 }}>
                    {isHidden && <Icon name="lock" size={12} />} {n.ssid}
                    {n.connected && (
                      <span style={{ color: 'var(--success)', marginLeft: 8 }}>
                        <Icon name="dot" size={8} /> {t('wifi.connected')}
                      </span>
                    )}
                  </div>
                  <div style={{ fontSize: 11, color: 'var(--text-dim)', marginTop: 2 }}>
                    {isHidden
                      ? t('wifi.hiddenNetwork')
                      : `${t('wifi.signal')}: ${n.signal}% • ${n.security || 'Open'}`}
                  </div>
                </div>
                {selectedSsid === n.ssid && !n.connected && !isHidden && (
                  <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                    {isUnsupported(n.security) ? (
                      <span
                        style={{ fontSize: 11, color: 'var(--text-dim)', maxWidth: 220 }}
                        title={t('wifi.enterpriseUnsupported')}
                      >
                        {t('wifi.enterpriseUnsupported')}
                      </span>
                    ) : (
                      <>
                        <input
                          className="text-input"
                          type="password"
                          placeholder={t('wifi.password')}
                          aria-label={t('wifi.passwordLabel')}
                          value={newPassword}
                          onChange={(e) => setNewPassword(e.target.value)}
                          onClick={(e) => e.stopPropagation()}
                          style={{ width: 140, fontSize: 12 }}
                        />
                        <button
                          className="btn btn-primary"
                          onClick={(e) => {
                            e.stopPropagation();
                            void handleConnect(n.ssid, n.security);
                          }}
                          // An empty password on a secured network can only
                          // fail; don't send the request (and don't overwrite a
                          // saved profile with an open one).
                          disabled={connecting || (requiresPassword(n.security) && !newPassword)}
                          title={
                            requiresPassword(n.security) && !newPassword
                              ? t('wifi.passwordRequired')
                              : undefined
                          }
                          style={{ padding: '4px 10px', fontSize: 12 }}
                        >
                          {connecting ? '...' : t('wifi.connect')}
                        </button>
                      </>
                    )}
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>

      <button
        className="btn btn-secondary"
        onClick={handleRefresh}
        style={{ marginTop: 12, width: '100%' }}
      >
        <Icon name="refresh" size={16} /> {t('wifi.refresh')}
      </button>
    </div>
  );
}
