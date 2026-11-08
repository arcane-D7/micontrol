import { useState, useEffect, useRef, useCallback, memo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { t } from '../../hooks/useI18n';
import { PageHeader } from './PageHeader';
import PerformanceMonitor from '../../components/PerformanceMonitor';
import PerformanceModeSelector from '../../components/PerformanceModeSelector';
import FunctionExplain from '../../components/FunctionExplain';
import { useToast } from '../../contexts/ToastContext';
import {
  OsTurboMeasuredBlock,
  osTurboCounts,
  type OsTurboMeasuredData,
} from '../../components/OsTurboMeasured';
import type { Hardware, AiSettings, PerfDebugInfo } from './shared';
import type { PowerProfilesController } from '../../hooks/usePowerProfiles';

interface Props {
  hw: Hardware;
  ai: AiSettings;
  /**
   * Power-profiles controller owned by the app shell (`MainWindow`).
   *
   * The AC/DC transition loop used to live in this component and died with the
   * tab (`key={activeTab}` remounts). The tab is now a pure subscriber: it
   * renders the persisted toggle and the backend's last application, and never
   * owns a poll or an auto-switch effect.
   */
  pp: PowerProfilesController;
  onOpenSettings: () => void;
}

function PerformanceTab({ hw, ai, pp, onOpenSettings }: Props) {
  const { addToast } = useToast();
  const aiApiKeySet = !!ai.settings.openai_api_key;
  const isAiMode = hw.performanceMode === 'smart' || hw.performanceMode === 'smart_acceleration';

  // ── Background logger ────────────────────────────────────────────────────
  const fanRef = useRef(hw.fan);
  const sysRef = useRef(hw.systemInfo);
  const modeRef = useRef(hw.performanceMode);
  fanRef.current = hw.fan;
  sysRef.current = hw.systemInfo;
  modeRef.current = hw.performanceMode;

  useEffect(() => {
    if (!isAiMode || !aiApiKeySet) return;
    const writeEntry = () => {
      const f = fanRef.current;
      const s = sysRef.current;
      if (!f && !s) return;
      const entry = {
        ts: new Date().toISOString().replace('T', ' ').slice(0, 19),
        mode: modeRef.current,
        cpu_temp: f?.cpu_temp_celsius ?? 0,
        gpu_temp: f?.gpu_temp_celsius ?? 0,
        tdp_watts: f?.tdp_watts ?? null,
        cpu_pct: s?.cpu_usage ?? 0,
        gpu_pct: s?.gpu_usage ?? 0,
        note: null,
      };
      void hw.writeAiPerfLog(entry);
    };
    writeEntry();
    const id = setInterval(writeEntry, 30_000);
    return () => clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isAiMode, aiApiKeySet]);

  // ── Log viewer state ──────────────────────────────────────────────────────
  const [showLogs, setShowLogs] = useState(false);
  const [logEntries, setLogEntries] = useState<import('../../hooks/useHardware').AiPerfLogEntry[]>(
    [],
  );
  const [loadingLogs, setLoadingLogs] = useState(false);

  // ── Perf channel debug ─────────────────────────────────────────────────────
  const [debugInfo, setDebugInfo] = useState<PerfDebugInfo | null>(null);
  const [loadingDebug, setLoadingDebug] = useState(false);

  const runPerfDebug = useCallback(async () => {
    setLoadingDebug(true);
    try {
      const info = await invoke<PerfDebugInfo>('get_perf_debug');
      setDebugInfo(info);
    } catch (e) {
      setDebugInfo(null);
      console.error('get_perf_debug failed', e);
      addToast({ message: t('performance.error'), type: 'error' });
    } finally {
      setLoadingDebug(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const loadLogs = useCallback(async () => {
    setLoadingLogs(true);
    try {
      const entries = await hw.readAiPerfLogs(50);
      setLogEntries(entries);
    } catch {
      /* non-fatal */
    } finally {
      setLoadingLogs(false);
    }
  }, [hw]);

  useEffect(() => {
    if (showLogs) void loadLogs();
  }, [showLogs, loadLogs]);

  // The backend reports the mode as its serialized id (`long_battery`, …);
  // the status line must show the same label as the selects above it.
  const modeLabel = (mode: string | null | undefined): string => {
    if (!mode) return '—';
    const keys: Record<string, string> = {
      silence: 'performance.modes.silence',
      long_battery: 'performance.modes.longBattery',
      balance: 'performance.modes.balance',
      turbo: 'performance.modes.turbo',
      decepticon: 'performance.modes.decepticon',
      overdrive: 'performance.modes.overdrive',
      overdrive_high: 'performance.modes.overdriveHigh',
      overdrive_max: 'performance.modes.overdriveMax',
      smart_adaptive: 'performance.modes.smartAdaptive',
      smart: 'performance.modes.smart',
      smart_acceleration: 'performance.modes.smartAcceleration',
    };
    const key = keys[mode];
    return key ? t(key as Parameters<typeof t>[0]) : mode;
  };

  return (
    <>
      <PageHeader title={t('performance.title')} subtitle={t('performance.subtitle')} />
      <PerformanceMonitor
        fan={hw.fan}
        systemInfo={hw.systemInfo}
        currentMode={hw.performanceMode}
        lastResult={hw.lastPerfResult}
      />
      <div className="card">
        <PerformanceModeSelector
          current={hw.performanceMode}
          onChange={hw.setPerformanceMode}
          disabled={hw.loading}
          applying={hw.perfApplying}
          aiApiKeySet={aiApiKeySet}
          onOpenSettings={onOpenSettings}
        />
      </div>

      {/* OS Turbo Card */}
      <OsTurboCard />

      {/* Power Profiles — config + backend status, no local poll loop */}
      <div className="card">
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            marginBottom: 4,
          }}
        >
          <div>
            <div className="card-title" style={{ marginBottom: 2 }}>
              {t('performance.powerProfiles.title')}
            </div>
            <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>
              {t('performance.powerProfiles.subtitle')}
            </div>
          </div>
          <label className="toggle-switch">
            <input
              type="checkbox"
              checked={pp.enabled}
              onChange={(e) => pp.setEnabled(e.target.checked)}
            />
            <span className="toggle-track" />
            <span className="toggle-knob" />
          </label>
        </div>
        {pp.enabled && (
          <div style={{ marginTop: 14, display: 'flex', flexDirection: 'column', gap: 10 }}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                gap: 8,
              }}
            >
              <span style={{ fontSize: 13 }} aria-hidden="true">
                🔌 {t('performance.powerProfiles.pluggedIn')}
              </span>
              <select
                className="select-input"
                style={{ minWidth: 160 }}
                value={pp.acMode ?? ''}
                onChange={(e) =>
                  pp.setAcMode(
                    (e.target.value || null) as
                      import('../../hooks/useHardware').PerformanceMode | null,
                  )
                }
              >
                <option value="">{t('performance.powerProfiles.manual')}</option>
                <option value="silence">{t('performance.modes.silence')}</option>
                <option value="long_battery">{t('performance.modes.longBattery')}</option>
                <option value="balance">{t('performance.modes.balance')}</option>
                <option value="turbo">{t('performance.modes.turbo')}</option>
                <option value="decepticon">{t('performance.modes.decepticon')}</option>
                <option value="overdrive">{t('performance.modes.overdrive')}</option>
                <option value="overdrive_high">{t('performance.modes.overdriveHigh')}</option>
                <option value="overdrive_max">{t('performance.modes.overdriveMax')}</option>
                <option value="smart_adaptive">{t('performance.modes.smartAdaptive')}</option>
                <option value="smart">{t('performance.modes.smart')}</option>
                <option value="smart_acceleration">
                  {t('performance.modes.smartAcceleration')}
                </option>
              </select>
            </div>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                gap: 8,
              }}
            >
              <span style={{ fontSize: 13 }} aria-hidden="true">
                🔋 {t('performance.powerProfiles.onBattery')}
              </span>
              <select
                className="select-input"
                style={{ minWidth: 160 }}
                value={pp.dcMode ?? ''}
                onChange={(e) =>
                  pp.setDcMode(
                    (e.target.value || null) as
                      import('../../hooks/useHardware').PerformanceMode | null,
                  )
                }
              >
                <option value="">{t('performance.powerProfiles.manual')}</option>
                <option value="silence">{t('performance.modes.silence')}</option>
                <option value="long_battery">{t('performance.modes.longBattery')}</option>
                <option value="balance">{t('performance.modes.balance')}</option>
                <option value="turbo">{t('performance.modes.turbo')}</option>
                <option value="decepticon">{t('performance.modes.decepticon')}</option>
                <option value="overdrive">{t('performance.modes.overdrive')}</option>
                <option value="overdrive_high">{t('performance.modes.overdriveHigh')}</option>
                <option value="overdrive_max">{t('performance.modes.overdriveMax')}</option>
                <option value="smart_adaptive">{t('performance.modes.smartAdaptive')}</option>
                <option value="smart">{t('performance.modes.smart')}</option>
                <option value="smart_acceleration">
                  {t('performance.modes.smartAcceleration')}
                </option>
              </select>
            </div>
            <div style={{ fontSize: 11, color: 'var(--text-muted)', paddingTop: 4 }}>
              {pp.status?.last_event
                ? `⚙ ${t('performance.powerProfiles.lastApplied', {
                    event: pp.status.last_event,
                    mode: modeLabel(pp.status.last_mode),
                  })}${pp.status.last_ok === false ? ` (${pp.status.last_detail ?? 'failed'})` : ''}`
                : hw.battery?.is_plugged
                  ? `⚡ ${t('performance.powerProfiles.currentlyPluggedIn')}`
                  : `🔋 ${t('performance.powerProfiles.currentlyOnBattery')}`}
            </div>
            {pp.syncError && (
              <div style={{ fontSize: 11, color: 'var(--error, #f44336)' }}>
                ⚠ {t('performance.powerProfiles.syncFailed')}
              </div>
            )}
          </div>
        )}
      </div>

      <div className="card">
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            marginBottom: showLogs ? 14 : 0,
          }}
        >
          <div>
            <div className="card-title" style={{ marginBottom: 2 }}>
              {t('ai.logsTitle')}
            </div>
            <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>
              {isAiMode && aiApiKeySet
                ? t('ai.loggingActive')
                : aiApiKeySet
                  ? t('ai.configureSmartMode')
                  : t('ai.configureApiKeyLogging')}
            </div>
          </div>
          <div style={{ display: 'flex', gap: 8 }}>
            <button
              className="btn-secondary"
              style={{ fontSize: 12 }}
              onClick={() => setShowLogs((v) => !v)}
            >
              {showLogs ? t('common.hide') : t('ai.viewLogs')}
            </button>
            <button
              className="btn-secondary"
              style={{ fontSize: 12 }}
              onClick={() => void hw.openAiLogsDir()}
              title={t('ai.openLogFolder')}
            >
              📂
            </button>
          </div>
        </div>

        {showLogs && (
          <div>
            {loadingLogs ? (
              <div
                style={{
                  textAlign: 'center',
                  color: 'var(--text-dim)',
                  padding: '16px 0',
                  fontSize: 13,
                }}
              >
                {t('common.loading')}
              </div>
            ) : logEntries.length === 0 ? (
              <div
                style={{
                  textAlign: 'center',
                  color: 'var(--text-dim)',
                  padding: '16px 0',
                  fontSize: 13,
                }}
              >
                {t('ai.noEntries')}
              </div>
            ) : (
              <div style={{ overflowX: 'auto' }}>
                <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 12 }}>
                  <thead>
                    <tr
                      style={{
                        color: 'var(--text-muted)',
                        borderBottom: '1px solid var(--border)',
                      }}
                    >
                      <th style={{ textAlign: 'left', padding: '4px 8px', fontWeight: 500 }}>
                        {t('ai.columnTime')}
                      </th>
                      <th style={{ textAlign: 'left', padding: '4px 8px', fontWeight: 500 }}>
                        {t('ai.columnMode')}
                      </th>
                      <th style={{ textAlign: 'right', padding: '4px 8px', fontWeight: 500 }}>
                        {t('ai.columnCpu')}
                      </th>
                      <th style={{ textAlign: 'right', padding: '4px 8px', fontWeight: 500 }}>
                        {t('ai.columnGpu')}
                      </th>
                      <th style={{ textAlign: 'right', padding: '4px 8px', fontWeight: 500 }}>
                        {t('ai.columnTdp')}
                      </th>
                      <th style={{ textAlign: 'right', padding: '4px 8px', fontWeight: 500 }}>
                        {t('ai.columnCpuPct')}
                      </th>
                      <th style={{ textAlign: 'right', padding: '4px 8px', fontWeight: 500 }}>
                        {t('ai.columnGpuPct')}
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {logEntries.map((e, i) => (
                      <tr
                        key={i}
                        style={{ borderBottom: '1px solid var(--border-faint, var(--border))' }}
                      >
                        <td
                          style={{
                            padding: '4px 8px',
                            fontFamily: 'var(--font-mono)',
                            color: 'var(--text-dim)',
                          }}
                        >
                          {e.ts.slice(11)}
                        </td>
                        <td style={{ padding: '4px 8px', color: 'var(--accent)' }}>{e.mode}</td>
                        <td
                          style={{
                            padding: '4px 8px',
                            textAlign: 'right',
                            fontFamily: 'var(--font-mono)',
                          }}
                        >
                          {e.cpu_temp.toFixed(0)}
                        </td>
                        <td
                          style={{
                            padding: '4px 8px',
                            textAlign: 'right',
                            fontFamily: 'var(--font-mono)',
                          }}
                        >
                          {e.gpu_temp.toFixed(0)}
                        </td>
                        <td
                          style={{
                            padding: '4px 8px',
                            textAlign: 'right',
                            fontFamily: 'var(--font-mono)',
                          }}
                        >
                          {e.tdp_watts != null ? e.tdp_watts.toFixed(1) : '—'}
                        </td>
                        <td
                          style={{
                            padding: '4px 8px',
                            textAlign: 'right',
                            fontFamily: 'var(--font-mono)',
                          }}
                        >
                          {e.cpu_pct.toFixed(0)}
                        </td>
                        <td
                          style={{
                            padding: '4px 8px',
                            textAlign: 'right',
                            fontFamily: 'var(--font-mono)',
                          }}
                        >
                          {e.gpu_pct.toFixed(0)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                <div
                  style={{
                    marginTop: 10,
                    display: 'flex',
                    justifyContent: 'space-between',
                    alignItems: 'center',
                  }}
                >
                  <span style={{ fontSize: 11, color: 'var(--text-dim)' }}>
                    {t('ai.showingEntries', { n: String(logEntries.length) })}
                  </span>
                  <button
                    className="btn-secondary"
                    style={{ fontSize: 11 }}
                    onClick={() => void loadLogs()}
                  >
                    ↻ {t('ai.refreshLogs')}
                  </button>
                </div>
              </div>
            )}
          </div>
        )}
      </div>

      {/* Performance channel diagnostics — dev only */}
      {import.meta.env.DEV && (
        <div className="card">
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
            <div>
              <div className="card-title" style={{ marginBottom: 2 }}>
                {t('performance.channels.title')}
              </div>
              <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>
                {t('performance.channels.subtitle')}
              </div>
            </div>
            <button
              className="btn-secondary"
              style={{ fontSize: 12 }}
              onClick={() => void runPerfDebug()}
              disabled={loadingDebug}
            >
              {loadingDebug
                ? t('performance.channels.checking')
                : t('performance.channels.checkNow')}
            </button>
          </div>

          {debugInfo && (
            <div
              style={{
                marginTop: 14,
                display: 'flex',
                flexDirection: 'column',
                gap: 8,
                fontSize: 13,
              }}
            >
              <div
                style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}
              >
                <span style={{ color: 'var(--text-muted)' }}>
                  {t('performance.channels.hqWmi')}
                </span>
                <span
                  style={{
                    color: debugInfo.hq_wmi_works
                      ? 'var(--success, #4caf50)'
                      : 'var(--error, #f44336)',
                    fontWeight: 600,
                  }}
                >
                  {debugInfo.hq_wmi_works
                    ? `✓ ${t('performance.channels.functional')}`
                    : `✗ ${t('performance.channels.unavailable')}`}
                </span>
              </div>
              {debugInfo.hq_wmi_instance && (
                <div
                  style={{ display: 'flex', justifyContent: 'space-between', gap: 8, fontSize: 11 }}
                >
                  <span style={{ color: 'var(--text-muted)' }}>
                    {t('performance.channels.instance')}
                  </span>
                  <code
                    style={{
                      color: 'var(--text-dim)',
                      fontFamily: 'var(--font-mono)',
                      maxWidth: 260,
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {debugInfo.hq_wmi_instance}
                  </code>
                </div>
              )}
              {debugInfo.hq_wmi_test_ret && (
                <div
                  style={{ display: 'flex', justifyContent: 'space-between', gap: 8, fontSize: 11 }}
                >
                  <span style={{ color: 'var(--text-muted)' }}>
                    {t('performance.channels.response')}
                  </span>
                  <code style={{ color: 'var(--text-dim)', fontFamily: 'var(--font-mono)' }}>
                    {debugInfo.hq_wmi_test_ret}
                  </code>
                </div>
              )}
              <div
                style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}
              >
                <span style={{ color: 'var(--text-muted)' }}>{t('performance.channels.vhf')}</span>
                <span
                  style={{
                    color: debugInfo.vhf_device_path
                      ? 'var(--success, #4caf50)'
                      : 'var(--text-dim)',
                    fontWeight: 600,
                  }}
                >
                  {debugInfo.vhf_device_path
                    ? `✓ ${t('performance.channels.found')}`
                    : `— ${t('performance.channels.notFound')}`}
                </span>
              </div>
              {debugInfo.vhf_device_path && (
                <div
                  style={{ display: 'flex', justifyContent: 'space-between', gap: 8, fontSize: 11 }}
                >
                  <span style={{ color: 'var(--text-muted)' }}>Path</span>
                  <code
                    style={{
                      color: 'var(--text-dim)',
                      fontFamily: 'var(--font-mono)',
                      maxWidth: 260,
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {debugInfo.vhf_device_path}
                  </code>
                </div>
              )}
              <div style={{ display: 'flex', justifyContent: 'space-between', gap: 8 }}>
                <span style={{ color: 'var(--text-muted)' }}>
                  {t('performance.channels.registry')}
                </span>
                <code style={{ color: 'var(--text-dim)', fontFamily: 'var(--font-mono)' }}>
                  {debugInfo.registry_mode}
                </code>
              </div>
              <div style={{ display: 'flex', justifyContent: 'space-between', gap: 8 }}>
                <span style={{ color: 'var(--text-muted)' }}>
                  {t('performance.channels.overlay')}
                </span>
                <code style={{ color: 'var(--text-dim)', fontFamily: 'var(--font-mono)' }}>
                  {debugInfo.overlay_mode}
                </code>
              </div>
            </div>
          )}
          {debugInfo && (
            <div style={{ fontSize: 11, color: 'var(--text-dim)', marginTop: 8, lineHeight: 1.5 }}>
              {t('performance.channels.note')}
            </div>
          )}
        </div>
      )}
    </>
  );
}

// ── OS Turbo Card ────────────────────────────────────────────────────────────

interface OsTurboStatus {
  enabled: boolean;
  throttled_processes: number;
  power_plan: string;
  /** The recovery snapshot references processes that no longer exist. */
  snapshot_stale?: boolean;
  /** Truthful readback counts from the last sweep (S39-003). */
  measured?: OsTurboMeasuredData;
}

function OsTurboCard() {
  const [status, setStatus] = useState<OsTurboStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [toggling, setToggling] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const { addToast } = useToast();

  const fetchStatus = useCallback(async () => {
    try {
      const s = await invoke<OsTurboStatus>('get_os_turbo');
      setStatus(s);
    } catch {
      setStatus(null);
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    void fetchStatus();
  }, [fetchStatus]);

  // The quadro is measured, so it has to stay measured: refresh it while the
  // tab is open instead of showing whatever was true at mount time.
  useEffect(() => {
    const timer = window.setInterval(() => void fetchStatus(), 30_000);
    return () => window.clearInterval(timer);
  }, [fetchStatus]);

  const handleToggle = async () => {
    if (!status) return;
    setToggling(true);
    try {
      const s = await invoke<OsTurboStatus>('set_os_turbo', { enabled: !status.enabled });
      setStatus(s);
      addToast({
        message: s.enabled ? t('performance.osTurboEnabled') : t('performance.osTurboDisabled'),
        type: 'success',
      });
    } catch (e) {
      setErrorMsg(String(e));
    }
    setToggling(false);
  };

  if (loading) return null;

  const { applied, inspected } = osTurboCounts(status ?? undefined);

  return (
    <div className="card">
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          marginBottom: 4,
        }}
      >
        <div>
          <div className="card-title" style={{ marginBottom: 2 }}>
            ⚡ {t('performance.osTurboTitle')}
          </div>
          <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>
            {t('performance.osTurboDesc')}
          </div>
        </div>
        <label className="toggle-switch">
          <input
            type="checkbox"
            checked={status?.enabled ?? false}
            onChange={handleToggle}
            disabled={toggling || !status}
          />
          <span className="toggle-track" />
          <span className="toggle-knob" />
        </label>
      </div>
      {errorMsg && (
        <div className="alert alert-error" style={{ marginTop: 8 }}>
          ⚠ {errorMsg}
        </div>
      )}
      {status?.enabled && (
        <div style={{ marginTop: 12, fontSize: 12, color: 'var(--text-muted)' }}>
          <div>
            📊{' '}
            {inspected > 0
              ? t('performance.osTurboLimitSummary', { applied, inspected })
              : t('performance.osTurboNoCandidates')}
          </div>
          <div>
            🔌 {t('performance.osTurboPowerPlan')}: {status.power_plan}
          </div>
          {status.snapshot_stale && (
            <div className="alert alert-warn" style={{ marginTop: 8 }}>
              ⚠ {t('performance.osTurboStaleSnapshot')}
            </div>
          )}
          {(status.measured?.failures ?? 0) > 0 && (
            <div className="alert alert-warn" style={{ marginTop: 8 }}>
              ⚠{' '}
              {t('performance.osTurboFailuresHint', {
                failures: status.measured?.failures ?? 0,
              })}
            </div>
          )}
          <OsTurboMeasuredBlock measured={status.measured} />
        </div>
      )}
      <FunctionExplain
        summary={t('performance.osTurboExplainSummary')}
        details={t('performance.osTurboExplainDetails')}
        bullets={[
          t('performance.osTurboExplainBullet1'),
          t('performance.osTurboExplainBullet2'),
          t('performance.osTurboExplainBullet3'),
          t('performance.osTurboExplainBullet4'),
        ]}
        note={t('performance.osTurboExplainNote')}
      />
    </div>
  );
}

export default memo(PerformanceTab);
