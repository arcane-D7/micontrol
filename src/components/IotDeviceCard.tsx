import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { IotDeviceInfo } from '../types/hardware';

export default function IotDeviceCard() {
  const [info, setInfo] = useState<IotDeviceInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [retryCount, setRetryCount] = useState(0);
  const [ensuringService, setEnsuringService] = useState(false);

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

  // When pipe is not available, try to start the ecram_service bridge
  // automatically, then retry.
  useEffect(() => {
    if (info?.pipe_available === false && !ensuringService) {
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
  }, [info?.pipe_available, ensuringService, loadInfo]);

  // Auto-retry every 5 seconds when pipe is not available
  useEffect(() => {
    if (info?.pipe_available === false) {
      const timer = setTimeout(() => {
        setRetryCount((c) => c + 1);
        void loadInfo();
      }, 5000);
      return () => clearTimeout(timer);
    }
  }, [info?.pipe_available, retryCount, loadInfo]);

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
        <div style={{ marginTop: 12, fontSize: 12, color: 'var(--text-muted)' }}>
          <div>
            Expected pipe: <code>{'\\\\.\\pipe\\LOCAL\\IoTService_IPC_Broker'}</code>
          </div>
          <div style={{ marginTop: 4 }}>
            Status: Not found
            {retryCount > 0 && ` (retry ${retryCount}...)`}
          </div>
          <div style={{ marginTop: 8, lineHeight: 1.5 }}>
            {ensuringService
              ? 'Starting IoT bridge service...'
              : 'The IoT bridge service is not running. It will be started automatically and the system will retry every 5 seconds.'}
          </div>
        </div>
        <button
          className="btn btn-secondary"
          style={{ marginTop: 12, width: '100%' }}
          onClick={() => void loadInfo()}
        >
          🔄 Refresh now
        </button>
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
