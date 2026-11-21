import { useState } from 'react';
import { t } from '../hooks/useI18n';
import type { HardwareProfile, MissingDriver } from '../types/hardware';
import { useToast } from '../contexts/ToastContext';
import { Icon } from './ui/icons';

interface Props {
  profile: HardwareProfile | null;
  loading: boolean;
  onRescan: () => Promise<unknown>;
  onInstallDriver: (name: string) => Promise<string>;
}

interface PathRowProps {
  label: string;
  value: string | null | undefined;
}

function PathRow({ label, value }: PathRowProps) {
  const found = Boolean(value);
  return (
    <div className="stat-row">
      <span className="stat-label">{label}</span>
      <span
        className="stat-value"
        style={{
          color: found ? 'var(--color-success, #4ade80)' : 'var(--color-warning, #facc15)',
          fontFamily: found ? 'monospace' : undefined,
          fontSize: found ? 11 : undefined,
          wordBreak: 'break-all',
        }}
        title={value ?? undefined}
      >
        {found ? (value!.length > 60 ? `…${value!.slice(-58)}` : value!) : t('discovery.notFound')}
      </span>
    </div>
  );
}

function BoolRow({ label, value }: { label: string; value: boolean }) {
  return (
    <div className="stat-row">
      <span className="stat-label">{label}</span>
      <span
        style={{ color: value ? 'var(--color-success, #4ade80)' : 'var(--color-warning, #facc15)' }}
      >
        {value ? (
          <>
            <Icon name="check" size={16} /> {t('common.yes')}
          </>
        ) : (
          <>
            <Icon name="x" size={16} /> {t('common.no')}
          </>
        )}
      </span>
    </div>
  );
}

function DriverInstallCard({
  driver,
  onInstall,
}: {
  driver: MissingDriver;
  onInstall: (name: string) => Promise<string>;
}) {
  const [status, setStatus] = useState<'idle' | 'installing' | 'success' | 'error'>('idle');
  const [message, setMessage] = useState('');

  async function handleInstall() {
    setStatus('installing');
    setMessage('');
    try {
      const result = await onInstall(driver.name);
      setStatus('success');
      setMessage(result);
    } catch (e) {
      setStatus('error');
      setMessage(String(e));
    }
  }

  return (
    <div className="card" style={{ marginBottom: 8 }}>
      <h2 className="card-title" style={{ color: 'var(--warning)' }}>
        <Icon name="alert-triangle" size={16} /> {driver.name}
      </h2>
      <p style={{ fontSize: 13, marginBottom: 8 }}>{driver.description}</p>
      {driver.bundled_inf ? (
        <button
          className="btn-primary"
          disabled={status === 'installing' || status === 'success'}
          onClick={() => void handleInstall()}
        >
          {status === 'installing'
            ? t('discovery.installing')
            : status === 'success'
              ? t('discovery.installSuccess')
              : t('discovery.installBtn')}
        </button>
      ) : (
        <span style={{ fontSize: 12, color: 'var(--color-text-muted)' }}>
          {t('discovery.noBundledInf')}
        </span>
      )}
      {message && (
        <pre
          style={{
            fontSize: 11,
            color:
              status === 'error' ? 'var(--color-danger, #f87171)' : 'var(--color-success, #4ade80)',
            whiteSpace: 'pre-wrap',
            marginTop: 4,
            marginBottom: 0,
          }}
        >
          {message}
        </pre>
      )}
      {status === 'error' && message.includes('Administrator') && (
        <p style={{ fontSize: 12, color: 'var(--color-text-muted)', marginTop: 4 }}>
          {t('discovery.adminHint')}
        </p>
      )}
    </div>
  );
}

