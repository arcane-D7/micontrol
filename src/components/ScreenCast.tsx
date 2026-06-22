import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useToast } from '../contexts/ToastContext';
import { t } from '../hooks/useI18n';

interface CastDevice {
  name: string;
  id: string;
  device_type: string;
}

interface CastResult {
  success: boolean;
  message: string;
}

export default function ScreenCast() {
  const [devices, setDevices] = useState<CastDevice[]>([]);
  const [casting, setCasting] = useState(false);
  const [loading, setLoading] = useState(false);
  const { addToast } = useToast();

  const handleScan = async () => {
    setLoading(true);
    try {
      const list = await invoke<CastDevice[]>('get_cast_devices');
      setDevices(list);
      if (list.length === 0) {
        addToast(t('cast.noDevices'), 'info');
      }
    } catch (e) {
      addToast(`${t('cast.scanError')}: ${String(e)}`, 'error');
    } finally {
      setLoading(false);
    }
  };

  const handleStartCast = async () => {
    setLoading(true);
    try {
      const result = await invoke<CastResult>('start_casting', { deviceId: '' });
      if (result.success) {
        setCasting(true);
        addToast(t('cast.panelOpened'), 'success');
      } else {
        addToast(result.message, 'error');
      }
    } catch (e) {
      addToast(`${t('cast.error')}: ${String(e)}`, 'error');
    } finally {
      setLoading(false);
    }
  };

  const handleStopCast = async () => {
    setLoading(true);
    try {
      const result = await invoke<CastResult>('stop_casting');
      if (result.success) {
        setCasting(false);
        addToast(t('cast.stopped'), 'info');
      }
    } catch (e) {
      addToast(`${t('cast.error')}: ${String(e)}`, 'error');
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="card">
      <div className="card-title">📺 {t('cast.title')}</div>
      <p className="page-subtitle">{t('cast.subtitle')}</p>

      <div style={{ display: 'flex', gap: 8, marginTop: 12 }}>
        <button
          className="btn btn-primary"
          onClick={handleStartCast}
          disabled={loading || casting}
          style={{ flex: 1 }}
        >
          {casting ? `🟢 ${t('cast.casting')}` : `📤 ${t('cast.start')}`}
        </button>
        {casting && (
          <button className="btn btn-secondary" onClick={handleStopCast} disabled={loading}>
            ⏹ {t('cast.stop')}
          </button>
        )}
      </div>

      <button
        className="btn btn-secondary"
        onClick={handleScan}
        disabled={loading}
        style={{ marginTop: 8, width: '100%' }}
      >
        🔍 {t('cast.scan')}
      </button>

      {devices.length > 0 && (
        <div style={{ marginTop: 12 }}>
          <div style={{ fontWeight: 600, marginBottom: 8, color: 'var(--text-dim)', fontSize: 13 }}>
            {t('cast.availableDevices')}
          </div>
          {devices.map((d) => (
            <div key={d.id} className="stat-row" style={{ padding: '6px 8px', marginBottom: 4 }}>
              <span style={{ flex: 1, fontSize: 13 }}>{d.name}</span>
              <span style={{ fontSize: 11, color: 'var(--text-dim)' }}>{d.device_type}</span>
            </div>
          ))}
        </div>
      )}

      <div
        style={{
          marginTop: 16,
          padding: 12,
          background: 'var(--bg-hover)',
          borderRadius: 'var(--r-xs)',
        }}
      >
        <div style={{ fontSize: 12, color: 'var(--text-dim)', lineHeight: 1.6 }}>
          <strong>{t('cast.tip')}:</strong> {t('cast.tipText')}
        </div>
      </div>
    </div>
  );
}
