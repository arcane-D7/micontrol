import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { IotDeviceInfo } from '../types/hardware';

/** All 16 EC commands available via the EC command protocol. */
const EC_COMMANDS: { id: string; name: string; desc: string }[] = [
  { id: '0x01', name: 'GetBindStatus', desc: 'Check if the IoT chip is bound to Xiaomi cloud' },
  { id: '0x02', name: 'SetBindStatus', desc: 'Set the cloud binding state on the chip' },
  { id: '0x03', name: 'ResetDevice', desc: 'Factory-reset the IoT chip (clears binding & WiFi)' },
  { id: '0x04', name: 'WriteWiFiItem', desc: 'Provision a WiFi network onto the chip' },
  { id: '0x05', name: 'EmptyWiFiItems', desc: 'Remove all saved WiFi networks from the chip' },
  { id: '0x06', name: 'DeleteWiFiItem', desc: 'Remove a specific saved WiFi network' },
  { id: '0x07', name: 'ReadWiFiStatus', desc: "Query the chip's WiFi connection status & SSID" },
  { id: '0x08', name: 'ReadWiFiCount', desc: 'Count of WiFi networks saved on the chip' },
  { id: '0x09', name: 'GetWiFiByIndex', desc: 'Retrieve a saved WiFi network by index' },
  { id: '0x0A', name: 'GetFwVersion', desc: 'Read the IoT chip firmware version' },
  { id: '0x0B', name: 'GetModel', desc: 'Read the IoT chip model identifier' },
  { id: '0x0C', name: 'ConnectWiFi', desc: 'Trigger the chip to connect to a saved WiFi network' },
  { id: '0x0D', name: 'GetDeviceID', desc: 'Read the unique device identifier (DID)' },
  {
    id: '0x0E',
    name: 'SendLaptopStatus (SUSPEND)',
    desc: 'Notify chip that the laptop is suspending',
  },
  {
    id: '0x0F',
    name: 'SendLaptopStatus (SHUTDOWN)',
    desc: 'Notify chip that the laptop is shutting down',
  },
  { id: '0x10', name: 'SendLaptopStatus (WIN_READY)', desc: 'Notify chip that Windows has booted' },
];

export default function IotDeviceCard() {
  const [info, setInfo] = useState<IotDeviceInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [retryCount, setRetryCount] = useState(0);
  const [ensuringService, setEnsuringService] = useState(false);
  const [showCommands, setShowCommands] = useState(false);

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

      {/* ── Device Info ─────────────────────────────────────────── */}
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
            <span className="stat-label">Cloud Binding</span>
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
            <span className="stat-label">IoT WiFi</span>
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
          ℹ️ About this device
        </div>
        <p style={{ margin: '0 0 8px 0' }}>
          <strong>Cloud Binding</strong> — Indicates whether the IoT chip is registered with the
          Xiaomi IoT cloud service. This is <em>not</em> related to Mi Home or any phone app. When
          bound, the chip can receive remote commands (e.g. wake-on-LAN, status queries) while the
          laptop is powered off or in sleep mode.
        </p>
        <p style={{ margin: '0 0 8px 0' }}>
          <strong>IoT WiFi</strong> — This is the IoT chip&apos;s own WiFi module, which is
          completely separate from your Windows WiFi. The chip uses this WiFi to maintain a cloud
          connection for remote management when the laptop is asleep or powered off. If the chip is
          not bound to the cloud, IoT WiFi will show &quot;Not connected&quot; — this is expected.
        </p>
        <p style={{ margin: 0 }}>
          <strong>Device Status</strong> — Queried via WMI (not EC commands). Reflects the ACPI
          power state of the IoT device as reported by Windows.
        </p>
      </div>

      {/* ── EC Command Protocol ─────────────────────────────────── */}
      <div style={{ marginTop: 12 }}>
        <button
          className="btn btn-secondary"
          onClick={() => setShowCommands((s) => !s)}
          style={{ width: '100%', fontSize: 12 }}
        >
          {showCommands ? '▼' : '▶'} EC Command Protocol (16 commands)
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
                  <th style={{ padding: '6px 8px', width: 48 }}>ID</th>
                  <th style={{ padding: '6px 8px' }}>Command</th>
                  <th style={{ padding: '6px 8px' }}>Description</th>
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
                    <td style={{ padding: '4px 8px', color: 'var(--text-muted)' }}>{cmd.desc}</td>
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
          Commands are sent via a 4-phase state machine over EC RAM (status register{' '}
          <code>0xFE0B0F00</code>, command register <code>0xFE0B0F01</code>, sensor data buffer{' '}
          <code>0xFE0B0F08</code>). See <code>docs/EC_COMMAND_PROTOCOL_RE.md</code> for full
          protocol details.
        </p>
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
