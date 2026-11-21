import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useToast } from '../contexts/ToastContext';
import { t } from '../hooks/useI18n';
import { Icon } from './ui/icons';

interface CastDevice {
  name: string;
  id: string;
  device_type: string;
  connected: boolean | null;
}

interface CastScanReport {
  duration_ms: number;
  devices: CastDevice[];
  added_events: number;
  updated_events: number;
  completed: boolean;
}

interface CastResult {
  success: boolean;
  message: string;
}

interface CastState {
  active: boolean | null;
  device_name: string | null;
  detail: string;
  last_attempt: string | null;
}

interface CastApiProbe {
  api: string;
  status: string;
  detail: string;
}

interface CastCapabilityReport {
  probed_at_unix_ms: number;
  screen_mirroring_by_api: boolean;
  conclusion: string;
  probes: CastApiProbe[];
}

/** Chip colour maps to state, never to assumption. A coloured (success) chip
 * is only for a device that itself reports being connected; unknown state
 * stays neutral instead of guessing. */
function stateChip(connected: boolean | null): { label: string; color: string } {
  if (connected === true) return { label: t('cast.connectedChip'), color: 'var(--success)' };
  if (connected === false) return { label: t('cast.disconnectedChip'), color: 'var(--text-dim)' };
  return { label: t('cast.unknownChip'), color: 'var(--text-dim)' };
}

