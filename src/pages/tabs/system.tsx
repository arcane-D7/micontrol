import { memo, useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import { useToast } from '../../contexts/ToastContext';
import HardwareDiscovery from '../../components/HardwareDiscovery';
import type { useHardware } from '../../hooks/useHardware';
import IotModulePanel from '../../components/IotModulePanel';
import { Icon } from '../../components/ui/icons';
import type { Hardware } from './shared';

type HardwareInstance = ReturnType<typeof useHardware>;

// ── Driver detail types ──────────────────────────────────────────────────────

interface DriverDetail {
  device_name: string;
  driver_version: string;
  driver_date: string;
  manufacturer: string;
  status: string;
}

/// Format a WMI CIM datetime (e.g. "20250514000000.000000-000") to a
/// human-readable date string (YYYY-MM-DD). Returns the original string
/// if parsing fails.
function formatDriverDate(raw: string): string {
  if (!raw || raw.length < 8) return raw || '—';
  const match = raw.match(/^(\d{4})(\d{2})(\d{2})/);
  if (match) {
    return `${match[1]}-${match[2]}-${match[3]}`;
  }
  return raw;
}

// ── Main System Tab ──────────────────────────────────────────────────────────

interface Props {
  hw: Hardware;
}

// ── Smart brightness model — honest status ──────────────────────────────────
//
// The learned model is only applied when the adaptive loop runs in "smart"
// mode. Production currently holds 295,919 samples across 201 mature buckets
// while `AiBrightnessMode` is 0 ("curve"): the fixed min/max formula drives
// brightness and every learned sample is inert. The Display tab's "Learned
// model" panel reports the sample count without saying that, which reads as
// "the app is learning for you". This card states the active mode and what it
// actually means, with live numbers instead of a hardcoded figure.
function SmartModelStatusCard({ hw }: { hw: HardwareInstance }) {
  const [stats, setStats] = useState<{ total: number; buckets: number } | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const res = await invoke<{ total_samples?: number; mature_buckets?: number }>(
          'get_smart_brightness_model',
        );
        if (!cancelled) {
          setStats({ total: res.total_samples ?? 0, buckets: res.mature_buckets ?? 0 });
        }
      } catch {
        if (!cancelled) setFailed(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const mode = hw.display?.ai_brightness_config?.mode ?? null;

  return (
    <div className="card" style={{ marginTop: 16 }}>
      <h2 className="card-title" style={{ marginBottom: 6 }}>
        <Icon name="brain" size={16} /> {t('system.smartModelTitle')}
      </h2>
      {failed ? (
        <p className="page-subtitle" style={{ margin: 0 }}>
          {t('system.smartModelUnavailable')}
        </p>
      ) : (
        <>
          <p className="page-subtitle" style={{ margin: '0 0 6px' }}>
            {t('system.smartModelSamples', {
              count: stats?.total ?? 0,
              buckets: String(stats?.buckets ?? 0),
            })}
          </p>
          <p
            style={{
              margin: 0,
              fontSize: 12,
              lineHeight: 1.5,
              color: mode === 'smart' ? 'var(--text-muted)' : 'var(--warning)',
            }}
          >
            {mode === 'smart' ? t('system.smartModelModeSmart') : t('system.smartModelModeCurve')}
          </p>
        </>
      )}
    </div>
  );
}

function SystemTab({ hw }: Props) {
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
      <PageHeader title={t('system.title')} subtitle={t('system.subtitle')} />

      {/* Hardware Discovery */}
      <HardwareDiscovery
        profile={hw.hardwareProfile}
        loading={hw.loadingDiscovery}
        onRescan={hw.runHardwareDiscovery}
        onInstallDriver={hw.installDriver}
      />

      {/* BIOS & Driver Information */}
      <div className="card" style={{ marginTop: 16 }}>
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            marginBottom: 12,
          }}
        >
          <h3>
            <Icon name="wrench" size={16} /> {t('updates.driverDetails')}
          </h3>
          <div style={{ display: 'flex', gap: 8 }}>
            <button className="btn btn-secondary" onClick={handleScan} disabled={scanning}>
              {scanning ? (
                <>
                  <Icon name="search" size={16} /> {t('updates.scanning')}
                </>
              ) : (
                <>
                  <Icon name="search" size={16} /> {t('updates.scanDrivers')}
                </>
              )}
            </button>
            <button className="btn btn-secondary" onClick={fetchDrivers} disabled={loadingDrivers}>
              {loadingDrivers ? (
                t('common.loading')
              ) : (
                <>
                  <Icon name="refresh" size={16} /> {t('updates.refreshDrivers')}
                </>
              )}
            </button>
          </div>
        </div>

        {errorMsg && (
          <div className="alert alert-error" style={{ marginBottom: 12 }}>
            <Icon name="alert-triangle" size={16} /> {errorMsg}
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

      {/* Smart brightness model — what is actually in effect */}
      <SmartModelStatusCard hw={hw as unknown as HardwareInstance} />

      {/* IoT Module Panel */}
      <IotModulePanel hw={hw} />
    </>
  );
}

export default memo(SystemTab);