export default function HardwareDiscovery({ profile, loading, onRescan, onInstallDriver }: Props) {
  const [scanning, setScanning] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);
  const { addToast } = useToast();

  async function handleRescan() {
    setScanning(true);
    setScanError(null);
    try {
      await onRescan();
      addToast({ message: t('discovery.scanSuccess'), type: 'success' });
    } catch (e) {
      setScanError(String(e));
      addToast({
        message: `${t('discovery.scanFailed')}: ${String(e)}`,
        type: 'error',
        onRetry: handleRescan,
      });
    } finally {
      setScanning(false);
    }
  }

  const discoveredAt = profile ? new Date(profile.discovered_at * 1000).toLocaleString() : null;

  return (
    <>
      {/* Actions */}
      <div className="card" style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
        <button
          className="btn-primary"
          disabled={scanning || loading}
          onClick={() => void handleRescan()}
        >
          {scanning ? t('discovery.scanning') : t('discovery.rescan')}
        </button>
        {discoveredAt && (
          <span style={{ fontSize: 12, color: 'var(--color-text-muted)' }}>
            {t('discovery.lastScan')}: {discoveredAt}
          </span>
        )}
        {scanError && (
          <span style={{ fontSize: 12, color: 'var(--color-danger, #f87171)' }}>{scanError}</span>
        )}
      </div>

      {!profile && !loading && (
        <div className="card">
          <p style={{ color: 'var(--color-text-muted)' }}>{t('discovery.noProfile')}</p>
        </div>
      )}

      {profile && (
        <>
          {/* Device */}
          <div className="card">
            <h2 className="card-title">{t('discovery.device')}</h2>
            <div className="stat-row">
              <span className="stat-label">{t('discovery.model')}</span>
              <span className="stat-value">{profile.device_model ?? t('common.unknown')}</span>
            </div>
            <BoolRow label={t('discovery.miRegistry')} value={profile.mi_registry_present} />
          </div>

          {/* Hardware paths */}
          <div className="card">
            <h2 className="card-title">{t('discovery.paths')}</h2>
            <PathRow label={t('discovery.vhfPath')} value={profile.vhf_device_path} />
            <PathRow label={t('discovery.touchpadPath')} value={profile.touchpad_hid_path} />
            <PathRow label={t('discovery.touchscreenPath')} value={profile.touchscreen_hid_path} />
            <PathRow label={t('discovery.stylusPath')} value={profile.stylus_hid_path} />
            <PathRow label={t('discovery.iotPipe')} value={profile.iot_pipe_path} />
            <PathRow label={t('discovery.iotService')} value={profile.iot_service_name} />
            <PathRow label={t('discovery.igclDll')} value={profile.igcl_dll_path} />
          </div>

          {/* Capability flags */}
          <div className="card">
            <h2 className="card-title">{t('discovery.capabilities')}</h2>
            <BoolRow
              label={t('discovery.cap.vhfPerformance')}
              value={profile.capabilities.has_vhf_performance}
            />
            {!profile.capabilities.has_vhf_performance && (
              <div
                style={{
                  marginTop: 4,
                  marginBottom: 6,
                  padding: '7px 10px',
                  borderRadius: 'var(--r-xs)',
                  background: 'color-mix(in oklab, var(--accent) 7%, transparent)',
                  border: '1px solid color-mix(in oklab, var(--accent) 18%, transparent)',
                  fontSize: 11.5,
                  color: 'var(--text-muted)',
                  lineHeight: 1.5,
                }}
              >
                <span style={{ color: 'var(--accent)', fontWeight: 600 }} aria-hidden="true">
                  <Icon name="info" size={16} />{' '}
                </span>
                {t('discovery.vhfNotRequired')}
              </div>
            )}
            <BoolRow
              label={t('discovery.cap.touchpadHid')}
              value={profile.capabilities.has_touchpad_hid}
            />
            <BoolRow
              label={t('discovery.cap.touchscreen')}
              value={profile.capabilities.has_touchscreen}
            />
            <BoolRow label={t('discovery.cap.stylus')} value={profile.capabilities.has_stylus} />
            <BoolRow label={t('discovery.cap.igcl')} value={profile.capabilities.has_igcl} />
            <BoolRow
              label={t('discovery.cap.iotCharging')}
              value={profile.capabilities.has_iot_charging}
            />
            <BoolRow
              label={t('discovery.cap.miRegistry')}
              value={profile.capabilities.has_mi_registry}
            />
          </div>

          {/* Missing drivers */}
          {profile.missing_drivers.length > 0 ? (
            <div className="card">
              <h2 className="card-title" style={{ color: 'var(--color-warning, #facc15)' }}>
                {t('discovery.missingDrivers')} ({profile.missing_drivers.length})
              </h2>
              <p style={{ fontSize: 13, marginBottom: 12 }}>{t('discovery.missingNote')}</p>
              {profile.missing_drivers.map((d) => (
                <DriverInstallCard key={d.name} driver={d} onInstall={onInstallDriver} />
              ))}
            </div>
          ) : (
            <div className="card">
              <h2 className="card-title" style={{ color: 'var(--color-success, #4ade80)' }}>
                <Icon name="check" size={16} /> {t('discovery.allDriversInstalled')}
              </h2>
            </div>
          )}
        </>
      )}
    </>
  );
}
