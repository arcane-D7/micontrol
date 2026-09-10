import { useState, useEffect, useCallback, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import { useToast } from '../../contexts/ToastContext';

interface SysOptStatus {
  id: string;
  applied: boolean;
  /** "green" | "yellow" | "red" — debloater accordion group (S55). */
  tier: string;
}

interface OsTurboStatus {
  enabled: boolean;
  throttled_processes: number;
  power_plan: string;
}

/**
 * S53/S55 — System Optimization tab.
 *
 * Hosts OS Turbo plus the Windows Debloater: a modal catalog of debloat
 * actions (inspired by tiny11builder, Raphire/Win11Debloat and
 * farag2/Sophia-Script-for-Windows) organized in three risk-tier accordions.
 * Every option is a real uninstall/reinstall (or apply/restore) button —
 * never a blind toggle:
 * - GREEN: zero-risk, reversible Store-app removals and privacy policies;
 *   accordion open, buttons enabled.
 * - YELLOW: may break non-essential functionality (Xbox/OneDrive/services);
 *   one consent click unlocks the buttons.
 * - RED: aggressive changes that may destabilize Windows; requires an
 *   explicit damage-acknowledgement checkbox.
 */
function SystemOptimizationTab() {
  const toast = useToast();
  const [statuses, setStatuses] = useState<SysOptStatus[]>([]);
  const [osTurbo, setOsTurbo] = useState<OsTurboStatus | null>(null);
  const [osTurboBusy, setOsTurboBusy] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);

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

  const tiers = useMemo(
    () => ({
      green: statuses.filter((s) => s.tier === 'green'),
      yellow: statuses.filter((s) => s.tier === 'yellow'),
      red: statuses.filter((s) => s.tier === 'red'),
    }),
    [statuses],
  );

  return (
    <>
      <PageHeader
        title={t('sysOpt.title')}
        subtitle={`${t('sysOpt.subtitle')} — ${appliedCount}/${statuses.length} ${t(
          'sysOpt.activeCount',
        )}`}
      />

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

      {/* ── S55: Windows Debloater card ── */}
      <div className="card" style={{ marginBottom: 14 }}>
        <div className="card-title">🧹 {t('sysOpt.debloaterTitle')}</div>
        <p className="text-sm" style={{ color: 'var(--color-text-muted)', lineHeight: 1.6 }}>
          {t('sysOpt.debloaterDesc')}
        </p>
        <p
          className="text-sm"
          style={{ color: 'var(--color-text-dim)', fontSize: '0.78rem', margin: '10px 0 14px' }}
        >
          {t('sysOpt.debloaterInspiration')}
        </p>
        <div style={{ display: 'flex', gap: 10, alignItems: 'center' }}>
          <button className="btn btn-primary" onClick={() => setModalOpen(true)}>
            {t('sysOpt.debloaterOpen')}
          </button>
          <span className="badge" style={{ fontSize: '0.65rem' }}>
            {appliedCount}/{statuses.length} {t('sysOpt.activeCount')}
          </span>
        </div>
      </div>

      {modalOpen && (
        <DebloaterModal
          tiers={tiers}
          onClose={() => setModalOpen(false)}
          onAction={async (id, next) => {
            try {
              await invoke('set_sys_opt_tweak', { id, enabled: next });
              // S55 FIX 7b: re-read the REAL applied state from the backend
              // instead of optimistically flipping the local flag — the
              // operation may have partially succeeded (e.g. appx group with
              // some packages missing), and a stale optimistic state made
              // the buttons look like a toggle ("clico num e o outro some").
              try {
                const fresh = await invoke<SysOptStatus[]>('get_sys_opt_status');
                setStatuses(fresh);
              } catch {
                setStatuses((prev) => prev.map((s) => (s.id === id ? { ...s, applied: next } : s)));
              }
              toast.addToast(
                `${t(`sysOpt.items.${id}.title` as Parameters<typeof t>[0])}: ${
                  next ? t('sysOpt.debloatDone') : t('sysOpt.debloatRestored')
                }`,
                'success',
              );
            } catch (e) {
              const msg =
                typeof e === 'object' && e !== null && 'message' in e
                  ? String((e as { message: unknown }).message)
                  : String(e);
              toast.addToast(`${t('sysOpt.applyFailed')}: ${msg}`, 'error');
            }
          }}
        />
      )}
    </>
  );
}

