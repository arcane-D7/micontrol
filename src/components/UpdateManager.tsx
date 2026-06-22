import { t } from '../hooks/useI18n';
import type { UpdateStatus } from '../hooks/useHardware';

interface Props {
  updateStatus: UpdateStatus | null;
  loadingUpdate: boolean;
  onRefreshUpdate: () => void;
}

export default function UpdateManager({ updateStatus, loadingUpdate, onRefreshUpdate }: Props) {
  if (loadingUpdate && !updateStatus) {
    return (
      <div className="card">
        <div className="loading-spinner">{t('common.loading')}</div>
      </div>
    );
  }

  const bios = updateStatus?.bios;
  const drivers = updateStatus?.xiaomi_drivers ?? [];

  return (
    <>
      {/* BIOS Information */}
      <div className="card">
        <div className="card-title">{t('updates.biosSection')}</div>
        <div className="stat-grid">
          <div className="stat-item">
            <span className="stat-label">{t('updates.biosVersion')}</span>
            <span className="stat-value">{bios?.version || t('common.unknown')}</span>
          </div>
          <div className="stat-item">
            <span className="stat-label">{t('updates.biosDate')}</span>
            <span className="stat-value">{bios?.release_date || t('common.unknown')}</span>
          </div>
          <div className="stat-item">
            <span className="stat-label">{t('updates.biosMfg')}</span>
            <span className="stat-value">{bios?.manufacturer || t('common.unknown')}</span>
          </div>
        </div>
      </div>

      {/* Installed Xiaomi drivers */}
      <div className="card">
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            marginBottom: 14,
          }}
        >
          <div className="card-title" style={{ margin: 0 }}>
            {t('updates.driversSection')}
          </div>
          <button
            className="btn-secondary"
            onClick={onRefreshUpdate}
            disabled={loadingUpdate}
            style={{ fontSize: 12, padding: '4px 12px' }}
          >
            {loadingUpdate ? t('common.loading') : '\u2935 ' + t('updates.refresh')}
          </button>
        </div>

        {drivers.length === 0 ? (
          <p style={{ color: 'var(--text-muted)', fontSize: '0.85rem' }}>
            {t('updates.noXiaomiDrivers')}
          </p>
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            {drivers.map((d) => (
              <div
                key={d.published_name}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 12,
                  padding: '9px 12px',
                  borderRadius: 'var(--r-sm)',
                  background: 'var(--surface-2)',
                  border: '1px solid var(--border)',
                }}
              >
                <div
                  style={{
                    width: 22,
                    height: 22,
                    borderRadius: '50%',
                    flexShrink: 0,
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    background: 'oklch(from var(--success) l c h / 0.15)',
                    color: 'var(--success)',
                    fontSize: 12,
                    fontWeight: 700,
                  }}
                >
                  &#10003;
                </div>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div
                    style={{
                      fontSize: 12.5,
                      fontWeight: 600,
                      fontFamily: 'var(--font-mono)',
                      color: 'var(--text)',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {d.original_name}
                  </div>
                  <div style={{ fontSize: 11, color: 'var(--text-dim)', marginTop: 1 }}>
                    {d.provider} {d.version_string}
                  </div>
                </div>
                <div
                  style={{
                    fontSize: 10.5,
                    fontWeight: 600,
                    color: 'var(--success)',
                    flexShrink: 0,
                    background: 'oklch(from var(--success) l c h / 0.12)',
                    padding: '2px 8px',
                    borderRadius: 99,
                    border: '1px solid oklch(from var(--success) l c h / 0.25)',
                  }}
                >
                  {t('updates.driverInstalled')}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </>
  );
}
