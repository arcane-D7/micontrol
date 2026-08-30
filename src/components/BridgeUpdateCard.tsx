import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { t } from '../hooks/useI18n';

type BridgeState = 'idle' | 'installing' | 'success' | 'error';

/**
 * S45-001: "Ponte Elevada" — silent self-update via the privileged
 * MiControlBridge service (runs as SYSTEM, no UAC).
 *
 * The user pastes the path to a freshly built / downloaded
 * `MiControl_<version>-setup.exe` and clicks "Install via Bridge".
 * The path is validated on the backend (must exist + end in .exe) and relayed
 * through the bridge pipe to the SYSTEM service, which launches the NSIS
 * installer silently (`/S /UPDATE /R`). The installer replaces the app,
 * re-creates the bridge service (NSIS POSTINSTALL hook) and relaunches
 * MiControl for the logged-on user — all without any elevation prompt.
 */
export default function BridgeUpdateCard() {
  const [path, setPath] = useState('');
  const [state, setState] = useState<BridgeState>('idle');
  const [message, setMessage] = useState('');

  const handleInstall = async () => {
    const trimmed = path.trim();
    if (!trimmed) {
      setState('error');
      setMessage(t('about.bridgePathMissing'));
      return;
    }

    setState('installing');
    setMessage('');
    try {
      const result = await invoke<{ launched?: boolean; note?: string }>('install_update', {
        installerPath: trimmed,
      });
      // The bridge launched the installer (fire-and-forget). Success means
      // the setup is now running as SYSTEM — the app will restart shortly.
      setState('success');
      setMessage(result?.note ?? t('about.bridgeSuccess'));
    } catch (e) {
      setState('error');
      const msg =
        typeof e === 'object' && e !== null && 'message' in e
          ? String((e as { message: unknown }).message)
          : String(e);
      // The bridge may have fallen back to UAC — still surfacing the raw error
      // from the backend; tell the user what to expect.
      setMessage(`${msg}`);
    }
  };

  const busy = state === 'installing';

  return (
    <div className="card" style={{ marginTop: 16 }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'flex-start',
          justifyContent: 'space-between',
          gap: 16,
        }}
      >
        <div style={{ flex: 1, minWidth: 0 }}>
          <div className="card-title" style={{ margin: 0, marginBottom: 6 }}>
            🌉 {t('about.bridgeTitle')}
          </div>
          <p style={{ fontSize: '0.85rem', color: 'var(--text-dim)', margin: 0 }}>
            {t('about.bridgeDesc')}
          </p>
        </div>
      </div>

      <div style={{ display: 'flex', gap: 8, marginTop: 12 }}>
        <input
          type="text"
          value={path}
          onChange={(e) => setPath(e.target.value)}
          placeholder={t('about.bridgePathPlaceholder')}
          disabled={busy}
          style={{ flex: 1, minWidth: 0 }}
          aria-label={t('about.bridgePathPlaceholder')}
        />
        <button
          className="btn-primary"
          onClick={handleInstall}
          disabled={busy}
          style={{ fontSize: 12, padding: '8px 16px', whiteSpace: 'nowrap' }}
        >
          {busy ? t('about.bridgeInstalling') : t('about.bridgeInstall')}
        </button>
      </div>

      {state === 'installing' && (
        <div style={{ marginTop: 10, fontSize: '0.85rem', color: 'var(--text-dim)' }}>
          <span className="loading-spinner" role="status" aria-live="polite" />{' '}
          {t('about.bridgeInstalling')}
        </div>
      )}

      {state === 'success' && (
        <div
          className="alert alert-success"
          style={{
            marginTop: 10,
            borderColor: 'oklch(from var(--success) l c h / 0.4)',
            background: 'oklch(from var(--success) l c h / 0.06)',
          }}
        >
          ✓ {message}
        </div>
      )}

      {state === 'error' && (
        <div className="alert alert-error" style={{ marginTop: 10 }}>
          ⚠ {message}
        </div>
      )}

      <details style={{ marginTop: 12 }}>
        <summary style={{ fontSize: '0.8rem', color: 'var(--text-dim)', cursor: 'pointer' }}>
          ℹ️ {t('about.bridgeHowTitle')}
        </summary>
        <p style={{ fontSize: '0.8rem', color: 'var(--text-dim)', marginTop: 8, lineHeight: 1.5 }}>
          {t('about.bridgeHow')}
        </p>
      </details>
    </div>
  );
}