// ── Debloater modal ─────────────────────────────────────────────────────────

type TierMap = { green: SysOptStatus[]; yellow: SysOptStatus[]; red: SysOptStatus[] };

function DebloaterModal({
  tiers,
  onClose,
  onAction,
}: {
  tiers: TierMap;
  onClose: () => void;
  onAction: (id: string, next: boolean) => Promise<void>;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [yellowUnlocked, setYellowUnlocked] = useState(false);
  const [redUnlocked, setRedUnlocked] = useState(false);
  const [greenOpen, setGreenOpen] = useState(true);
  const [yellowOpen, setYellowOpen] = useState(false);
  const [redOpen, setRedOpen] = useState(false);

  const run = useCallback(
    async (id: string, next: boolean) => {
      setBusy(id);
      try {
        await onAction(id, next);
      } finally {
        setBusy(null);
      }
    },
    [onAction],
  );

  return (
    <div
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 9999,
        background: 'rgba(0,0,0,0.55)',
        backdropFilter: 'blur(4px)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={t('sysOpt.debloaterTitle')}
        style={{
          background: 'var(--color-surface, #1e1e2e)',
          border: '1px solid var(--color-border, rgba(255,255,255,0.08))',
          borderRadius: 12,
          padding: 20,
          width: 'min(720px, 92vw)',
          maxHeight: '82vh',
          overflow: 'auto',
          boxShadow: '0 24px 64px rgba(0,0,0,0.6)',
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
            marginBottom: 14,
          }}
        >
          <div style={{ fontWeight: 600, fontSize: 15 }}>🧹 {t('sysOpt.debloaterTitle')}</div>
          <button
            onClick={onClose}
            aria-label={t('common.close')}
            style={{
              background: 'none',
              border: 'none',
              cursor: 'pointer',
              color: 'var(--color-text-dim)',
              fontSize: 18,
              lineHeight: 1,
              padding: '0 4px',
            }}
          >
            ✕
          </button>
        </div>

        {/* GREEN tier */}
        <Accordion
          open={greenOpen}
          onToggle={() => setGreenOpen((v) => !v)}
          color="#22c55e"
          title={t('sysOpt.tierGreenTitle')}
          subtitle={t('sysOpt.tierGreenDesc')}
          items={tiers.green}
          enabled
          busy={busy}
          onAction={run}
        />

        {/* YELLOW tier */}
        <Accordion
          open={yellowOpen}
          onToggle={() => setYellowOpen((v) => !v)}
          color="#eab308"
          title={t('sysOpt.tierYellowTitle')}
          subtitle={t('sysOpt.tierYellowDesc')}
          items={tiers.yellow}
          enabled={yellowUnlocked}
          busy={busy}
          onAction={run}
        >
          {!yellowUnlocked && (
            <div style={{ marginBottom: 10 }}>
              <button className="btn btn-secondary" onClick={() => setYellowUnlocked(true)}>
                {t('sysOpt.tierYellowConsent')}
              </button>
            </div>
          )}
        </Accordion>

        {/* RED tier */}
        <Accordion
          open={redOpen}
          onToggle={() => setRedOpen((v) => !v)}
          color="#ef4444"
          title={t('sysOpt.tierRedTitle')}
          subtitle={t('sysOpt.tierRedDesc')}
          items={tiers.red}
          enabled={redUnlocked}
          busy={busy}
          onAction={run}
        >
          {!redUnlocked && (
            <label
              className="text-sm"
              style={{
                display: 'flex',
                gap: 8,
                alignItems: 'flex-start',
                cursor: 'pointer',
                marginBottom: 10,
                padding: 10,
                border: '1px solid rgba(239,68,68,0.35)',
                borderRadius: 8,
                background: 'rgba(239,68,68,0.06)',
              }}
            >
              <input
                type="checkbox"
                checked={redUnlocked}
                onChange={(e) => setRedUnlocked(e.target.checked)}
                style={{ marginTop: 3 }}
              />
              <span style={{ lineHeight: 1.5 }}>{t('sysOpt.tierRedAck')}</span>
            </label>
          )}
        </Accordion>
      </div>
    </div>
  );
}

/** One risk-tier accordion with per-item uninstall/reinstall buttons. */
function Accordion({
  open,
  onToggle,
  color,
  title,
  subtitle,
  items,
  enabled,
  busy,
  onAction,
  children,
}: {
  open: boolean;
  onToggle: () => void;
  color: string;
  title: string;
  subtitle: string;
  items: SysOptStatus[];
  enabled: boolean;
  busy: string | null;
  onAction: (id: string, next: boolean) => Promise<void>;
  children?: React.ReactNode;
}) {
  return (
    <div
      style={{
        border: `1px solid ${color}44`,
        borderRadius: 10,
        marginBottom: 12,
        overflow: 'hidden',
      }}
    >
      <button
        onClick={onToggle}
        style={{
          width: '100%',
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          padding: '12px 14px',
          background: `${color}11`,
          border: 'none',
          cursor: 'pointer',
          color: 'var(--color-text)',
          textAlign: 'left',
        }}
      >
        <span style={{ color, fontSize: 14 }}>{open ? '▾' : '▸'}</span>
        <span style={{ color, fontWeight: 600, fontSize: 13 }}>{title}</span>
        <span
          className="text-sm"
          style={{ color: 'var(--color-text-dim)', fontSize: '0.72rem', flex: 1 }}
        >
          {subtitle}
        </span>
        <span className="badge" style={{ fontSize: '0.6rem' }}>
          {items.length}
        </span>
      </button>
      {open && (
        <div style={{ padding: '10px 14px 14px' }}>
          {children}
          {items.map((st) => (
            <DebloaterItem
              key={st.id}
              st={st}
              enabled={enabled}
              busy={busy === st.id}
              onAction={onAction}
            />
          ))}
        </div>
      )}
    </div>
  );
}

/** One catalog entry: description + uninstall/reinstall action buttons. */
function DebloaterItem({
  st,
  enabled,
  busy,
  onAction,
}: {
  st: SysOptStatus;
  enabled: boolean;
  busy: boolean;
  onAction: (id: string, next: boolean) => Promise<void>;
}) {
  const key = `sysOpt.items.${st.id}`;
  return (
    <div
      style={{
        display: 'flex',
        gap: 12,
        alignItems: 'flex-start',
        padding: '10px 0',
        borderBottom: '1px solid var(--color-border, rgba(255,255,255,0.06))',
      }}
    >
      <div style={{ flex: 1 }}>
        <div
          style={{ fontWeight: 600, fontSize: 13, display: 'flex', alignItems: 'center', gap: 8 }}
        >
          {t(`${key}.title` as Parameters<typeof t>[0])}
          {/* S55 FIX 11: explicit applied/removed badge — users misread the
              button states ("everything looks enabled again") when the only
              signal was which button was disabled. */}
          {st.applied ? (
            <span
              className="badge warning"
              style={{ fontSize: '0.58rem', color: '#f59e0b', border: '1px solid #f59e0b55' }}
            >
              {t('sysOpt.stateApplied')}
            </span>
          ) : (
            <span
              className="badge"
              style={{ fontSize: '0.58rem', color: '#22c55e', border: '1px solid #22c55e55' }}
            >
              {t('sysOpt.stateDefault')}
            </span>
          )}
        </div>
        <div className="text-sm" style={{ color: 'var(--color-text-dim)', fontSize: '0.76rem' }}>
          {t(`${key}.desc` as Parameters<typeof t>[0])}
        </div>
        <div className="text-sm" style={{ color: 'var(--color-text-muted)', fontSize: '0.72rem' }}>
          {t(`${key}.why` as Parameters<typeof t>[0])}
        </div>
      </div>
      <div style={{ display: 'flex', gap: 6, flexShrink: 0 }}>
        <button
          className="btn btn-secondary"
          disabled={busy || !enabled || !st.applied}
          style={{ fontSize: '0.72rem', padding: '5px 10px' }}
          title={t('sysOpt.debloatReinstallTitle')}
          onClick={() => void onAction(st.id, false)}
        >
          {busy ? t('sysOpt.debloatWorking') : t('sysOpt.debloatReinstall')}
        </button>
        <button
          className="btn btn-danger"
          disabled={busy || !enabled || st.applied}
          style={{ fontSize: '0.72rem', padding: '5px 10px' }}
          title={t('sysOpt.debloatUninstallTitle')}
          onClick={() => void onAction(st.id, true)}
        >
          {busy ? t('sysOpt.debloatWorking') : t('sysOpt.debloatUninstall')}
        </button>
      </div>
    </div>
  );
}

export default SystemOptimizationTab;
