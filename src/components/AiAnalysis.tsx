/**
 * AiAnalysis — dedicated AI Analysis tab component.
 *
 * Four cards:
 *  1. Settings — enable toggle, polling interval, daily analyses
 *  2. Live Charts — inline SVG showing last N log entries
 *  3. AI Analysis — trigger + latest result rendered as markdown
 *  4. Log Table — collapsible raw log viewer
 */

import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { t } from '../hooks/useI18n';
import { getLanguage } from '../hooks/useI18n';
import { useToast } from '../contexts/ToastContext';
import type { useSettings as UseSettings } from '../hooks/useSettings';
import type { AnalysisLogEntry } from '../hooks/useSettings';
import type { useHardware } from '../hooks/useHardware';
import {
  loadLogs,
  saveLogs,
  loadLastAnalysis,
  deleteLogById,
  LOGS_KEY,
  type LastAnalysis,
} from '../hooks/useAnalysisLogger';
import './AiAnalysis.css';

type Hardware = ReturnType<typeof useHardware>;
type AiSettings = ReturnType<typeof UseSettings>;

interface Props {
  hw: Hardware;
  ai: AiSettings;
  onOpenSettings: () => void;
}

// ── Markdown renderer (reused pattern from AiAdvisor) ────────────────────────

function RenderAnalysis({ text }: { text: string }) {
  const lines = text.split('\n').filter((l) => l.trim());
  return (
    <div className="ai-analysis-render">
      {lines.map((line, i) => {
        const clean = line.replace(/^[-*•]\s*/, '').trim();
        const isBullet = /^[-*•]/.test(line.trim()) || /^\d+\./.test(line.trim());
        const isHeading = line.trim().startsWith('**') || /^#{1,3}\s/.test(line.trim());
        const cleanHeading = clean.replace(/\*\*/g, '').replace(/^#+\s*/, '');
        if (isHeading) {
          return (
            <div key={i} className="ai-analysis-render__heading">
              {cleanHeading}
            </div>
          );
        }
        if (isBullet) {
          return (
            <div key={i} className="ai-analysis-render__bullet">
              <span className="ai-analysis-render__bullet-marker">›</span>
              <span>{clean.replace(/\*\*/g, '')}</span>
            </div>
          );
        }
        return (
          <div key={i} className="ai-analysis-render__paragraph">
            {clean.replace(/\*\*/g, '')}
          </div>
        );
      })}
    </div>
  );
}

// ── Inline SVG line chart ─────────────────────────────────────────────────────

interface SeriesDef {
  values: number[];
  color: string;
  label: string;
}

function LineChart({
  series,
  height = 100,
  unit = '',
}: {
  series: SeriesDef[];
  height?: number;
  unit?: string;
}) {
  if (series.every((s) => s.values.length < 2)) {
    return (
      <div className="ai-line-chart__empty" style={{ height }}>
        {t('aiAnalysis.charts.noData')}
      </div>
    );
  }

  const allVals = series.flatMap((s) => s.values);
  const minV = Math.min(...allVals);
  const maxV = Math.max(...allVals);
  const range = maxV - minV || 1;
  const W = 400;
  const H = height;
  const PAD = { top: 8, right: 8, bottom: 20, left: 36 };
  const innerW = W - PAD.left - PAD.right;
  const innerH = H - PAD.top - PAD.bottom;

  function toX(i: number, len: number) {
    return PAD.left + (i / Math.max(len - 1, 1)) * innerW;
  }
  function toY(v: number) {
    return PAD.top + innerH - ((v - minV) / range) * innerH;
  }

  // Y-axis labels (3 ticks)
  const ticks = [minV, minV + range / 2, maxV];

  return (
    <svg
      viewBox={`0 0 ${W} ${H}`}
      preserveAspectRatio="xMidYMid meet"
      className="ai-line-chart__svg"
      style={{ height }}
    >
      {/* Grid lines */}
      {ticks.map((tick, i) => (
        <g key={i}>
          <line
            x1={PAD.left}
            y1={toY(tick)}
            x2={W - PAD.right}
            y2={toY(tick)}
            stroke="var(--border)"
            strokeWidth="0.5"
            strokeDasharray="3,3"
          />
          <text
            x={PAD.left - 4}
            y={toY(tick) + 3.5}
            textAnchor="end"
            fontSize="9"
            fill="var(--text-dim)"
          >
            {tick.toFixed(0)}
            {unit}
          </text>
        </g>
      ))}

      {/* Series polylines */}
      {series.map((s) => {
        if (s.values.length < 2) return null;
        const points = s.values
          .map((v, i) => `${toX(i, s.values.length).toFixed(1)},${toY(v).toFixed(1)}`)
          .join(' ');
        return (
          <polyline
            key={s.label}
            points={points}
            fill="none"
            stroke={s.color}
            strokeWidth="1.5"
            strokeLinejoin="round"
            strokeLinecap="round"
          />
        );
      })}

      {/* X-axis: first and last timestamps handled by caller */}
      <line
        x1={PAD.left}
        y1={H - PAD.bottom}
        x2={W - PAD.right}
        y2={H - PAD.bottom}
        stroke="var(--border)"
        strokeWidth="0.8"
      />
    </svg>
  );
}

function ChartLegend({ items }: { items: { color: string; label: string; last?: string }[] }) {
  return (
    <div className="ai-chart-legend">
      {items.map((item) => (
        <div key={item.label} className="ai-chart-legend__item">
          <span className="ai-chart-legend__swatch" style={{ background: item.color }} />
          {item.label}
          {item.last !== undefined && <span className="ai-chart-legend__last">({item.last})</span>}
        </div>
      ))}
    </div>
  );
}

// ── Process bar chart ─────────────────────────────────────────────────────────

function ProcessBars({
  processes,
}: {
  processes: Array<{ name: string; cpu_pct: number; memory_mb: number }>;
}) {
  if (!processes.length) {
    return <div className="ai-process-bars__empty">{t('aiAnalysis.charts.noData')}</div>;
  }
  const maxCpu = Math.max(...processes.map((p) => p.cpu_pct), 1);
  return (
    <div className="ai-process-bars">
      {processes.slice(0, 8).map((p) => (
        <div key={p.name} className="ai-process-bars__row">
          <span className="ai-process-bars__name" title={p.name}>
            {p.name}
          </span>
          <div className="ai-process-bars__bar-track">
            <div
              className="ai-process-bars__bar-fill"
              style={{
                width: `${(p.cpu_pct / maxCpu) * 100}%`,
                background: p.cpu_pct > 50 ? 'var(--warning)' : 'var(--accent)',
              }}
            />
          </div>
          <span className="ai-process-bars__cpu">{p.cpu_pct.toFixed(1)}%</span>
          <span className="ai-process-bars__mem">
            {p.memory_mb >= 1024
              ? `${(p.memory_mb / 1024).toFixed(1)} GB`
              : `${p.memory_mb.toFixed(0)} MB`}
          </span>
        </div>
      ))}
    </div>
  );
}

// ── Format helpers ────────────────────────────────────────────────────────────

function fmtTime(iso: string): string {
  try {
    const d = new Date(iso);
    return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  } catch {
    return iso.slice(11, 16);
  }
}

function fmtDatetime(iso: string): string {
  try {
    return new Date(iso).toLocaleString([], {
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  } catch {
    return iso;
  }
}

function nextAnalysisIn(dailyAnalyses: number): string {
  try {
    const sched = JSON.parse(
      localStorage.getItem('micontrol_analysis_schedule_v1') ?? '{"recent":[]}',
    ) as { recent: string[] };
    if (!sched.recent.length) return '—';
    const intervalMs = (24 * 3600 * 1000) / Math.max(1, dailyAnalyses);
    const lastTs = new Date(sched.recent[sched.recent.length - 1]).getTime();
    const nextTs = lastTs + intervalMs;
    const diffSec = Math.round((nextTs - Date.now()) / 1000);
    if (diffSec <= 0) return t('aiAnalysis.analysis.due');
    if (diffSec < 60) return `${diffSec}s`;
    if (diffSec < 3600) return `${Math.round(diffSec / 60)} min`;
    return `${(diffSec / 3600).toFixed(1)} h`;
  } catch {
    return '—';
  }
}

// ── AI Usage Panel ───────────────────────────────────────────────────────────

interface AiUsageStats {
  total_requests: number;
  total_input_tokens: number;
  total_output_tokens: number;
  estimated_cost_usd: number;
}

function AiUsagePanel() {
  const [usage, setUsage] = useState<AiUsageStats | null>(null);

  useEffect(() => {
    invoke<AiUsageStats>('get_ai_usage')
      .then(setUsage)
      .catch(() => {
        /* ignore */
      });
  }, []);

  const handleReset = async () => {
    await invoke('reset_ai_usage');
    const updated = await invoke<AiUsageStats>('get_ai_usage');
    setUsage(updated);
  };

  if (!usage) return null;

  return (
    <div className="card ai-usage-panel">
      <div className="ai-usage-panel__header">
        <div>
          <div className="card-title ai-usage-panel__title">{t('ai.usage.title')}</div>
          <div className="ai-usage-panel__stats">
            {t('ai.usage.requests')}: <strong>{usage.total_requests}</strong>
            {' · '}
            {t('ai.usage.tokens')}:{' '}
            <strong>{usage.total_input_tokens + usage.total_output_tokens}</strong>
            {' · '}
            {t('ai.usage.estimatedCost')}: <strong>${usage.estimated_cost_usd.toFixed(4)}</strong>
          </div>
        </div>
        <button onClick={handleReset} className="ai-usage-panel__reset">
          {t('ai.usage.reset')}
        </button>
      </div>
    </div>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

export default function AiAnalysis({ hw, ai, onOpenSettings }: Props) {
  const { addToast } = useToast();
  const [logs, setLogs] = useState<AnalysisLogEntry[]>([]);
  const [lastAnalysis, setLastAnalysis] = useState<LastAnalysis | null>(null);
  const [analyzing, setAnalyzing] = useState(false);
  const [analyzeError, setAnalyzeError] = useState<string | null>(null);
  const [showLogTable, setShowLogTable] = useState(false);
  const [logTableCount, setLogTableCount] = useState(50);
  const [nextIn, setNextIn] = useState('—');

  // Refresh state from localStorage
  const refresh = useCallback(() => {
    setLogs(loadLogs());
    setLastAnalysis(loadLastAnalysis());
  }, []);

  useEffect(() => {
    refresh();
    // Also refresh when localStorage changes (polling hook writes)
    const handleStorage = (e: StorageEvent) => {
      if (e.key === LOGS_KEY || e.key === 'micontrol_last_analysis_v1') refresh();
    };
    window.addEventListener('storage', handleStorage);
    return () => window.removeEventListener('storage', handleStorage);
  }, [refresh]);

  // Poll "next in" display every 30s
  useEffect(() => {
    setNextIn(nextAnalysisIn(ai.settings.ai_daily_analyses));
    const id = setInterval(() => setNextIn(nextAnalysisIn(ai.settings.ai_daily_analyses)), 30_000);
    return () => clearInterval(id);
  }, [ai.settings.ai_daily_analyses, lastAnalysis]);

  // Manual trigger
  async function handleAnalyze() {
    if (!ai.isConfigured) {
      onOpenSettings();
      return;
    }
    setAnalyzing(true);
    setAnalyzeError(null);
    try {
      const currentLogs = loadLogs();
      if (currentLogs.length < 2) {
        setAnalyzeError(t('aiAnalysis.analysis.notEnoughData'));
        return;
      }
      const text = await ai.analyzeWithLogs(
        currentLogs,
        {
          deviceModel: hw.hardwareProfile?.device_model ?? null,
          systemInfo: hw.systemInfo,
          battery: hw.battery,
          performanceMode: hw.performanceMode,
          fan: hw.fan,
          display: hw.display,
          capabilities: hw.hardwareProfile?.capabilities ?? null,
        },
        getLanguage(),
      );

      const result: LastAnalysis = {
        ts: new Date().toISOString(),
        text,
        log_count: currentLogs.length,
      };
      localStorage.setItem('micontrol_last_analysis_v1', JSON.stringify(result));
      setLastAnalysis(result);
      addToast({ message: t('aiAnalysis.analysis.success'), type: 'success' });

      // Update schedule
      try {
        const sched = JSON.parse(
          localStorage.getItem('micontrol_analysis_schedule_v1') ?? '{"recent":[]}',
        ) as { recent: string[] };
        sched.recent = [...sched.recent.slice(-23), result.ts];
        localStorage.setItem('micontrol_analysis_schedule_v1', JSON.stringify(sched));
      } catch {
        /* ignore */
      }
    } catch (e) {
      const msg = String(e).replace(/^Error:\s*/, '');
      const displayMsg =
        msg === 'api_key_missing'
          ? t('settings.apiKeyMissing')
          : msg === 'no_logs'
            ? t('aiAnalysis.analysis.notEnoughData')
            : msg === 'consent_denied'
              ? t('ai.consentRequired')
              : msg;
      setAnalyzeError(displayMsg);
      addToast({ message: displayMsg, type: 'error', onRetry: handleAnalyze });
    } finally {
      setAnalyzing(false);
    }
  }

  function handleClearLogs() {
    saveLogs([]);
    setLogs([]);
  }

  function handleDeleteLog(id: string) {
    deleteLogById(id);
    setLogs((prev) => prev.filter((log) => log.id !== id));
  }

  // Derive chart data from last 80 log entries
  const chartLogs = logs.slice(-80);
  const cpuTemps: SeriesDef = {
    values: chartLogs.map((l) => l.cpu_temp),
    color: 'var(--error, #f44336)',
    label: t('aiAnalysis.charts.cpuTemp'),
  };
  const gpuTemps: SeriesDef = {
    values: chartLogs.map((l) => l.gpu_temp),
    color: '#ff9800',
    label: t('aiAnalysis.charts.gpuTemp'),
  };
  const tdpSeries: SeriesDef = {
    values: chartLogs.filter((l) => l.tdp_watts != null).map((l) => l.tdp_watts as number),
    color: 'var(--info, #2196f3)',
    label: t('aiAnalysis.charts.tdp'),
  };
  const cpuPct: SeriesDef = {
    values: chartLogs.map((l) => l.cpu_pct),
    color: 'var(--accent)',
    label: 'CPU%',
  };
  const gpuPct: SeriesDef = {
    values: chartLogs.map((l) => l.gpu_pct),
    color: '#00bcd4',
    label: 'GPU%',
  };

  const lastLog = logs[logs.length - 1];
  const topProcs = lastLog?.top_processes ?? [];

  const spanMs =
    chartLogs.length >= 2
      ? new Date(chartLogs[chartLogs.length - 1].ts).getTime() - new Date(chartLogs[0].ts).getTime()
      : 0;
  const spanLabel =
    spanMs > 0
      ? spanMs < 3600_000
        ? `${Math.round(spanMs / 60000)} min`
        : `${(spanMs / 3600_000).toFixed(1)} h`
      : '';

  // ── Render ──────────────────────────────────────────────────────────────────

  return (
    <>
      {/* ── Settings card ─────────────────────────────────────────────────── */}
      <div className="card">
        <div className="ai-settings-card__header">
          <div>
            <div className="card-title ai-settings-card__title">
              {t('aiAnalysis.settings.title')}
            </div>
            <div className="ai-settings-card__subtitle">{t('aiAnalysis.settings.subtitle')}</div>
          </div>
          <label className="toggle-switch">
            <input
              type="checkbox"
              checked={ai.settings.ai_analysis_enabled}
              onChange={(e) => {
                ai.updateKey('ai_analysis_enabled', e.target.checked);
                addToast({
                  message: e.target.checked
                    ? t('aiAnalysis.settings.toastEnabled')
                    : t('aiAnalysis.settings.toastDisabled'),
                  type: 'info',
                });
              }}
            />
            <span className="toggle-track" />
            <span className="toggle-knob" />
          </label>
        </div>

        {ai.settings.ai_analysis_enabled && (
          <div className="ai-settings-card__body">
            {/* Polling interval */}
            <div className="ai-settings-row">
              <div>
                <div className="ai-settings-row__label">
                  {t('aiAnalysis.settings.pollInterval')}
                </div>
                <div className="ai-settings-row__desc">
                  {t('aiAnalysis.settings.pollIntervalDesc')}
                </div>
              </div>
              <select
                className="select-input ai-settings-row__select"
                value={ai.settings.ai_poll_interval_sec}
                onChange={(e) => ai.updateKey('ai_poll_interval_sec', Number(e.target.value))}
              >
                <option value={15}>15 s</option>
                <option value={30}>30 s</option>
                <option value={60}>1 min</option>
                <option value={120}>2 min</option>
                <option value={300}>5 min</option>
              </select>
            </div>

            {/* Daily analyses */}
            <div className="ai-settings-row">
              <div>
                <div className="ai-settings-row__label">
                  {t('aiAnalysis.settings.dailyAnalyses')}
                </div>
                <div className="ai-settings-row__desc">
                  {t('aiAnalysis.settings.dailyAnalysesDesc')}
                </div>
              </div>
              <select
                className="select-input ai-settings-row__select"
                value={ai.settings.ai_daily_analyses}
                onChange={(e) => ai.updateKey('ai_daily_analyses', Number(e.target.value))}
              >
                <option value={1}>{t('aiAnalysis.settings.timesPerDay', { n: '1' })}</option>
                <option value={2}>{t('aiAnalysis.settings.timesPerDay', { n: '2' })}</option>
                <option value={4}>{t('aiAnalysis.settings.timesPerDay', { n: '4' })}</option>
                <option value={6}>{t('aiAnalysis.settings.timesPerDay', { n: '6' })}</option>
                <option value={12}>{t('aiAnalysis.settings.timesPerDay', { n: '12' })}</option>
                <option value={24}>{t('aiAnalysis.settings.timesPerDay', { n: '24' })}</option>
              </select>
            </div>

            {/* Model hint */}
            {!ai.isConfigured ? (
              <div className="ai-settings-warning">
                ⚠️ {t('ai.notConfigured')}{' '}
                <button onClick={onOpenSettings} className="ai-settings-link">
                  {t('settings.title')}
                </button>
              </div>
            ) : (
              <div className="ai-settings-model-hint">
                🤖{' '}
                {t('aiAnalysis.settings.modelHint', {
                  model: ai.settings.openai_model || 'gpt-4o-mini',
                })}
              </div>
            )}

            {/* Stats row */}
            <div className="ai-settings-stats">
              <div className="ai-settings-stat">
                {t('aiAnalysis.settings.logCount')}: <strong>{logs.length}</strong> / {500}
              </div>
              {spanLabel && (
                <div className="ai-settings-stat">
                  {t('aiAnalysis.settings.span')}: <strong>{spanLabel}</strong>
                </div>
              )}
              <button onClick={handleClearLogs} className="ai-settings-clear-logs">
                {t('aiAnalysis.settings.clearLogs')}
              </button>
            </div>
          </div>
        )}

        {!ai.settings.ai_analysis_enabled && (
          <div className="ai-settings-disabled-hint">{t('aiAnalysis.settings.disabledHint')}</div>
        )}
      </div>

      {/* ── Charts card ───────────────────────────────────────────────────── */}
      <div className="card">
        <div className="card-title ai-charts-card__title">{t('aiAnalysis.charts.title')}</div>
        <div className="ai-charts-card__subtitle">
          {chartLogs.length > 0
            ? t('aiAnalysis.charts.subtitle', {
                n: String(chartLogs.length),
                span: spanLabel || '—',
              })
            : t('aiAnalysis.charts.noData')}
        </div>

        {chartLogs.length >= 2 ? (
          <div className="ai-charts-container">
            {/* Temperature chart */}
            <div>
              <div className="ai-chart-section__label">
                {t('aiAnalysis.charts.temperature')} (°C)
              </div>
              <LineChart series={[cpuTemps, gpuTemps]} height={100} unit="°" />
              <div className="ai-chart-axis-labels">
                <span>{fmtTime(chartLogs[0].ts)}</span>
                <span>{fmtTime(chartLogs[chartLogs.length - 1].ts)}</span>
              </div>
              <ChartLegend
                items={[
                  {
                    color: cpuTemps.color,
                    label: cpuTemps.label,
                    last: `${lastLog?.cpu_temp.toFixed(0)}°C`,
                  },
                  {
                    color: gpuTemps.color,
                    label: gpuTemps.label,
                    last: `${lastLog?.gpu_temp.toFixed(0)}°C`,
                  },
                ]}
              />
            </div>

            {/* TDP chart */}
            {tdpSeries.values.length >= 2 && (
              <div>
                <div className="ai-chart-section__label">{t('aiAnalysis.charts.tdp')} (W)</div>
                <LineChart series={[tdpSeries]} height={80} unit="W" />
                <div className="ai-chart-axis-labels">
                  <span>{fmtTime(chartLogs[0].ts)}</span>
                  <span>{fmtTime(chartLogs[chartLogs.length - 1].ts)}</span>
                </div>
                <ChartLegend
                  items={[
                    {
                      color: tdpSeries.color,
                      label: tdpSeries.label,
                      last: `${lastLog?.tdp_watts?.toFixed(1) ?? '—'} W`,
                    },
                  ]}
                />
              </div>
            )}

            {/* CPU/GPU usage chart */}
            <div>
              <div className="ai-chart-section__label">{t('aiAnalysis.charts.usage')} (%)</div>
              <LineChart series={[cpuPct, gpuPct]} height={80} unit="%" />
              <div className="ai-chart-axis-labels">
                <span>{fmtTime(chartLogs[0].ts)}</span>
                <span>{fmtTime(chartLogs[chartLogs.length - 1].ts)}</span>
              </div>
              <ChartLegend
                items={[
                  {
                    color: cpuPct.color,
                    label: cpuPct.label,
                    last: `${lastLog?.cpu_pct.toFixed(0)}%`,
                  },
                  {
                    color: gpuPct.color,
                    label: gpuPct.label,
                    last: `${lastLog?.gpu_pct.toFixed(0)}%`,
                  },
                ]}
              />
            </div>

            {/* Process bars */}
            {topProcs.length > 0 && (
              <div>
                <div className="ai-chart-section__label ai-chart-section__label--processes">
                  {t('aiAnalysis.charts.topProcesses')}
                </div>
                <ProcessBars processes={topProcs} />
              </div>
            )}
          </div>
        ) : (
          <div className="ai-charts-empty">
            <span className="ai-charts-empty__icon" aria-hidden="true">
              📊
            </span>
            <span className="ai-charts-empty__title">{t('aiAnalysis.charts.waitingForData')}</span>
            <span className="ai-charts-empty__hint">{t('aiAnalysis.charts.enableHint')}</span>
          </div>
        )}
      </div>

      {/* ── Analysis card ─────────────────────────────────────────────────── */}
      <div className="card">
        <div className="ai-analysis-card__header">
          <div>
            <div className="card-title ai-analysis-card__title">
              {t('aiAnalysis.analysis.title')}
            </div>
            <div className="ai-analysis-card__meta">
              {lastAnalysis
                ? t('aiAnalysis.analysis.lastAt', {
                    time: fmtDatetime(lastAnalysis.ts),
                    n: String(lastAnalysis.log_count),
                  })
                : t('aiAnalysis.analysis.neverRun')}
            </div>
          </div>
          <div className="ai-analysis-card__actions">
            <button
              className="btn-primary ai-analysis-card__trigger"
              onClick={handleAnalyze}
              disabled={analyzing || logs.length < 2}
            >
              {analyzing ? t('aiAnalysis.analysis.analyzing') : t('aiAnalysis.analysis.trigger')}
            </button>
            {lastAnalysis && (
              <div className="ai-analysis-card__next-in">
                {t('aiAnalysis.analysis.nextIn')}: {nextIn}
              </div>
            )}
          </div>
        </div>

        {analyzeError && <div className="ai-analysis-error">{analyzeError}</div>}

        {analyzing && (
          <div className="ai-analysis-loading">
            <span className="ai-analysis-loading__spinner" />
            {t('aiAnalysis.analysis.analyzing')}…
          </div>
        )}

        {lastAnalysis && !analyzing && (
          <div className="ai-analysis-result">
            <RenderAnalysis text={lastAnalysis.text} />
          </div>
        )}

        {!lastAnalysis && !analyzing && !analyzeError && (
          <div className="ai-analysis-empty">
            <span className="ai-analysis-empty__icon" aria-hidden="true">
              🤖
            </span>
            <span className="ai-analysis-empty__text">{t('aiAnalysis.analysis.noResultYet')}</span>
          </div>
        )}
      </div>

      {/* ── AI disclaimer ──────────────────────────────────────────────────── */}
      <div className="ai-disclaimer">⚠️ {t('ai.disclaimer')}</div>

      {/* ── AI Usage Panel ─────────────────────────────────────────────────── */}
      <AiUsagePanel />

      {/* ── Log table card ────────────────────────────────────────────────── */}
      <div className="card">
        <div className="ai-log-table__header">
          <div>
            <div className="card-title ai-log-table__title">{t('aiAnalysis.logs.title')}</div>
            <div className="ai-log-table__count">{t('common.logs', { count: logs.length })}</div>
          </div>
          <button
            className="btn-secondary ai-log-table__toggle"
            onClick={() => setShowLogTable((v) => !v)}
          >
            {showLogTable ? t('common.hide') : t('common.show')}
          </button>
        </div>

        {showLogTable && (
          <div className="ai-log-table__body">
            {logs.length === 0 ? (
              <div className="ai-log-table__empty">{t('aiAnalysis.logs.noEntries')}</div>
            ) : (
              <>
                <div className="ai-log-table__wrapper">
                  <table className="ai-log-table__table">
                    <thead className="ai-log-table__thead">
                      <tr>
                        <th className="ai-log-table__cell ai-log-table__cell--left">
                          {t('aiAnalysis.logs.time')}
                        </th>
                        <th className="ai-log-table__cell ai-log-table__cell--left">
                          {t('aiAnalysis.logs.mode')}
                        </th>
                        <th className="ai-log-table__cell ai-log-table__cell--num">CPU°C</th>
                        <th className="ai-log-table__cell ai-log-table__cell--num">GPU°C</th>
                        <th className="ai-log-table__cell ai-log-table__cell--num">TDP W</th>
                        <th className="ai-log-table__cell ai-log-table__cell--num">CPU%</th>
                        <th className="ai-log-table__cell ai-log-table__cell--num">GPU%</th>
                        <th className="ai-log-table__cell ai-log-table__cell--num">🔋%</th>
                        <th className="ai-log-table__cell ai-log-table__cell--action"></th>
                      </tr>
                    </thead>
                    <tbody>
                      {logs
                        .slice(-logTableCount)
                        .reverse()
                        .map((e, i) => (
                          <tr key={i} className="ai-log-table__row">
                            <td className="ai-log-table__cell ai-log-table__cell--time">
                              {fmtTime(e.ts)}
                            </td>
                            <td className="ai-log-table__cell ai-log-table__cell--mode">
                              {e.mode}
                            </td>
                            <td className="ai-log-table__cell ai-log-table__cell--num">
                              {e.cpu_temp.toFixed(0)}
                            </td>
                            <td className="ai-log-table__cell ai-log-table__cell--num">
                              {e.gpu_temp.toFixed(0)}
                            </td>
                            <td className="ai-log-table__cell ai-log-table__cell--num">
                              {e.tdp_watts != null ? e.tdp_watts.toFixed(1) : '—'}
                            </td>
                            <td className="ai-log-table__cell ai-log-table__cell--num">
                              {e.cpu_pct.toFixed(0)}
                            </td>
                            <td className="ai-log-table__cell ai-log-table__cell--num">
                              {e.gpu_pct.toFixed(0)}
                            </td>
                            <td className="ai-log-table__cell ai-log-table__cell--num">
                              {e.battery_level != null ? e.battery_level.toFixed(0) : '—'}
                            </td>
                            <td className="ai-log-table__cell ai-log-table__cell--action">
                              <button
                                onClick={() => handleDeleteLog(e.id)}
                                className="delete-log-button"
                                aria-label={t('aiAnalysis.logs.deleteLog')}
                                title={t('aiAnalysis.logs.deleteLog')}
                              >
                                ×
                              </button>
                            </td>
                          </tr>
                        ))}
                    </tbody>
                  </table>
                </div>
                {logs.length > logTableCount && (
                  <button
                    className="btn-secondary ai-log-table__load-more"
                    onClick={() => setLogTableCount((n) => n + 50)}
                  >
                    {t('aiAnalysis.logs.loadMore')}
                  </button>
                )}
              </>
            )}
          </div>
        )}
      </div>
    </>
  );
}
