import { useState, useEffect, useRef, useCallback, memo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { t } from '../../hooks/useI18n';
import { PageHeader } from './PageHeader';
import PerformanceMonitor from '../../components/PerformanceMonitor';
import PerformanceModeSelector from '../../components/PerformanceModeSelector';
import { useToast } from '../../contexts/ToastContext';
import type { Hardware, AiSettings, PerfDebugInfo } from './shared';

interface Props {
  hw: Hardware;
  ai: AiSettings;
  onOpenSettings: () => void;
}

function PerformanceTab({ hw, ai, onOpenSettings }: Props) {
  const { addToast } = useToast();
  const aiApiKeySet = !!ai.settings.openai_api_key;
  const isAiMode = hw.performanceMode === 'smart' || hw.performanceMode === 'smart_acceleration';

  // ── Auto-switch performance mode on AC ↔ DC transition ───────────────────
  const autoSwitchRef = useRef(ai.settings.auto_switch_perf);
  const acModeRef = useRef(ai.settings.perf_mode_ac);
  const dcModeRef = useRef(ai.settings.perf_mode_dc);
  autoSwitchRef.current = ai.settings.auto_switch_perf;
  acModeRef.current = ai.settings.perf_mode_ac;
  dcModeRef.current = ai.settings.perf_mode_dc;

  const prevPluggedRef = useRef<boolean | null>(null);
  useEffect(() => {
    const plugged = hw.battery?.is_plugged ?? null;
    if (plugged === null) return;
    if (prevPluggedRef.current === null) {
      prevPluggedRef.current = plugged;
      return;
    }
    if (prevPluggedRef.current === plugged) return;
    prevPluggedRef.current = plugged;
    if (!autoSwitchRef.current) return;
    const targetMode = plugged ? acModeRef.current : dcModeRef.current;
    if (targetMode) void hw.setPerformanceMode(targetMode);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hw.battery?.is_plugged]);

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
          aiApiKeySet={aiApiKeySet}
          onOpenSettings={onOpenSettings}
        />
      </div>

      {/* Power Profiles */}
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
              checked={ai.settings.auto_switch_perf}
              onChange={(e) => ai.updateKey('auto_switch_perf', e.target.checked)}
            />
            <span className="toggle-track" />
            <span className="toggle-knob" />
          </label>
        </div>
        {ai.settings.auto_switch_perf && (
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
                value={ai.settings.perf_mode_ac ?? ''}
                onChange={(e) =>
                  ai.updateKey(
                    'perf_mode_ac',
                    (e.target.value || null) as
                      | import('../../hooks/useHardware').PerformanceMode
                      | null,
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
                value={ai.settings.perf_mode_dc ?? ''}
                onChange={(e) =>
                  ai.updateKey(
                    'perf_mode_dc',
                    (e.target.value || null) as
                      | import('../../hooks/useHardware').PerformanceMode
                      | null,
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
              {hw.battery?.is_plugged
                ? `⚡ ${t('performance.powerProfiles.currentlyPluggedIn')}`
                : `🔋 ${t('performance.powerProfiles.currentlyOnBattery')}`}
            </div>
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

      {/* Performance channel diagnostics */}
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
            {loadingDebug ? t('performance.channels.checking') : t('performance.channels.checkNow')}
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
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
              <span style={{ color: 'var(--text-muted)' }}>{t('performance.channels.hqWmi')}</span>
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
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
              <span style={{ color: 'var(--text-muted)' }}>{t('performance.channels.vhf')}</span>
              <span
                style={{
                  color: debugInfo.vhf_device_path ? 'var(--success, #4caf50)' : 'var(--text-dim)',
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
    </>
  );
}

export default memo(PerformanceTab);
