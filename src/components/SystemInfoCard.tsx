import { useState } from 'react';
import { t } from '../hooks/useI18n';
import type { SystemInfo, ProcessInfo } from '../hooks/useHardware';
import ProcessModal from './ProcessModal';

interface Props {
  info: SystemInfo | null;
  getProcessList?: () => Promise<ProcessInfo[]>;
}

export default function SystemInfoCard({ info, getProcessList }: Props) {
  const [processResource, setProcessResource] = useState<'cpu' | 'gpu' | 'ram' | null>(null);

  if (!info) {
    return (
      <div className="card">
        <div className="card-title">{t('overview.system')}</div>
        <div className="skeleton" style={{ height: 120 }} />
      </div>
    );
  }

  const cpuUsageColor = info.cpu_usage > 80 ? 'temp' : 'cpu';
  const gpuUsageColor = info.gpu_usage > 80 ? 'temp' : 'gpu';
  const ramUsedPct = (info.ram_used_gb / info.ram_total_gb) * 100;

  const PlusBtn = ({ resource }: { resource: 'cpu' | 'gpu' | 'ram' }) => (
    <button
      title="Show top processes"
      onClick={() => setProcessResource(resource)}
      style={{
        background: 'none',
        border: 'none',
        cursor: 'pointer',
        color: 'var(--color-text-dim)',
        fontSize: 13,
        lineHeight: 1,
        padding: '0 2px',
        marginLeft: 4,
        opacity: 0.7,
      }}
    >
      ⊕
    </button>
  );

  return (
    <div className="card">
      <div className="card-title">{t('overview.system')}</div>

      <div className="stat-row">
        <span className="stat-label">{t('overview.cpu')}</span>
        <span className="stat-value">{info.cpu_name}</span>
      </div>
      <div style={{ marginBottom: 12 }}>
        <div className="stat-row" style={{ paddingBottom: 4 }}>
          <span className="stat-label" style={{ display: 'flex', alignItems: 'center' }}>
            {t('overview.cpuUsage')}
            {getProcessList && <PlusBtn resource="cpu" />}
          </span>
          <span className="stat-value">{info.cpu_usage.toFixed(1)}%</span>
        </div>
        <div className="progress-bar">
          <div
            className={`progress-fill ${cpuUsageColor}`}
            style={{ width: `${info.cpu_usage}%` }}
          />
        </div>
      </div>

      <div className="stat-row">
        <span className="stat-label">{t('overview.gpu')}</span>
        <span className="stat-value">{info.gpu_name}</span>
      </div>
      <div style={{ marginBottom: 12 }}>
        <div className="stat-row" style={{ paddingBottom: 4 }}>
          <span className="stat-label" style={{ display: 'flex', alignItems: 'center' }}>
            GPU Usage
            {getProcessList && <PlusBtn resource="gpu" />}
          </span>
          <span className="stat-value">{info.gpu_usage.toFixed(1)}%</span>
        </div>
        <div className="progress-bar">
          <div
            className={`progress-fill ${gpuUsageColor}`}
            style={{ width: `${info.gpu_usage}%` }}
          />
        </div>
      </div>

      <div style={{ marginBottom: 4 }}>
        <div className="stat-row" style={{ paddingBottom: 4 }}>
          <span className="stat-label" style={{ display: 'flex', alignItems: 'center' }}>
            {t('overview.ram')}
            {getProcessList && <PlusBtn resource="ram" />}
          </span>
          <span className="stat-value">
            {info.ram_used_gb.toFixed(1)} / {info.ram_total_gb.toFixed(0)} GB
          </span>
        </div>
        <div className="progress-bar">
          <div className="progress-fill ram" style={{ width: `${ramUsedPct}%` }} />
        </div>
      </div>

      <div className="stat-row" style={{ marginTop: 8 }}>
        <span className="stat-label">{t('overview.cores')}</span>
        <span className="stat-value">
          {info.cpu_cores}C / {info.cpu_threads}T
        </span>
      </div>

      {processResource && getProcessList && (
        <ProcessModal
          open
          onClose={() => setProcessResource(null)}
          resource={processResource}
          getProcessList={getProcessList}
        />
      )}
    </div>
  );
}
