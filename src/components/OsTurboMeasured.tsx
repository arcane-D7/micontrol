import { t } from '../hooks/useI18n';

/**
 * Truthful readback counts from the OS Turbo worker, as returned by
 * `get_os_turbo`. Every number comes from a real `GetProcessInformation`
 * readback in the backend — nothing here is estimated or extrapolated.
 *
 * - `inspected`: allowlisted background processes examined in the last sweep
 * - `applied`: of those, verified throttled (EcoQoS)
 * - `pending`: alive but not throttled yet (work still to do)
 * - `restored`: verified back in their captured state
 * - `skipped`: already exited (terminated), never a failure
 * - `failures`: live processes that could not be opened/read
 */
export interface OsTurboMeasuredData {
  inspected: number;
  applied: number;
  pending: number;
  restored: number;
  skipped: number;
  failures: number;
  last_sweep_ms: number;
  worker_active: boolean;
}

/**
 * "N of M throttled" for a status. Prefers the measured sweep; before the
 * first sweep only `throttled_processes` exists, so it is used as both sides
 * rather than claiming "0 of 0" while processes are in fact throttled.
 */
export function osTurboCounts(
  status?: { throttled_processes?: number; measured?: OsTurboMeasuredData } | null,
): { applied: number; inspected: number } {
  const measured = status?.measured;
  if (measured && measured.inspected > 0) {
    return { applied: measured.applied, inspected: measured.inspected };
  }
  const fallback = status?.throttled_processes ?? 0;
  return { applied: fallback, inspected: fallback };
}

/**
 * The "measured results" quadro, shared by the Performance and System
 * Optimization tabs so the two views can never disagree about what OS Turbo
 * is doing. Renders nothing when no sweep has been reported yet.
 */
export function OsTurboMeasuredBlock({ measured }: { measured?: OsTurboMeasuredData }) {
  if (!measured) return null;

  const cells: [string, number][] = [
    [t('sysOpt.osTurboMeasuredInspected'), measured.inspected],
    [t('sysOpt.osTurboMeasuredApplied'), measured.applied],
    [t('sysOpt.osTurboMeasuredPending'), measured.pending],
    [t('sysOpt.osTurboMeasuredRestored'), measured.restored],
    [t('sysOpt.osTurboMeasuredSkipped'), measured.skipped],
    [t('sysOpt.osTurboMeasuredFailures'), measured.failures],
  ];

  const sweepAgeSeconds =
    measured.last_sweep_ms > 0
      ? Math.max(0, Math.round((Date.now() - measured.last_sweep_ms) / 1000))
      : null;

  return (
    <div
      style={{
        marginTop: 14,
        paddingTop: 12,
        borderTop: '1px solid var(--border)',
      }}
    >
      <div
        style={{
          fontSize: 11,
          fontWeight: 600,
          color: 'var(--text-dim)',
          textTransform: 'uppercase',
          letterSpacing: '0.08em',
          marginBottom: 8,
        }}
      >
        {t('sysOpt.osTurboMeasuredTitle')}
      </div>
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(110px, 1fr))',
          gap: '8px 14px',
        }}
      >
        {cells.map(([label, value]) => (
          <div key={label}>
            <div style={{ fontSize: 10, color: 'var(--text-muted)' }}>{label}</div>
            <div
              style={{
                fontSize: 15,
                fontWeight: 600,
                fontVariantNumeric: 'tabular-nums',
                color: 'var(--text)',
              }}
            >
              {value}
            </div>
          </div>
        ))}
      </div>
      <div
        style={{
          marginTop: 10,
          fontSize: 11,
          color: 'var(--text-dim)',
          display: 'flex',
          gap: 12,
          flexWrap: 'wrap',
        }}
      >
        <span>
          {measured.worker_active ? t('sysOpt.osTurboWorkerActive') : t('sysOpt.osTurboWorkerIdle')}
        </span>
        <span>
          {sweepAgeSeconds !== null
            ? t('sysOpt.osTurboSweepAgo', { seconds: sweepAgeSeconds })
            : t('sysOpt.osTurboSweepNever')}
        </span>
      </div>
    </div>
  );
}
