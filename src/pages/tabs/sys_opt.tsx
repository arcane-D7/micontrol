import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import { useToast } from '../../contexts/ToastContext';

interface SysOptStatus {
  id: string;
  applied: boolean;
}

interface OsTurboStatus {
  enabled: boolean;
  throttled_processes: number;
  power_plan: string;
}

/**
 * S53 — System Optimization tab.
 *
 * Hosts OS Turbo (Windows scheduler-level optimization) plus the curated,
 * SAFE, fully-reversible Windows debloat/telemetry tweak set (cross-checked
 * against WinUtil's tweaks.json + O&O ShutUp10 philosophy):
 * - never touches Windows Update services/tasks, Defender, BitLocker;
 * - scheduled tasks are disabled (never deleted);
 * - every applied tweak stores its previous value for faithful restore;
 * - all changes are re-applied automatically at boot (lib.rs setup).
 */
function SystemOptimizationTab() {
  const toast = useToast();
  const [statuses, setStatuses] = useState<SysOptStatus[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [osTurbo, setOsTurbo] = useState<OsTurboStatus | null>(null);
  const [osTurboBusy, setOsTurboBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      const st = await invoke<SysOptStatus[]>('get_sys_opt_status');
      setStatuses(st);
    } catch (e) {
      console.error('[sys_opt] status load failed:', e);
    }
    try {
      const ot = await invoke<OsTurboStatus>('get_os_turbo');
      setOsTurbo(ot);
    } catch (e) {
      console.error('[sys_opt] os_turbo load failed:', e);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const appliedCount = statuses.filter((s) => s.applied).length;

  const toggle = useCallback(
    async (id: string, next: boolean) => {
      setBusy(id);
      try {
        await invoke('set_sys_opt_tweak', { id, enabled: next });
        setStatuses((prev) => prev.map((s) => (s.id === id ? { ...s, applied: next } : s)));
        toast.addToast(
          `${t(`sysOpt.items.${id}.title` as Parameters<typeof t>[0])}: ${
            next ? t('sysOpt.enabled') : t('sysOpt.disabled')
          }`,
          'success',
        );
      } catch (e) {
        const msg =
          typeof e === 'object' && e !== null && 'message' in e
            ? String((e as { message: unknown }).message)
            : String(e);
        toast.addToast(`${t('sysOpt.applyFailed')}: ${msg}`, 'error');
      } finally {
        setBusy(null);
      }
    },
    [toast],
  );

  const toggleOsTurbo = useCallback(async () => {
    if (!osTurbo) return;
    setOsTurboBusy(true);
    try {
      const next = !osTurbo.enabled;
      const res = await invoke<OsTurboStatus>('set_os_turbo', { enabled: next });
      setOsTurbo(res);
      toast.addToast(next ? t('sysOpt.osTurboEnabled') : t('sysOpt.osTurboDisabled'), 'success');
    } catch (e) {
      const msg =
        typeof e === 'object' && e !== null && 'message' in e
          ? String((e as { message: unknown }).message)
          : String(e);
      toast.addToast(`${t('sysOpt.applyFailed')}: ${msg}`, 'error');
    } finally {
      setOsTurboBusy(false);
    }
  }, [osTurbo, toast]);

  return (
    <>
      <PageHeader
        title={t('sysOpt.title')}
        subtitle={`${t('sysOpt.subtitle')} — ${appliedCount}/${statuses.length} ${t(
          'sysOpt.activeCount',
        )}`}
      />

      {/* Reversible notice */}
      <div className="alert alert-success" style={{ marginBottom: 14 }}>
        🔒 {t('sysOpt.safetyNotice')}
      </div>

      {/* OS Turbo card */}
      <div className="card" style={{ marginBottom: 14 }}>
        <div className="card-title">⚡ {t('sysOpt.osTurboTitle')}</div>
        <p className="text-sm" style={{ color: 'var(--color-text-muted)', marginBottom: 14 }}>
          {t('sysOpt.osTurboDesc')}
        </p>
        <label className="toggle" style={{ padding: '4px 0 14px' }}>
          <span className="toggle-info">
            <span className="toggle-name">{t('sysOpt.osTurboToggle')}</span>
            <span className="toggle-desc">
              {osTurbo
                ? osTurbo.enabled
                  ? `${t('sysOpt.osTurboOn')} — ${t('sysOpt.osTurboPlan')}: ${osTurbo.power_plan}`
                  : t('sysOpt.osTurboOff')
                : t('common.unknown')}
            </span>
          </span>
          <span className="toggle-switch">
            <input
              type="checkbox"
              checked={osTurbo?.enabled ?? false}
              disabled={osTurboBusy}
              onChange={() => void toggleOsTurbo()}
            />
            <span className="toggle-track" />
            <span className="toggle-knob" />
          </span>
        </label>
        <details style={{ marginTop: 4 }}>
          <summary style={{ fontSize: '0.8rem', color: 'var(--text-muted)', cursor: 'pointer' }}>
            ℹ️ {t('sysOpt.osTurboHowTitle')}
          </summary>
          <p
            style={{
              fontSize: '0.8rem',
              color: 'var(--text-muted)',
              marginTop: 8,
              lineHeight: 1.6,
            }}
          >
            {t('sysOpt.osTurboHow')}
          </p>
        </details>
      </div>

      {/* Tweak list — split in two groups: privacy/policies vs app removal */}
      {statuses.length === 0 ? (
        <div className="card">
          <div className="skeleton" style={{ height: 200 }} />
        </div>
      ) : (
        <>
          {statuses
            .filter((s) => !APPX_GROUP_IDS.has(s.id))
            .map((st) => (
              <TweakCard key={st.id} st={st} busy={busy === st.id} onToggle={toggle} />
            ))}

          {/* ── App removal section (tiny11-style) ── */}
          <div className="alert alert-warn" style={{ margin: '14px 0' }}>
            🗑 {t('sysOpt.appRemovalNotice')}
          </div>
          {statuses
            .filter((s) => APPX_GROUP_IDS.has(s.id))
            .map((st) => (
              <TweakCard key={st.id} st={st} busy={busy === st.id} onToggle={toggle} />
            ))}
        </>
      )}
    </>
  );
}

const APPX_GROUP_IDS = new Set([
  'appx_bing_games',
  'appx_office_media',
  'appx_misc_tools',
  'appx_xbox',
  'onedrive_uninstall',
  'services_unused',
]);

/** One toggle card: title + impact stars + desc + why. */
function TweakCard({
  st,
  busy,
  onToggle,
}: {
  st: SysOptStatus;
  busy: boolean;
  onToggle: (id: string, next: boolean) => Promise<void>;
}) {
  return (
    <div className="card" style={{ marginBottom: 10 }}>
      <label className="toggle" style={{ padding: '4px 0' }}>
        <span className="toggle-info">
          <span className="toggle-name">
            {t(`sysOpt.items.${st.id}.title` as Parameters<typeof t>[0])}
            <span
              className="badge warning"
              style={{ marginLeft: 8, fontSize: '0.6rem' }}
              title={t('sysOpt.impact')}
            >
              {'★'.repeat(impactOf(st.id))}
            </span>
          </span>
          <span className="toggle-desc">
            {t(`sysOpt.items.${st.id}.desc` as Parameters<typeof t>[0])}
          </span>
          <span
            className="toggle-desc"
            style={{ display: 'block', color: 'var(--color-text)', marginTop: 4 }}
          >
            ✓ {t(`sysOpt.items.${st.id}.why` as Parameters<typeof t>[0])}
          </span>
        </span>
        <span className="toggle-switch">
          <input
            type="checkbox"
            checked={st.applied}
            disabled={busy}
            onChange={(e) => void onToggle(st.id, e.target.checked)}
          />
          <span className="toggle-track" />
          <span className="toggle-knob" />
        </span>
      </label>
    </div>
  );
}

/** Impact stars per tweak id (matches TWEAKS.impact in sys_opt.rs). */
function impactOf(id: string): number {
  const map: Record<string, number> = {
    telemetry_level: 5,
    diagtrack_service: 4,
    ceip_tasks: 5,
    compat_appraiser: 5,
    advertising_id: 2,
    tailored_experiences: 2,
    activity_history: 3,
    feedback_requests: 2,
    web_search_start: 3,
    consumer_features: 3,
    delivery_opt_upload: 3,
    background_apps: 4,
  };
  return map[id] ?? 2;
}

export default SystemOptimizationTab;
