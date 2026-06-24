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
    <div style={{ lineHeight: 1.75, fontSize: 13 }}>
      {lines.map((line, i) => {
        const clean = line.replace(/^[-*•]\s*/, '').trim();
        const isBullet = /^[-*•]/.test(line.trim()) || /^\d+\./.test(line.trim());
        const isHeading = line.trim().startsWith('**') || /^#{1,3}\s/.test(line.trim());
        const cleanHeading = clean.replace(/\*\*/g, '').replace(/^#+\s*/, '');
        if (isHeading) {
          return (
            <div key={i} style={{ fontWeight: 700, marginTop: i > 0 ? 14 : 0, marginBottom: 2 }}>
              {cleanHeading}
            </div>
          );
        }
        if (isBullet) {
          return (
            <div key={i} style={{ display: 'flex', gap: 8, marginLeft: 4 }}>
              <span style={{ color: 'var(--color-accent)', flexShrink: 0 }}>›</span>
              <span>{clean.replace(/\*\*/g, '')}</span>
            </div>
          );
        }
        return (
          <div key={i} style={{ marginTop: 4 }}>
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
      <div
        style={{
          height,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: 'var(--text-dim)',
          fontSize: 12,
        }}
      >
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
      preserveAspectRatio="none"
      style={{ width: '100%', height, display: 'block' }}
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
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: '6px 14px', marginTop: 6 }}>
      {items.map((item) => (
        <div
          key={item.label}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 5,
            fontSize: 11,
            color: 'var(--text-muted)',
          }}
        >
          <span
            style={{
              width: 10,
              height: 2.5,
              background: item.color,
              borderRadius: 2,
              display: 'inline-block',
              flexShrink: 0,
            }}
          />
          {item.label}
          {item.last !== undefined && (
            <span style={{ color: 'var(--text)', fontFamily: 'var(--font-mono)', fontSize: 10 }}>
              ({item.last})
            </span>
          )}
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
    return (
      <div style={{ fontSize: 12, color: 'var(--text-dim)' }}>{t('aiAnalysis.charts.noData')}</div>
    );
  }
  const maxCpu = Math.max(...processes.map((p) => p.cpu_pct), 1);
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
      {processes.slice(0, 8).map((p) => (
        <div key={p.name} style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 11 }}>
          <span
            style={{
              width: 140,
              flexShrink: 0,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
              color: 'var(--text-muted)',
            }}
            title={p.name}
          >
            {p.name}
          </span>
          <div
            style={{
              flex: 1,
              height: 6,
              background: 'var(--surface-2)',
              borderRadius: 3,
              overflow: 'hidden',
            }}
          >
            <div
              style={{
                width: `${(p.cpu_pct / maxCpu) * 100}%`,
                height: '100%',
                background: p.cpu_pct > 50 ? 'var(--warning)' : 'var(--accent)',
                borderRadius: 3,
                transition: 'width 0.3s ease',
              }}
            />
          </div>
          <span
            style={{
              width: 42,
              textAlign: 'right',
              fontFamily: 'var(--font-mono)',
              color: 'var(--text-dim)',
            }}
          >
            {p.cpu_pct.toFixed(1)}%
          </span>
          <span
            style={{
              width: 54,
              textAlign: 'right',
              fontFamily: 'var(--font-mono)',
              color: 'var(--text-dim)',
              fontSize: 10,
            }}
          >
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
    <div className="card" style={{ marginTop: 12 }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <div>
          <div className="card-title" style={{ marginBottom: 2 }}>
            {t('ai.usage.title')}
          </div>
          <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>
            {t('ai.usage.requests')}: <strong>{usage.total_requests}</strong>
            {' · '}
            {t('ai.usage.tokens')}:{' '}
            <strong>{usage.total_input_tokens + usage.total_output_tokens}</strong>
            {' · '}
            {t('ai.usage.estimatedCost')}: <strong>${usage.estimated_cost_usd.toFixed(4)}</strong>
          </div>
        </div>
        <button
          onClick={handleReset}
          style={{
            background: 'none',
            border: '1px solid var(--border)',
            borderRadius: 6,
            padding: '4px 12px',
            cursor: 'pointer',
            fontSize: 11,
            color: 'var(--text-muted)',
            whiteSpace: 'nowrap',
          }}
        >
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
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            marginBottom: 12,
          }}
        >
          <div>
            <div className="card-title" style={{ marginBottom: 2 }}>
              {t('aiAnalysis.settings.title')}
            </div>
            <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>
              {t('aiAnalysis.settings.subtitle')}
            </div>
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
          <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
            {/* Polling interval */}
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                gap: 8,
              }}
            >
              <div>
                <div style={{ fontSize: 13 }}>{t('aiAnalysis.settings.pollInterval')}</div>
                <div style={{ fontSize: 11, color: 'var(--text-muted)', marginTop: 2 }}>
                  {t('aiAnalysis.settings.pollIntervalDesc')}
                </div>
              </div>
              <select
                className="select-input"
                style={{ minWidth: 130 }}
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
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                gap: 8,
              }}
            >
              <div>
                <div style={{ fontSize: 13 }}>{t('aiAnalysis.settings.dailyAnalyses')}</div>
                <div style={{ fontSize: 11, color: 'var(--text-muted)', marginTop: 2 }}>
                  {t('aiAnalysis.settings.dailyAnalysesDesc')}
                </div>
              </div>
              <select
                className="select-input"
                style={{ minWidth: 130 }}
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
              <div
                style={{
                  fontSize: 12,
                  color: 'var(--warning, #ff9800)',
                  display: 'flex',
                  alignItems: 'center',
                  gap: 6,
                }}
              >
                ⚠️ {t('ai.notConfigured')}{' '}
                <button
                  onClick={onOpenSettings}
                  style={{
                    background: 'none',
                    border: 'none',
                    color: 'var(--accent)',
                    cursor: 'pointer',
                    fontSize: 12,
                    padding: 0,
                    textDecoration: 'underline',
                  }}
                >
                  {t('settings.title')}
                </button>
              </div>
            ) : (
              <div style={{ fontSize: 11, color: 'var(--text-dim)' }}>
                🤖{' '}
                {t('aiAnalysis.settings.modelHint', {
                  model: ai.settings.openai_model || 'gpt-4o-mini',
                })}
              </div>
            )}

            {/* Stats row */}
            <div
              style={{
                display: 'flex',
                gap: 16,
                paddingTop: 6,
                borderTop: '1px solid var(--border)',
                flexWrap: 'wrap',
              }}
            >
              <div style={{ fontSize: 11, color: 'var(--text-muted)' }}>
                {t('aiAnalysis.settings.logCount')}:{' '}
                <strong style={{ color: 'var(--text)' }}>{logs.length}</strong> / {500}
              </div>
              {spanLabel && (
                <div style={{ fontSize: 11, color: 'var(--text-muted)' }}>
                  {t('aiAnalysis.settings.span')}:{' '}
                  <strong style={{ color: 'var(--text)' }}>{spanLabel}</strong>
                </div>
              )}
              <button
                onClick={handleClearLogs}
                style={{
                  marginLeft: 'auto',
                  background: 'none',
                  border: '1px solid var(--border)',
                  borderRadius: 6,
                  padding: '2px 10px',
                  cursor: 'pointer',
                  fontSize: 11,
                  color: 'var(--text-muted)',
                }}
              >
                {t('aiAnalysis.settings.clearLogs')}
              </button>
            </div>
          </div>
        )}

        {!ai.settings.ai_analysis_enabled && (
          <div style={{ fontSize: 12, color: 'var(--text-dim)', marginTop: 4 }}>
            {t('aiAnalysis.settings.disabledHint')}
          </div>
        )}
      </div>

      {/* ── Charts card ───────────────────────────────────────────────────── */}
      <div className="card">
        <div className="card-title" style={{ marginBottom: 2 }}>
          {t('aiAnalysis.charts.title')}
        </div>
        <div style={{ fontSize: 12, color: 'var(--text-muted)', marginBottom: 14 }}>
          {chartLogs.length > 0
            ? t('aiAnalysis.charts.subtitle', {
                n: String(chartLogs.length),
                span: spanLabel || '—',
              })
            : t('aiAnalysis.charts.noData')}
        </div>

        {chartLogs.length >= 2 ? (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 20 }}>
            {/* Temperature chart */}
            <div>
              <div
                style={{
                  fontSize: 11,
                  fontWeight: 600,
                  color: 'var(--text-dim)',
                  textTransform: 'uppercase',
                  letterSpacing: '0.07em',
                  marginBottom: 6,
                }}
              >
                {t('aiAnalysis.charts.temperature')} (°C)
              </div>
              <LineChart series={[cpuTemps, gpuTemps]} height={100} unit="°" />
              <div
                style={{
                  display: 'flex',
                  justifyContent: 'space-between',
                  fontSize: 10,
                  color: 'var(--text-dim)',
                  marginTop: 2,
                  paddingInline: 36,
                }}
              >
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
                <div
                  style={{
                    fontSize: 11,
                    fontWeight: 600,
                    color: 'var(--text-dim)',
                    textTransform: 'uppercase',
                    letterSpacing: '0.07em',
                    marginBottom: 6,
                  }}
                >
                  {t('aiAnalysis.charts.tdp')} (W)
                </div>
                <LineChart series={[tdpSeries]} height={80} unit="W" />
                <div
                  style={{
                    display: 'flex',
                    justifyContent: 'space-between',
                    fontSize: 10,
                    color: 'var(--text-dim)',
                    marginTop: 2,
                    paddingInline: 36,
                  }}
                >
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
              <div
                style={{
                  fontSize: 11,
                  fontWeight: 600,
                  color: 'var(--text-dim)',
                  textTransform: 'uppercase',
                  letterSpacing: '0.07em',
                  marginBottom: 6,
                }}
              >
                {t('aiAnalysis.charts.usage')} (%)
              </div>
              <LineChart series={[cpuPct, gpuPct]} height={80} unit="%" />
              <div
                style={{
                  display: 'flex',
                  justifyContent: 'space-between',
                  fontSize: 10,
                  color: 'var(--text-dim)',
                  marginTop: 2,
                  paddingInline: 36,
                }}
              >
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
                <div
                  style={{
                    fontSize: 11,
                    fontWeight: 600,
                    color: 'var(--text-dim)',
                    textTransform: 'uppercase',
                    letterSpacing: '0.07em',
                    marginBottom: 8,
                  }}
                >
                  {t('aiAnalysis.charts.topProcesses')}
                </div>
                <ProcessBars processes={topProcs} />
              </div>
            )}
          </div>
        ) : (
          <div
            style={{
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              justifyContent: 'center',
              padding: '32px 0',
              gap: 8,
              color: 'var(--text-dim)',
            }}
          >
            <span style={{ fontSize: 32 }} aria-hidden="true">
              📊
            </span>
            <span style={{ fontSize: 13 }}>{t('aiAnalysis.charts.waitingForData')}</span>
            <span style={{ fontSize: 11 }}>{t('aiAnalysis.charts.enableHint')}</span>
          </div>
        )}
      </div>

      {/* ── Analysis card ─────────────────────────────────────────────────── */}
      <div className="card">
        <div
          style={{
            display: 'flex',
            alignItems: 'flex-start',
            justifyContent: 'space-between',
            gap: 12,
            marginBottom: 12,
          }}
        >
          <div>
            <div className="card-title" style={{ marginBottom: 2 }}>
              {t('aiAnalysis.analysis.title')}
            </div>
            <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>
              {lastAnalysis
                ? t('aiAnalysis.analysis.lastAt', {
                    time: fmtDatetime(lastAnalysis.ts),
                    n: String(lastAnalysis.log_count),
                  })
                : t('aiAnalysis.analysis.neverRun')}
            </div>
          </div>
          <div
            style={{
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'flex-end',
              gap: 4,
              flexShrink: 0,
            }}
          >
            <button
              className="btn-primary"
              style={{ fontSize: 12, minWidth: 100 }}
              onClick={handleAnalyze}
              disabled={analyzing || logs.length < 2}
            >
              {analyzing ? t('aiAnalysis.analysis.analyzing') : t('aiAnalysis.analysis.trigger')}
            </button>
            {lastAnalysis && (
              <div style={{ fontSize: 10, color: 'var(--text-dim)', textAlign: 'right' }}>
                {t('aiAnalysis.analysis.nextIn')}: {nextIn}
              </div>
            )}
          </div>
        </div>

        {analyzeError && (
          <div
            style={{
              padding: '10px 14px',
              background: 'color-mix(in srgb, var(--error, #f44336) 12%, transparent)',
              borderRadius: 'var(--r-sm)',
              borderLeft: '3px solid var(--error, #f44336)',
              fontSize: 12,
              color: 'var(--error, #f44336)',
              marginBottom: 12,
            }}
          >
            {analyzeError}
          </div>
        )}

        {analyzing && (
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 10,
              padding: '12px 0',
              color: 'var(--text-muted)',
              fontSize: 13,
            }}
          >
            <span
              style={{
                display: 'inline-block',
                width: 14,
                height: 14,
                border: '2px solid var(--accent)',
                borderTopColor: 'transparent',
                borderRadius: '50%',
                animation: 'spin 0.8s linear infinite',
              }}
            />
            {t('aiAnalysis.analysis.analyzing')}…
          </div>
        )}

        {lastAnalysis && !analyzing && (
          <div
            style={{
              padding: '14px 16px',
              background: 'var(--surface-2)',
              borderRadius: 'var(--r-sm)',
              borderLeft: '3px solid var(--accent)',
            }}
          >
            <RenderAnalysis text={lastAnalysis.text} />
          </div>
        )}

        {!lastAnalysis && !analyzing && !analyzeError && (
          <div
            style={{
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              justifyContent: 'center',
              padding: '24px 0',
              gap: 8,
              color: 'var(--text-dim)',
            }}
          >
            <span style={{ fontSize: 28 }} aria-hidden="true">
              🤖
            </span>
            <span style={{ fontSize: 12 }}>{t('aiAnalysis.analysis.noResultYet')}</span>
          </div>
        )}
      </div>

      {/* ── AI disclaimer ──────────────────────────────────────────────────── */}
      <div
        style={{
          fontSize: 11,
          color: 'var(--color-text-muted)',
          padding: '8px 14px',
          marginBottom: 12,
          background: 'var(--surface-2, rgba(255,255,255,0.03))',
          borderRadius: 8,
          border: '1px solid var(--color-border, #3a3a4e)',
          lineHeight: 1.5,
        }}
      >
        ⚠️ {t('ai.disclaimer')}
      </div>

      {/* ── AI Usage Panel ─────────────────────────────────────────────────── */}
      <AiUsagePanel />

      {/* ── Log table card ────────────────────────────────────────────────── */}
      <div className="card">
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <div>
            <div className="card-title" style={{ marginBottom: 2 }}>
              {t('aiAnalysis.logs.title')}
            </div>
            <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>
              {t('common.logs', { count: logs.length })}
            </div>
          </div>
          <button
            className="btn-secondary"
            style={{ fontSize: 12 }}
            onClick={() => setShowLogTable((v) => !v)}
          >
            {showLogTable ? t('common.hide') : t('common.show')}
          </button>
        </div>

        {showLogTable && (
          <div style={{ marginTop: 14 }}>
            {logs.length === 0 ? (
              <div
                style={{
                  fontSize: 12,
                  color: 'var(--text-dim)',
                  textAlign: 'center',
                  padding: '16px 0',
                }}
              >
                {t('aiAnalysis.logs.noEntries')}
              </div>
            ) : (
              <>
                <div style={{ overflowX: 'auto' }}>
                  <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 11 }}>
                    <thead>
                      <tr
                        style={{
                          color: 'var(--text-muted)',
                          borderBottom: '1px solid var(--border)',
                        }}
                      >
                        <th style={{ textAlign: 'left', padding: '4px 6px', fontWeight: 500 }}>
                          {t('aiAnalysis.logs.time')}
                        </th>
                        <th style={{ textAlign: 'left', padding: '4px 6px', fontWeight: 500 }}>
                          {t('aiAnalysis.logs.mode')}
                        </th>
                        <th style={{ textAlign: 'right', padding: '4px 6px', fontWeight: 500 }}>
                          CPU°C
                        </th>
                        <th style={{ textAlign: 'right', padding: '4px 6px', fontWeight: 500 }}>
                          GPU°C
                        </th>
                        <th style={{ textAlign: 'right', padding: '4px 6px', fontWeight: 500 }}>
                          TDP W
                        </th>
                        <th style={{ textAlign: 'right', padding: '4px 6px', fontWeight: 500 }}>
                          CPU%
                        </th>
                        <th style={{ textAlign: 'right', padding: '4px 6px', fontWeight: 500 }}>
                          GPU%
                        </th>
                        <th style={{ textAlign: 'right', padding: '4px 6px', fontWeight: 500 }}>
                          🔋%
                        </th>
                        <th
                          style={{
                            textAlign: 'center',
                            padding: '4px 6px',
                            fontWeight: 500,
                            width: 30,
                          }}
                        ></th>
                      </tr>
                    </thead>
                    <tbody>
                      {logs
                        .slice(-logTableCount)
                        .reverse()
                        .map((e, i) => (
                          <tr
                            key={i}
                            style={{ borderBottom: '1px solid var(--border-faint, var(--border))' }}
                          >
                            <td
                              style={{
                                padding: '3px 6px',
                                fontFamily: 'var(--font-mono)',
                                color: 'var(--text-dim)',
                              }}
                            >
                              {fmtTime(e.ts)}
                            </td>
                            <td style={{ padding: '3px 6px', color: 'var(--accent)' }}>{e.mode}</td>
                            <td
                              style={{
                                padding: '3px 6px',
                                textAlign: 'right',
                                fontFamily: 'var(--font-mono)',
                              }}
                            >
                              {e.cpu_temp.toFixed(0)}
                            </td>
                            <td
                              style={{
                                padding: '3px 6px',
                                textAlign: 'right',
                                fontFamily: 'var(--font-mono)',
                              }}
                            >
                              {e.gpu_temp.toFixed(0)}
                            </td>
                            <td
                              style={{
                                padding: '3px 6px',
                                textAlign: 'right',
                                fontFamily: 'var(--font-mono)',
                              }}
                            >
                              {e.tdp_watts != null ? e.tdp_watts.toFixed(1) : '—'}
                            </td>
                            <td
                              style={{
                                padding: '3px 6px',
                                textAlign: 'right',
                                fontFamily: 'var(--font-mono)',
                              }}
                            >
                              {e.cpu_pct.toFixed(0)}
                            </td>
                            <td
                              style={{
                                padding: '3px 6px',
                                textAlign: 'right',
                                fontFamily: 'var(--font-mono)',
                              }}
                            >
                              {e.gpu_pct.toFixed(0)}
                            </td>
                            <td
                              style={{
                                padding: '3px 6px',
                                textAlign: 'right',
                                fontFamily: 'var(--font-mono)',
                              }}
                            >
                              {e.battery_level != null ? e.battery_level.toFixed(0) : '—'}
                            </td>
                            <td style={{ padding: '3px 4px', textAlign: 'center' }}>
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
                    className="btn-secondary"
                    style={{ marginTop: 10, fontSize: 11 }}
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
