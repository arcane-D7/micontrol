import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import type { Hardware } from './shared';
import type { NetworkInterfaceSample, ProcessTaskInfo } from '../../types/hardware';

interface Props {
  hw: Hardware;
}

type SortKey = 'cpu' | 'gpu' | 'npu' | 'ram' | 'name' | 'pid';

const SORT_LABELS: Record<SortKey, string> = {
  cpu: 'CPU',
  gpu: 'GPU',
  npu: 'NPU',
  ram: 'RAM',
  name: 'Name',
  pid: 'PID',
};

const fmtBps = (b: number) => {
  if (b <= 0) return '—';
  if (b >= 1024 * 1024) return `${(b / (1024 * 1024)).toFixed(1)} MB/s`;
  if (b >= 1024) return `${(b / 1024).toFixed(0)} KB/s`;
  return `${b.toFixed(0)} B/s`;
};

const fmtMb = (mb: number) => {
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
  return `${mb.toFixed(0)} MB`;
};

/**
 * Global Task Manager tab:
 *  - live per-process CPU / GPU / NPU / RAM columns
 *  - filter box (by name / pid)
 *  - click-to-sort on every column (asc/desc)
 *  - kill process button (with confirmation)
 *
 * There is no per-process network column: Windows only exposes per-PID byte
 * counters through an ETW kernel session (administrator) and the old field was
 * a hardcoded 0.0, so the column could only ever show "—" (audit F7.3). The
 * interface-level throughput is still shown in the summary strip.
 */
