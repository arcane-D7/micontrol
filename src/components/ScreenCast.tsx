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
  const [selectedId, setSelectedId] = useState<string>('');
  const [panelOpened, setPanelOpened] = useState(false);
  const [loading, setLoading] = useState(false);
  const { addToast } = useToast();

  // Lists the displays Windows already knows about. This is NOT discovery:
  // FindAllAsync only enumerates AssociationEndpoints Windows has seen before,
  // so an empty list means "nothing paired yet", not "nothing in range". The
  // copy below says exactly that instead of promising a scan.
  const handleRefreshKnown = async () => {
    setLoading(true);
    try {
      const list = await invoke<CastDevice[]>('get_cast_devices');
      setDevices(list);
      // Keep the old selection only if it still exists in the refreshed list.
      setSelectedId((prev) => (list.some((d) => d.id === prev) ? prev : ''));
      if (list.length === 0) {
        addToast({ message: t('cast.noDevices'), type: 'info' });
      }
    } catch (e) {
      addToast({
        message: `${t('cast.scanError')}: ${String(e)}`,
        type: 'error',
        onRetry: handleRefreshKnown,
      });
    } finally {
      setLoading(false);
    }
  };

  const handleOpenPanel = async () => {
    // With a known display selected the ID is validated first; with none the
    // panel still opens and the user picks the sink inside Windows.
    const deviceId = selectedId || '';
    setLoading(true);
    try {
      const result = await invoke<CastResult>('start_casting', { deviceId });
      if (result.success) {
        setPanelOpened(true);
        addToast({ message: t('cast.panelOpened'), type: 'success' });
      } else {
        addToast({ message: result.message, type: 'error' });
      }
    } catch (e) {
      addToast({
        message: `${t('cast.error')}: ${String(e)}`,
        type: 'error',
        onRetry: handleOpenPanel,
      });
    } finally {
      setLoading(false);
    }
  };

  const handleClosePanel = async () => {
    setLoading(true);
    try {
      const result = await invoke<CastResult>('stop_casting');
      // `success` is only true when a Connect panel was actually closed — the
      // backend no longer hardcodes it. Show whatever it reports.
      if (result.success) {
        setPanelOpened(false);
        addToast({ message: t('cast.stopped'), type: 'info' });
      } else {
        addToast({ message: result.message, type: 'info' });
      }
    } catch (e) {
      addToast({
        message: `${t('cast.error')}: ${String(e)}`,
        type: 'error',
        onRetry: handleClosePanel,
      });
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="card">
      <h2 className="card-title">📺 {t('cast.title')}</h2>
      <p className="page-subtitle">{t('cast.subtitle')}</p>

      <div style={{ display: 'flex', gap: 8, marginTop: 12 }}>
        <button
          className="btn btn-primary"
          onClick={handleOpenPanel}
          disabled={loading}
          style={{ flex: 1 }}
        >
          📤 {t('cast.start')}
        </button>
        {panelOpened && (
          <button className="btn btn-secondary" onClick={handleClosePanel} disabled={loading}>
            ⏹ {t('cast.stop')}
          </button>
        )}
      </div>

      <button
        className="btn btn-secondary"
        onClick={handleRefreshKnown}
        disabled={loading}
        style={{ marginTop: 8, width: '100%' }}
      >
        🔍 {t('cast.scan')}
      </button>

      <p className="text-muted" style={{ fontSize: 12, marginTop: 8 }}>
        {t('cast.knownOnly')}
      </p>

      {devices.length > 0 && (
        <div style={{ marginTop: 12 }}>
          <div style={{ fontWeight: 600, marginBottom: 8, color: 'var(--text-dim)', fontSize: 13 }}>
            {t('cast.availableDevices')}
          </div>
          {devices.map((d) => (
            <label
              key={d.id}
              className="stat-row"
              style={{
                padding: '6px 8px',
                marginBottom: 4,
                cursor: 'pointer',
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                borderRadius: 'var(--r-sm)',
                border: selectedId === d.id ? '1px solid var(--accent)' : '1px solid transparent',
                background: selectedId === d.id ? 'var(--surface-2)' : 'transparent',
              }}
            >
              <input
                type="radio"
                name="cast-device"
                checked={selectedId === d.id}
                onChange={() => setSelectedId(d.id)}
                style={{ accentColor: 'var(--accent)', margin: 0 }}
              />
              <span style={{ flex: 1, fontSize: 13 }}>{d.name}</span>
              <span style={{ fontSize: 11, color: 'var(--text-dim)' }}>{d.device_type}</span>
            </label>
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
          <strong>{t('cast.panelNoteTitle')}:</strong> {t('cast.panelNote')}
        </div>
      </div>
    </div>
  );
}