export default function ScreenCast() {
  const [devices, setDevices] = useState<CastDevice[]>([]);
  const [selectedId, setSelectedId] = useState<string>('');
  // Which action is running, so each button shows its own progress and the
  // others stay honestly disabled while a backend call is in flight.
  const [busy, setBusy] = useState<
    'idle' | 'known' | 'scan' | 'connect' | 'panel' | 'stop' | 'probe'
  >('idle');
  const [state, setState] = useState<CastState | null>(null);
  const [probe, setProbe] = useState<CastCapabilityReport | null>(null);
  const { addToast } = useToast();

  const refreshState = useCallback(async () => {
    try {
      const s = await invoke<CastState>('get_cast_state');
      setState(s);
    } catch {
      // A state read failure is surfaced through the list errors; the panel
      // must not double-report the same backend problem.
      setState(null);
    }
  }, []);

  useEffect(() => {
    void refreshState();
    void handleRefreshKnown();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshState]);

  // Lists endpoints Windows already knows with real connection state. This
  // does NOT scan the radio: FindAllAsync enumerates AssociationEndpoints
  // Windows has seen before. The copy below keeps saying exactly that.
  const handleRefreshKnown = async () => {
    setBusy('known');
    try {
      const list = await invoke<CastDevice[]>('get_cast_devices');
      setDevices(list);
      // Keep the old selection only if it still exists in the refreshed list.
      setSelectedId((prev) => (list.some((d) => d.id === prev) ? prev : ''));
      if (list.length === 0) {
        addToast({ message: t('cast.noDevices'), type: 'info' });
      }
      await refreshState();
    } catch (e) {
      addToast({
        message: `${t('cast.scanError')}: ${String(e)}`,
        type: 'error',
        onRetry: handleRefreshKnown,
      });
    } finally {
      setBusy('idle');
    }
  };

  // Real discovery: a DeviceWatcher runs over both projection selectors for
  // a bounded window (~6 s). The report says whether enumeration COMPLETED;
  // an expired window means the list may be partial and the copy says so.
  const handleScan = async () => {
    setBusy('scan');
    try {
      const report = await invoke<CastScanReport>('scan_cast_devices');
      const list = report.devices;
      setDevices(list);
      setSelectedId((prev) => (list.some((d) => d.id === prev) ? prev : ''));
      addToast({ message: t('cast.scanDone', { count: list.length }), type: 'info' });
      if (!report.completed) {
        addToast({ message: t('cast.scanIncomplete'), type: 'info' });
      }
      await refreshState();
    } catch (e) {
      addToast({
        message: `${t('cast.scanError')}: ${String(e)}`,
        type: 'error',
        onRetry: handleScan,
      });
    } finally {
      setBusy('idle');
    }
  };

  // Attempts the real connection chain (FromIdAsync, CreateCastingConnection,
  // RequestStartCastingAsync with the only source a non-UI process can build).
  // Windows' own answer is reported verbatim; a refusal is the expected
  // outcome on builds where only media casting exists, and it names Win+K.
  const handleConnect = async () => {
    setBusy('connect');
    try {
      const result = await invoke<CastResult>('start_casting', {
        deviceId: selectedId,
      });
      addToast({
        message: result.message,
        type: result.success ? 'success' : 'error',
      });
      await refreshState();
    } catch (e) {
      addToast({
        message: `${t('cast.connectError')}: ${String(e)}`,
        type: 'error',
        onRetry: handleConnect,
      });
    } finally {
      setBusy('idle');
    }
  };

  // Explicit fallback: the Windows projection panel owns discovery, start
  // and stop of desktop mirroring.
  const handleOpenPanel = async () => {
    setBusy('panel');
    try {
      const result = await invoke<CastResult>('open_cast_panel');
      addToast({
        message: result.message,
        type: result.success ? 'success' : 'error',
      });
    } catch (e) {
      addToast({
        message: `${t('cast.error')}: ${String(e)}`,
        type: 'error',
        onRetry: handleOpenPanel,
      });
    } finally {
      setBusy('idle');
    }
  };

  const handleClosePanel = async () => {
    setBusy('stop');
    try {
      const result = await invoke<CastResult>('stop_casting');
      addToast({ message: result.message, type: 'info' });
      await refreshState();
    } catch (e) {
      addToast({
        message: `${t('cast.error')}: ${String(e)}`,
        type: 'error',
        onRetry: handleClosePanel,
      });
    } finally {
      setBusy('idle');
    }
  };

  // Runs the capability probe: one real API answer per line, cached by the
  // backend. This is the proof panel for WHY start-from-app is unavailable
  // when it is; it never guesses on the UI side.
  const handleProbe = async () => {
    setBusy('probe');
    try {
      const report = await invoke<CastCapabilityReport>('probe_cast_capabilities');
      setProbe(report);
    } catch (e) {
      addToast({
        message: `${t('cast.error')}: ${String(e)}`,
        type: 'error',
        onRetry: handleProbe,
      });
    } finally {
      setBusy('idle');
    }
  };

  const loading = busy !== 'idle';
  const selectedDevice = devices.find((d) => d.id === selectedId) ?? null;

  return (
    <div className="card">
      <h2 className="card-title">
        <Icon name="tv" size={16} /> {t('cast.title')}
      </h2>
      <p className="page-subtitle">{t('cast.subtitle')}</p>

      {state && (
        <div
          style={{
            marginTop: 12,
            padding: 12,
            background: 'var(--bg-hover)',
            borderRadius: 'var(--r-xs)',
            borderLeft: `3px solid ${state.active ? 'var(--success)' : 'var(--text-dim)'}`,
          }}
          aria-live="polite"
        >
          <div style={{ fontSize: 12, color: 'var(--text-dim)', fontWeight: 600 }}>
            {t('cast.stateTitle')}
          </div>
          <div style={{ fontSize: 13, marginTop: 4 }}>
            {state.active === true && state.device_name
              ? t('cast.stateActive', { name: state.device_name })
              : state.active === false
                ? t('cast.stateInactive')
                : t('cast.stateNoEndpoints')}
          </div>
          <div
            style={{
              fontSize: 11,
              color: 'var(--text-dim)',
              marginTop: 4,
              lineHeight: 1.5,
            }}
          >
            {state.detail}
          </div>
          <div style={{ fontSize: 11, color: 'var(--text-dim)', marginTop: 4 }}>
            <strong>{t('cast.lastAttempt')}:</strong> {state.last_attempt ?? t('cast.noAttempt')}
          </div>
        </div>
      )}

      <div style={{ display: 'flex', gap: 8, marginTop: 12 }}>
        <button
          className="btn btn-primary"
          onClick={handleConnect}
          disabled={loading || !selectedDevice}
          style={{ flex: 1 }}
          title={selectedDevice ? selectedDevice.name : undefined}
        >
          <Icon name="upload" size={16} />{' '}
          {busy === 'connect' ? t('cast.connecting') : t('cast.connect')}
        </button>
        <button className="btn btn-secondary" onClick={handleOpenPanel} disabled={loading}>
          <Icon name="search" size={16} /> {t('cast.openPanel')}
        </button>
        <button className="btn btn-secondary" onClick={handleClosePanel} disabled={loading}>
          <Icon name="stop" size={16} /> {t('cast.stop')}
        </button>
      </div>

      <div style={{ display: 'flex', gap: 8, marginTop: 8 }}>
        <button
          className="btn btn-secondary"
          onClick={handleScan}
          disabled={loading}
          style={{ flex: 1 }}
        >
          <Icon name="search" size={16} />{' '}
          {busy === 'scan' ? t('cast.scanning') : t('cast.scanNow')}
        </button>
        <button className="btn btn-secondary" onClick={handleRefreshKnown} disabled={loading}>
          {t('cast.scan')}
        </button>
      </div>

      <p className="text-muted" style={{ fontSize: 12, marginTop: 8 }}>
        {t('cast.knownOnly')}
      </p>

      {devices.length > 0 && (
        <div style={{ marginTop: 12 }}>
          <div style={{ fontWeight: 600, marginBottom: 8, color: 'var(--text-dim)', fontSize: 13 }}>
            {t('cast.availableDevices')}
          </div>
          {devices.map((d) => {
            const chip = stateChip(d.connected);
            return (
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
                <span style={{ fontSize: 11, color: chip.color }}>{chip.label}</span>
                <span style={{ fontSize: 11, color: 'var(--text-dim)' }}>{d.device_type}</span>
              </label>
            );
          })}
        </div>
      )}

      <div
        style={{
          marginTop: 12,
          display: 'flex',
          justifyContent: 'flex-end',
        }}
      >
        <button className="btn btn-secondary" onClick={handleProbe} disabled={loading}>
          {t('cast.probe')}
        </button>
      </div>

      {probe && (
        <div
          style={{
            marginTop: 12,
            padding: 12,
            background: 'var(--bg-hover)',
            borderRadius: 'var(--r-xs)',
          }}
        >
          <div style={{ fontSize: 12, color: 'var(--text-dim)', fontWeight: 600, marginBottom: 6 }}>
            {t('cast.probeReport')}
          </div>
          <div style={{ fontSize: 12, lineHeight: 1.6, marginBottom: 8 }}>{probe.conclusion}</div>
          {probe.probes.map((p) => (
            <div key={p.api} style={{ fontSize: 11, color: 'var(--text-dim)', marginTop: 4 }}>
              <strong>{p.status}</strong> - {p.api}: {p.detail}
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
          <strong>{t('cast.panelNoteTitle')}:</strong> {t('cast.panelFallbackNote')}
        </div>
      </div>
    </div>
  );
}
