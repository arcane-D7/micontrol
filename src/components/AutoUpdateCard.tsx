import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { t } from '../hooks/useI18n';
import ToggleSwitch from './ToggleSwitch';

interface AutoUpdateConfig {
  enabled: boolean;
  betaFeedUrl: string;
}

type UpdateState = 'idle' | 'updating' | 'success' | 'error';

/**
 * S45-002: Auto-update system.
 *
 * - Master toggle "Auto-update" — **OFF by default**; the user can turn it on
 *   in Settings so future releases install themselves silently via the
 *   privileged bridge (same "Ponte Elevada" path as BridgeUpdateCard).
 * - Hidden dev feed: a suppressed input (only shown when the card is
 *   "unlocked" by clicking the version mark 7×, or reachable via CLI/MCP with
 *   `set_auto_update_beta_feed`) that points the pipeline at a specific beta
 *   installer URL. The installer then does everything by itself.
 * - "Update now" — fetch + apply using the configured feed (or a pasted URL).
 */
export default function AutoUpdateCard() {
  const [config, setConfig] = useState<AutoUpdateConfig>({ enabled: false, betaFeedUrl: '' });
  const [feedDraft, setFeedDraft] = useState('');
  const [showDevFeed, setShowDevFeed] = useState(false);
  const [state, setState] = useState<UpdateState>('idle');
  const [message, setMessage] = useState('');

  // Load persisted config (registry) on mount.
  useEffect(() => {
    invoke<AutoUpdateConfig>('get_auto_update_config')
      .then((cfg) => {
        setConfig(cfg);
        setFeedDraft(cfg.betaFeedUrl);
        if (cfg.betaFeedUrl) setShowDevFeed(true);
      })
      .catch(() => {
        // Registry not readable — keep default (off).
      });
  }, []);

  const persistEnabled = async (enabled: boolean) => {
    setConfig((c) => ({ ...c, enabled }));
    try {
      await invoke('set_auto_update_enabled', { enabled });
    } catch {
      setConfig((c) => ({ ...c, enabled: !enabled }));
    }
  };

  const saveBetaFeed = async () => {
    const next = feedDraft.trim();
    try {
      await invoke('set_auto_update_beta_feed', { url: next });
      setConfig((c) => ({ ...c, betaFeedUrl: next }));
      setState('success');
      setMessage(next ? t('about.feedSaved') : t('about.feedCleared'));
    } catch {
      setState('error');
      setMessage(t('about.feedSaveFailed'));
    }
  };

  const updateNow = async () => {
    setState('updating');
    setMessage('');
    try {
      const result = await invoke<{ launched?: boolean; note?: string }>('trigger_auto_update', {
        installerUrl: config.betaFeedUrl || null,
      });
      setState('success');
      setMessage(result?.note ?? t('about.updateNowSuccess'));
    } catch (e) {
      setState('error');
      setMessage(
        typeof e === 'object' && e !== null && 'message' in e
          ? String((e as { message: unknown }).message)
          : String(e),
      );
    }
  };

  const busy = state === 'updating';

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
            🔄 {t('about.autoUpdateTitle')}
          </div>
          <p style={{ fontSize: '0.85rem', color: 'var(--text-dim)', margin: 0 }}>
            {t('about.autoUpdateDesc')}
          </p>
        </div>
        <ToggleSwitch
          checked={config.enabled}
          onChange={persistEnabled}
          ariaLabel={t('about.autoUpdateTitle')}
        />
      </div>

      {/* Hidden dev feed — only visible when unlocked by the user/dev. */}
      {showDevFeed && (
        <div style={{ marginTop: 12, paddingTop: 10, borderTop: '1px solid var(--border)' }}>
          <div style={{ fontSize: '0.8rem', color: 'var(--text-dim)', marginBottom: 6 }}>
            🧪 {t('about.feedLabel')}
          </div>
          <div style={{ display: 'flex', gap: 8 }}>
            <input
              type="text"
              value={feedDraft}
              onChange={(e) => setFeedDraft(e.target.value)}
              placeholder={t('about.feedPlaceholder')}
              style={{ flex: 1, minWidth: 0 }}
              aria-label={t('about.feedPlaceholder')}
            />
            <button
              className="btn-secondary"
              onClick={saveBetaFeed}
              style={{ fontSize: 12, whiteSpace: 'nowrap' }}
            >
              {t('about.feedSave')}
            </button>
            <button
              className="btn-primary"
              onClick={updateNow}
              disabled={busy}
              style={{ fontSize: 12, padding: '8px 16px', whiteSpace: 'nowrap' }}
            >
              {busy ? t('about.updateNowInstalling') : t('about.updateNow')}
            </button>
          </div>
          <p style={{ fontSize: '0.75rem', color: 'var(--text-dim)', marginTop: 6 }}>
            {t('about.feedHint')}
          </p>
        </div>
      )}

      {/* Dev unlock — 7 clicks on the title reveals the beta feed input. */}
      <div
        style={{
          fontSize: '0.7rem',
          color: 'transparent',
          userSelect: 'none',
          cursor: 'default',
          marginTop: 10,
        }}
        onClick={(e) => {
          const n = Number(e.currentTarget.dataset.clicks ?? '0') + 1;
          e.currentTarget.dataset.clicks = String(n);
          if (n >= 7) {
            setShowDevFeed(true);
            e.currentTarget.dataset.clicks = '0';
          }
        }}
        aria-hidden
      >
        dev
      </div>

      {state === 'updating' && (
        <div style={{ marginTop: 10, fontSize: '0.85rem', color: 'var(--text-dim)' }}>
          <span className="loading-spinner" role="status" aria-live="polite" />{' '}
          {t('about.updateNowInstalling')}
        </div>
      )}

      {state === 'success' && (
        <div className="alert alert-success" style={{ marginTop: 10 }}>
          ✓ {message}
        </div>
      )}

      {state === 'error' && (
        <div className="alert alert-error" style={{ marginTop: 10 }}>
          ⚠ {message}
        </div>
      )}
    </div>
  );
}