function TaskManagerTab({ hw }: Props) {
  const [processes, setProcesses] = useState<ProcessTaskInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [sortKey, setSortKey] = useState<SortKey>('cpu');
  const [sortDesc, setSortDesc] = useState(true);
  const [filter, setFilter] = useState('');
  const [killing, setKilling] = useState<number | null>(null);
  const [killError, setKillError] = useState<string | null>(null);
  const [network, setNetwork] = useState<NetworkInterfaceSample[]>([]);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const load = useCallback(async () => {
    const [procs, net] = await Promise.all([hw.getTaskManager(), hw.getNetworkPerf()]);
    setProcesses(procs);
    setNetwork(net);
    setLoading(false);
  }, [hw]);

  useEffect(() => {
    void load();
    timerRef.current = setInterval(() => void load(), 2500);
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [load]);

  const toggleSort = (key: SortKey) => {
    if (sortKey === key) {
      setSortDesc((d) => !d);
    } else {
      setSortKey(key);
      setSortDesc(key === 'name' || key === 'pid');
    }
  };

  const rows = useMemo(() => {
    let list = processes;
    const q = filter.trim().toLowerCase();
    if (q) {
      list = list.filter((p) => p.name.toLowerCase().includes(q) || String(p.pid).includes(q));
    }
    const dir = sortDesc ? -1 : 1;
    return [...list].sort((a, b) => {
      let r = 0;
      switch (sortKey) {
        case 'cpu':
          r = a.cpu_percent - b.cpu_percent;
          break;
        case 'gpu':
          r = a.gpu_percent - b.gpu_percent;
          break;
        case 'npu':
          r = a.npu_percent - b.npu_percent;
          break;
        case 'ram':
          r = a.memory_mb - b.memory_mb;
          break;
        case 'pid':
          r = a.pid - b.pid;
          break;
        default:
          r = a.name.localeCompare(b.name);
      }
      return r * dir;
    });
  }, [processes, sortKey, sortDesc, filter]);

  const totalNpu = useMemo(
    () => processes.reduce((s, p) => s + (p.npu_percent || 0), 0),
    [processes],
  );

  const totalNet = useMemo(() => network.reduce((s, n) => s + n.bytes_per_sec, 0), [network]);

  const doKill = async (pid: number, name: string) => {
    // Name what is lost, not just "are you sure" (audit #6 / Nielsen #5).
    if (!window.confirm(t('taskmgr.endProcessConfirm', { name, pid }))) return;
    setKilling(pid);
    setKillError(null);
    try {
      await hw.killProcess(pid);
      // Force immediate refresh.
      await load();
    } catch (e) {
      console.error('kill failed:', e);
      setKillError(String(e));
    } finally {
      setKilling(null);
    }
  };

  /**
   * Sortable column header.
   *
   * P0-2 (audit 2026-11-07): `<th onClick>` was unreachable by keyboard and
   * carried no sort state. The interactive element is now a real <button>
   * (Enter/Space free, native focus ring), `aria-sort` lives on the <th> as
   * the spec requires, and the glyph is aria-hidden so it is not read out.
   */
  const sortableHeader = (key: SortKey, label: string, align: 'left' | 'right' = 'right') => {
    const active = sortKey === key;
    return (
      <th
        scope="col"
        aria-sort={active ? (sortDesc ? 'descending' : 'ascending') : 'none'}
        style={{
          textAlign: align,
          padding: 0,
          color: 'var(--color-text-muted)',
          fontWeight: 600,
          fontSize: 11,
          textTransform: 'uppercase',
          letterSpacing: '0.04em',
          whiteSpace: 'nowrap',
        }}
      >
        <button
          type="button"
          className="th-sort"
          onClick={() => toggleSort(key)}
          aria-label={t('taskmgr.sortBy', { column: label })}
          title={t('taskmgr.sortBy', { column: label })}
          style={{ justifyContent: align === 'right' ? 'flex-end' : 'flex-start' }}
        >
          {label}
          <span className="th-sort-glyph" aria-hidden="true">
            {active ? (sortDesc ? '↓' : '↑') : ''}
          </span>
        </button>
      </th>
    );
  };

  return (
    <>
      <PageHeader title={t('taskmgr.title')} />

      {/* Summary strip */}
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(150px, 1fr))',
          gap: 10,
          marginBottom: 12,
        }}
      >
        {(['cpu', 'gpu', 'npu'] as const).map((k) => (
          <div key={k} className="card" style={{ padding: '10px 14px' }}>
            <div style={{ fontSize: 11, color: 'var(--color-text-muted)' }}>{SORT_LABELS[k]} %</div>
            <div style={{ fontSize: 22, fontWeight: 700, fontVariantNumeric: 'tabular-nums' }}>
              {k === 'cpu'
                ? (hw.systemInfo?.cpu_usage?.toFixed(1) ?? '—')
                : k === 'gpu'
                  ? (hw.systemInfo?.gpu_usage?.toFixed(1) ?? '—')
                  : totalNpu.toFixed(1) || '—'}
            </div>
            <div
              style={{
                height: 4,
                borderRadius: 2,
                background: 'var(--color-border, rgba(255,255,255,0.08))',
                marginTop: 6,
                overflow: 'hidden',
              }}
            >
              <div
                style={{
                  width: `${Math.min(100, k === 'cpu' ? (hw.systemInfo?.cpu_usage ?? 0) : k === 'gpu' ? (hw.systemInfo?.gpu_usage ?? 0) : totalNpu)}%`,
                  height: '100%',
                  background: k === 'gpu' ? '#22c55e' : k === 'npu' ? '#a855f7' : '#3b82f6',
                  borderRadius: 2,
                  transition: 'width 300ms',
                }}
              />
            </div>
          </div>
        ))}
        <div className="card" style={{ padding: '10px 14px' }}>
          <div style={{ fontSize: 11, color: 'var(--color-text-muted)' }}>NET ↓↑</div>
          <div style={{ fontSize: 22, fontWeight: 700, fontVariantNumeric: 'tabular-nums' }}>
            {fmtBps(totalNet)}
          </div>
          <div
            style={{
              fontSize: 11,
              color: 'var(--color-text-muted)',
              marginTop: 4,
              whiteSpace: 'nowrap',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
            }}
          >
            {network.map((n) => n.name).join(' · ') || t('taskmgr.noTraffic')}
          </div>
        </div>
      </div>

      {/* Filter + refresh */}
      <div style={{ display: 'flex', gap: 8, alignItems: 'center', marginBottom: 10 }}>
        <input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder={t('taskmgr.filterPlaceholder')}
          aria-label={t('taskmgr.filterLabel')}
          style={{
            flex: 1,
            padding: '7px 10px',
            borderRadius: 8,
            border: '1px solid var(--color-border, rgba(255,255,255,0.12))',
            background: 'var(--color-surface, #1e1e2e)',
            color: 'var(--color-text)',
            fontSize: 13,
          }}
        />
        <button
          onClick={() => void load()}
          style={{
            padding: '7px 12px',
            borderRadius: 8,
            border: '1px solid var(--color-border, rgba(255,255,255,0.12))',
            background: 'var(--color-surface, #1e1e2e)',
            color: 'var(--color-text)',
            fontSize: 13,
            cursor: 'pointer',
          }}
        >
          ↻ {t('taskmgr.refresh')}
        </button>
      </div>

      {killError && (
        <div
          role="alert"
          style={{
            padding: '8px 12px',
            marginBottom: 8,
            borderRadius: 8,
            background: 'oklch(from var(--error) l c h / 0.12)',
            color: 'var(--error)',
            fontSize: 12,
          }}
        >
          {t('taskmgr.killFailed')}: {killError}
        </div>
      )}

      <div
        className="card"
        style={{ padding: 0, overflow: 'auto', maxHeight: 'calc(100vh - 320px)' }}
      >
        <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 12.5 }}>
          <caption className="sr-only">{t('nav.taskmgr')}</caption>
          <thead
            style={{ position: 'sticky', top: 0, background: 'var(--color-surface, #1e1e2e)' }}
          >
            <tr style={{ borderBottom: '1px solid var(--color-border, rgba(255,255,255,0.1))' }}>
              {sortableHeader('name', t('taskmgr.colProcess'), 'left')}
              {sortableHeader('pid', t('taskmgr.colPid'))}
              {sortableHeader('cpu', t('taskmgr.colCpu'))}
              {sortableHeader('gpu', t('taskmgr.colGpu'))}
              {sortableHeader('npu', t('taskmgr.colNpu'))}
              {sortableHeader('ram', t('taskmgr.colRam'))}
              <th
                scope="col"
                style={{
                  padding: '6px 8px',
                  textAlign: 'right',
                  color: 'var(--color-text-muted)',
                  fontWeight: 600,
                  fontSize: 11,
                  textTransform: 'uppercase',
                  letterSpacing: '0.04em',
                  whiteSpace: 'nowrap',
                }}
              >
                {t('taskmgr.colThreads')}
              </th>
              <th scope="col" style={{ padding: '6px 8px', width: 56 }}>
                <span className="sr-only">{t('taskmgr.colActions')}</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {loading && rows.length === 0 ? (
              <tr>
                <td
                  colSpan={7}
                  style={{ textAlign: 'center', padding: 28, color: 'var(--color-text-muted)' }}
                >
                  {t('common.loading')}…
                </td>
              </tr>
            ) : rows.length === 0 ? (
              <tr>
                <td
                  colSpan={7}
                  style={{ textAlign: 'center', padding: 28, color: 'var(--color-text-muted)' }}
                >
                  {filter ? t('taskmgr.noMatches') : t('taskmgr.noProcesses')}
                </td>
              </tr>
            ) : (
              rows.map((p) => (
                <tr
                  key={p.pid}
                  style={{
                    borderBottom: '1px solid var(--color-border, rgba(255,255,255,0.05))',
                  }}
                >
                  <td
                    style={{
                      padding: '5px 8px',
                      maxWidth: 260,
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {p.name}
                  </td>
                  <td
                    style={{
                      padding: '5px 8px',
                      textAlign: 'right',
                      color: 'var(--color-text-muted)',
                      fontVariantNumeric: 'tabular-nums',
                    }}
                  >
                    {p.pid}
                  </td>
                  <td
                    style={{
                      padding: '5px 8px',
                      textAlign: 'right',
                      fontVariantNumeric: 'tabular-nums',
                    }}
                  >
                    {p.cpu_percent > 0 ? `${p.cpu_percent.toFixed(1)}%` : '—'}
                  </td>
                  <td
                    style={{
                      padding: '5px 8px',
                      textAlign: 'right',
                      fontVariantNumeric: 'tabular-nums',
                    }}
                  >
                    {p.gpu_percent > 0.05 ? `${p.gpu_percent.toFixed(1)}%` : '—'}
                  </td>
                  <td
                    style={{
                      padding: '5px 8px',
                      textAlign: 'right',
                      fontVariantNumeric: 'tabular-nums',
                    }}
                  >
                    {p.npu_percent > 0.05 ? `${p.npu_percent.toFixed(1)}%` : '—'}
                  </td>
                  <td
                    style={{
                      padding: '5px 8px',
                      textAlign: 'right',
                      fontVariantNumeric: 'tabular-nums',
                    }}
                  >
                    {fmtMb(p.memory_mb)}
                  </td>
                  <td
                    style={{
                      padding: '5px 8px',
                      textAlign: 'right',
                      fontVariantNumeric: 'tabular-nums',
                      color: 'var(--color-text-muted)',
                    }}
                  >
                    {p.thread_count || '—'}
                  </td>
                  <td style={{ padding: '5px 8px', textAlign: 'right' }}>
                    <button
                      onClick={() => void doKill(p.pid, p.name)}
                      disabled={killing === p.pid}
                      aria-label={t('taskmgr.endProcess', { name: p.name })}
                      title={t('taskmgr.endProcess', { name: p.name })}
                      style={{
                        background: 'oklch(from var(--error) l c h / 0.12)',
                        /* One step darker than --error so the ✕ clears AA on
                           its own 12% tint in light mode (3.7:1 → 5.2:1). */
                        color: 'oklch(from var(--error) calc(l - 0.07) c h)',
                        border: 'none',
                        borderRadius: 6,
                        padding: '3px 8px',
                        fontSize: 11,
                        minWidth: 24,
                        minHeight: 24,
                        cursor: 'pointer',
                      }}
                    >
                      <span aria-hidden="true">{killing === p.pid ? '…' : '✕'}</span>
                    </button>
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>

      <div style={{ marginTop: 8, fontSize: 11, color: 'var(--color-text-muted)' }}>
        {t('taskmgr.footer', { count: rows.length })}
      </div>
    </>
  );
}

export default memo(TaskManagerTab);
