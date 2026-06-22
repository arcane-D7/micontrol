import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface IotDeviceInfo {
  pipe_available: boolean;
  model: string | null;
  fw_version: string | null;
  bind_status: { bound: boolean; uid: number | null } | null;
  device_id: number | null;
  device_status: string | null;
  wifi_status: { wifi_status: number; ssid: string | null } | null;
  wifi_network_count: number | null;
}

export default function IotDeviceCard() {
  const [info, setInfo] = useState<IotDeviceInfo | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    void loadInfo();
  }, []);

  const loadInfo = async () => {
    try {
      const data = await invoke<IotDeviceInfo>('get_iot_device_info');
      setInfo(data);
    } catch (e) {
      console.error('Failed to load IoT device info:', e);
    } finally {
      setLoading(false);
    }
  };

  if (loading) {
    return (
      <div className="card">
        <div className="card-title">🔌 IoT Device</div>
        <p className="page-subtitle">Loading device information...</p>
      </div>
    );
  }

  if (!info?.pipe_available) {
    return (
      <div className="card">
        <div className="card-title">🔌 IoT Device</div>
        <p className="page-subtitle" style={{ color: 'var(--text-dim)' }}>
          IoT Service not available. The Xiaomi IoT chip was not detected on this system.
        </p>
      </div>
    );
  }

  return (
    <div className="card">
      <div className="card-title">🔌 IoT Device</div>
      <p className="page-subtitle">Xiaomi IoT chip information</p>

      <div style={{ marginTop: 12 }}>
        {info.model && (
          <div className="stat-row">
            <span className="stat-label">Model</span>
            <span className="stat-value">{info.model}</span>
          </div>
        )}
        {info.fw_version && (
          <div className="stat-row">
            <span className="stat-label">Firmware</span>
            <span className="stat-value">{info.fw_version}</span>
          </div>
        )}
        {info.device_id !== null && (
          <div className="stat-row">
            <span className="stat-label">Device ID</span>
            <span className="stat-value">{info.device_id}</span>
          </div>
        )}
        {info.device_status && (
          <div className="stat-row">
            <span className="stat-label">Status</span>
            <span className="stat-value">{info.device_status}</span>
          </div>
        )}
        {info.bind_status && (
          <div className="stat-row">
            <span className="stat-label">Account Binding</span>
            <span
              className="stat-value"
              style={{ color: info.bind_status.bound ? 'var(--success)' : 'var(--text-dim)' }}
            >
              {info.bind_status.bound ? `✓ Bound (UID: ${info.bind_status.uid})` : 'Not bound'}
            </span>
          </div>
        )}
        {info.wifi_status && (
          <div className="stat-row">
            <span className="stat-label">WiFi</span>
            <span className="stat-value">{info.wifi_status.ssid || 'Not connected'}</span>
          </div>
        )}
        {info.wifi_network_count !== null && (
          <div className="stat-row">
            <span className="stat-label">Saved Networks</span>
            <span className="stat-value">{info.wifi_network_count}</span>
          </div>
        )}
      </div>

      <button
        className="btn btn-secondary"
        onClick={loadInfo}
        style={{ marginTop: 12, width: '100%' }}
      >
        🔄 Refresh
      </button>
    </div>
  );
}
