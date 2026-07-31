import { memo, useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import { useToast } from '../../contexts/ToastContext';
import UpdateManager from '../../components/UpdateManager';
import type { Hardware } from './shared';
import type { AppUpdateState, AppUpdateInfo } from '../../hooks/useAutoUpdate';

// ── Driver detail types ──────────────────────────────────────────────────────

interface DriverDetail {
  device_name: string;
  driver_version: string;
  driver_date: string;
  manufacturer: string;
  status: string;
}

interface Props {
  hw: Hardware;
  appUpdateState: AppUpdateState;
  appUpdateInfo: AppUpdateInfo | null;
  appUpdateProgress: number;
  appUpdateError: string;
  onCheckAppUpdate: () => void;
  onInstallAppUpdate: () => void;
  onDismissAppUpdate: () => void;
}

/// Format a WMI CIM datetime (e.g. "20250514000000.000000-000") to a
/// human-readable date string (YYYY-MM-DD). Returns the original string
/// if parsing fails.
function formatDriverDate(raw: string): string {
  if (!raw || raw.length < 8) return raw || '—';
  // WMI CIM datetime format: YYYYMMDDHHMMSS.mmmmmm+UUU
  const match = raw.match(/^(\d{4})(\d{2})(\d{2})/);
  if (match) {
    return `${match[1]}-${match[2]}-${match[3]}`;
  }
  return raw;
}

function UpdatesTab({
  hw,
  appUpdateState,
  appUpdateInfo,
  appUpdateProgress,
  appUpdateError,
  onCheckAppUpdate,
  onInstallAppUpdate,
  onDismissAppUpdate,
}: Props) {
  const [drivers, setDrivers] = useState<DriverDetail[] | null>(null);
  const [loadingDrivers, setLoadingDrivers] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const { addToast } = useToast();

  const fetchDrivers = useCallback(async () => {
    setLoadingDrivers(true);
    setErrorMsg(null);
    try {
      const details = await invoke<DriverDetail[]>('get_drivers_detail');
      setDrivers(details);
    } catch (e) {
      const msg =
        typeof e === 'object' && e !== null && 'message' in e
          ? String((e as { message: unknown }).message)
          : String(e);
      setErrorMsg(msg);
    }
    setLoadingDrivers(false);
  }, []);

  const handleScan = async () => {
    setScanning(true);
    setErrorMsg(null);
    try {
      await invoke('trigger_driver_scan');
      await fetchDrivers();
      addToast({ message: t('updates.scanComplete'), type: 'success' });
    } catch (e) {
      const msg =
        typeof e === 'object' && e !== null && 'message' in e
          ? String((e as { message: unknown }).message)
          : String(e);
      setErrorMsg(msg);
      addToast({ message: `${t('updates.scanError')}: ${msg}`, type: 'error' });
    }
    setScanning(false);
  };

  useEffect(() => {
    void fetchDrivers();
  }, [fetchDrivers]);

  return (
    <>
      <PageHeader title={t('updates.title')} subtitle={t('updates.subtitle')} />
      <UpdateManager
        updateStatus={hw.updateStatus}
        loadingUpdate={hw.loadingUpdate}
        onRefreshUpdate={hw.refreshUpdateStatus}
        appUpdateState={appUpdateState}
        appUpdateInfo={appUpdateInfo}
        appUpdateProgress={appUpdateProgress}
        appUpdateError={appUpdateError}
        onCheckAppUpdate={onCheckAppUpdate}
        onInstallAppUpdate={onInstallAppUpdate}
        onDismissAppUpdate={onDismissAppUpdate}
      />

      {/* Driver Details Card */}
      <div className="card" style={{ marginTop: 16 }}>
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            marginBottom: 12,
          }}
        >
          <h3>🔧 {t('updates.driverDetails')}</h3>
          <div style={{ display: 'flex', gap: 8 }}>
            <button className="btn btn-secondary" onClick={handleScan} disabled={scanning}>
              {scanning ? `🔍 ${t('updates.scanning')}` : `🔍 ${t('updates.scanDrivers')}`}
            </button>
            <button className="btn btn-secondary" onClick={fetchDrivers} disabled={loadingDrivers}>
              {loadingDrivers ? t('common.loading') : `🔄 ${t('updates.refreshDrivers')}`}
            </button>
          </div>
        </div>

        {errorMsg && (
          <div className="alert alert-error" style={{ marginBottom: 12 }}>
            ⚠ {errorMsg}
          </div>
        )}

        {loadingDrivers && !drivers ? (
          <p className="text-muted">{t('common.loading')}</p>
        ) : drivers && drivers.length > 0 ? (
          <div style={{ overflowX: 'auto' }}>
            <table style={{ width: '100%', fontSize: 12, borderCollapse: 'collapse' }}>
              <thead>
                <tr style={{ textAlign: 'left', borderBottom: '1px solid var(--border)' }}>
                  <th style={{ padding: '8px 4px' }}>{t('updates.driverDevice')}</th>
                  <th style={{ padding: '8px 4px' }}>{t('updates.driverVersion')}</th>
                  <th style={{ padding: '8px 4px' }}>{t('updates.driverDate')}</th>
                  <th style={{ padding: '8px 4px' }}>{t('updates.driverManufacturer')}</th>
                  <th style={{ padding: '8px 4px' }}>{t('updates.driverStatus')}</th>
                </tr>
              </thead>
              <tbody>
                {drivers.map((d, i) => (
                  <tr key={i} style={{ borderBottom: '1px solid var(--border)' }}>
                    <td style={{ padding: '8px 4px' }}>
                      {d.device_name || <span className="text-muted">—</span>}
                    </td>
                    <td style={{ padding: '8px 4px' }}>{d.driver_version || '—'}</td>
                    <td style={{ padding: '8px 4px' }}>{formatDriverDate(d.driver_date)}</td>
                    <td style={{ padding: '8px 4px' }}>{d.manufacturer || '—'}</td>
                    <td style={{ padding: '8px 4px' }}>
                      <span className={d.status === 'OK' ? 'status-ok' : 'status-warn'}>
                        {d.status || 'Unknown'}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <p className="text-muted">{t('updates.noDrivers')}</p>
        )}
      </div>
    </>
  );
}

export default memo(UpdatesTab);
