import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import { Icon } from '../../components/ui/icons';
import { InfoHint } from '../../components/ui/info-hint';

// ── Types matching Rust structs ──────────────────────────────────────────────

interface DefenderStatus {
  installed: boolean;
  enabled: boolean;
  antivirus_enabled: boolean;
  antispyware_enabled: boolean;
  real_time_protection: boolean;
  definitions_updated: string | null;
  engine_version: string | null;
  product_version: string | null;
  signature_version: string | null;
  last_scan_time: string | null;
}

interface ThreatEntry {
  threat_name: string;
  severity_id: number | null;
  severity_name: string | null;
  category_id: number | null;
  category_name: string | null;
  action_success: boolean;
  action_id: number | null;
  action_name: string | null;
  initial_detection_time: string | null;
  remediation_time: string | null;
  resources: string[];
}

interface ThreatHistory {
  threats: ThreatEntry[];
  total_count: number;
}

interface ScanResult {
  scan_type: string;
  exit_code: number;
  status: string;
  output: string;
  duration_secs: number;
}

// ── Component ────────────────────────────────────────────────────────────────

export default function SecurityTab() {
  const [status, setStatus] = useState<DefenderStatus | null>(null);
  const [threats, setThreats] = useState<ThreatEntry[]>([]);
  // Distinct from "no threats": the query failed, so the empty list means
  // "unknown", not "clean" (audit F4.2).
  const [threatsError, setThreatsError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [scanning, setScanning] = useState(false);
  const [scanResult, setScanResult] = useState<ScanResult | null>(null);
  const [customPath, setCustomPath] = useState('');
  const [updating, setUpdating] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const fetchStatus = useCallback(async () => {
    try {
      const s = await invoke<DefenderStatus>('get_defender_status');
      setStatus(s);
    } catch (e) {
      setStatus(null);
      setErrorMsg(String(e));
    }
    try {
      const h = await invoke<ThreatHistory>('get_threat_history');
      setThreats(h.threats || []);
      setThreatsError(null);
    } catch (e) {
      setThreats([]);
      setThreatsError(String(e));
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    void fetchStatus();
  }, [fetchStatus]);

  const handleQuickScan = async () => {
    setScanning(true);
    setScanResult(null);
    try {
      const result = await invoke<ScanResult>('quick_security_scan');
      setScanResult(result);
      void fetchStatus();
    } catch (e) {
      setScanResult({
        scan_type: 'quick',
        exit_code: -1,
        status: 'error',
        output: String(e),
        duration_secs: 0,
      });
    }
    setScanning(false);
  };

  const handleFullScan = async () => {
    setScanning(true);
    setScanResult(null);
    try {
      const result = await invoke<ScanResult>('full_security_scan');
      setScanResult(result);
      void fetchStatus();
    } catch (e) {
      setScanResult({
        scan_type: 'full',
        exit_code: -1,
        status: 'error',
        output: String(e),
        duration_secs: 0,
      });
    }
    setScanning(false);
  };

  const handleCustomScan = async () => {
    if (!customPath.trim()) return;
    setScanning(true);
    setScanResult(null);
    try {
      const result = await invoke<ScanResult>('custom_security_scan', { path: customPath });
      setScanResult(result);
      void fetchStatus();
    } catch (e) {
      setScanResult({
        scan_type: 'custom',
        exit_code: -1,
        status: 'error',
        output: String(e),
        duration_secs: 0,
      });
    }
    setScanning(false);
  };

  const handleUpdateSignatures = async () => {
    setUpdating(true);
    setErrorMsg(null);
    try {
      const result = await invoke<ScanResult>('update_defender_signatures');
      setScanResult(result);
      void fetchStatus();
    } catch (e) {
      setErrorMsg(String(e));
    }
    setUpdating(false);
  };

  const handleOpenDefender = async () => {
    try {
      await invoke('open_windows_security');
    } catch (e) {
      setErrorMsg(String(e));
    }
  };

  if (loading) {
    return (
      <>
        <PageHeader title={t('security.title')} subtitle={t('security.subtitle')} />
        <div className="loading-spinner">{t('common.loading')}</div>
      </>
    );
  }

  return (
    <>
      <PageHeader title={t('security.title')} subtitle={t('security.subtitle')} />

      {/* Defender Status Card */}
      <div className="card" style={{ marginBottom: 16 }}>
        <h3>{t('security.defenderStatus')}</h3>
        {status ? (
          <div className="info-grid">
            <div className="info-row">
              <span className="info-label">{t('security.realTimeProtection')}</span>
              <span
                className={`info-value ${status.real_time_protection ? 'status-ok' : 'status-warn'}`}
              >
                {status.real_time_protection ? t('common.on') : t('common.off')}
              </span>
            </div>
            <div className="info-row">
              <span className="info-label">{t('security.antivirusEnabled')}</span>
              <span
                className={`info-value ${status.antivirus_enabled ? 'status-ok' : 'status-error'}`}
              >
                {status.antivirus_enabled
                  ? t('security.antivirusEnabled')
                  : t('security.antivirusDisabled')}
              </span>
            </div>
            {status.signature_version && (
              <div className="info-row">
                <span className="info-label">{t('security.signatureVersion')}</span>
                <span className="info-value">{status.signature_version}</span>
              </div>
            )}
            {status.engine_version && (
              <div className="info-row">
                <span className="info-label">{t('security.engineVersion')}</span>
                <span className="info-value">{status.engine_version}</span>
              </div>
            )}
            {status.last_scan_time && (
              <div className="info-row">
                <span className="info-label">{t('security.lastScan')}</span>
                <span className="info-value">{status.last_scan_time}</span>
              </div>
            )}
          </div>
        ) : (
          <p className="text-muted">{t('security.defenderNotAvailable')}</p>
        )}
      </div>

      {/* Error message */}
      {errorMsg && (
        <div className="alert alert-error" style={{ marginBottom: 16 }}>
          <Icon name="alert-triangle" size={16} /> {errorMsg}
        </div>
      )}

      {/* Scan Actions Card */}
      <div className="card" style={{ marginBottom: 16 }}>
        <h3 className="mc-label-with-hint">
          <span>{t('security.scanActions')}</span>
          <InfoHint
            label={t('hint.securityScan')}
            buttonLabel={t('hint.about', { name: t('security.scanActions') })}
          />
        </h3>
        <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', marginBottom: 12 }}>
          <button className="btn btn-primary" onClick={handleQuickScan} disabled={scanning}>
            {scanning ? (
              t('security.scanning')
            ) : (
              <>
                <Icon name="search" size={16} /> {t('security.quickScan')}
              </>
            )}
          </button>
          <button className="btn btn-secondary" onClick={handleFullScan} disabled={scanning}>
            {scanning ? (
              t('security.scanning')
            ) : (
              <>
                <Icon name="virus" size={16} /> {t('security.fullScan')}
              </>
            )}
          </button>
          <button
            className="btn btn-secondary"
            onClick={handleUpdateSignatures}
            disabled={updating}
          >
            {updating ? (
              t('security.updating')
            ) : (
              <>
                <Icon name="download" size={16} /> {t('security.updateSignatures')}
              </>
            )}
          </button>
          <button className="btn btn-secondary" onClick={handleOpenDefender}>
            <Icon name="shield" size={16} /> {t('security.openDefender')}
          </button>
        </div>

        {/* Custom scan path input */}
        <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          <input
            type="text"
            className="text-input"
            placeholder={t('security.customScanPlaceholder')}
            value={customPath}
            onChange={(e) => setCustomPath(e.target.value)}
            style={{ flex: 1 }}
          />
          <button
            className="btn btn-secondary"
            onClick={handleCustomScan}
            disabled={scanning || !customPath.trim()}
          >
            {t('security.customScan')}
          </button>
        </div>
        <p className="text-muted" style={{ marginTop: 4, fontSize: 12 }}>
          {t('security.quickScanDesc')}
        </p>

        {/* Scan result */}
        {scanResult && (
          <div
            className={`alert ${scanResult.status === 'clean' ? 'alert-success' : scanResult.status === 'error' ? 'alert-error' : 'alert-warn'}`}
            style={{ marginTop: 12 }}
          >
            {scanResult.status === 'clean' && (
              <>
                <Icon name="check" size={16} /> {t('security.scanClean')}
              </>
            )}
            {scanResult.status === 'threats_detected' &&
              t('security.scanThreats', { count: scanResult.exit_code === 2 ? 1 : 0 })}
            {scanResult.status === 'error' && (
              <>
                <Icon name="alert-triangle" size={16} />{' '}
                {scanResult.output || t('errors.unknownError')}
              </>
            )}
            {scanResult.status === 'updated' && (
              <>
                <Icon name="check" size={16} /> {t('security.updateComplete')}
              </>
            )}
          </div>
        )}
      </div>

      {/* Threat History Card */}
      <div className="card">
        <h3>{t('security.threatHistory')}</h3>
        {threatsError ? (
          // The history could not be read. This must be visibly different from
          // "no threats": the old UI claimed a tick + "Nenhuma ameaça" on every failure.
          <div className="alert alert-error">
            <div>
              <Icon name="alert-triangle" size={16} /> {t('security.threatHistoryError')}
            </div>
            <div style={{ fontSize: 12, marginTop: 4, opacity: 0.85 }}>{threatsError}</div>
          </div>
        ) : threats.length === 0 ? (
          <p className="text-muted">
            <Icon name="check" size={16} /> {t('security.noThreats')}
          </p>
        ) : (
          <table className="data-table" style={{ width: '100%' }}>
            <thead>
              <tr>
                <th>{t('security.threatName')}</th>
                <th>{t('security.threatType')}</th>
                <th>{t('security.threatSeverity')}</th>
                <th>{t('security.threatAction')}</th>
                <th>{t('security.threatTime')}</th>
              </tr>
            </thead>
            <tbody>
              {threats.map((threat, i) => (
                <tr key={i}>
                  <td>{threat.threat_name}</td>
                  <td>{threat.category_name || threat.category_id?.toString() || '—'}</td>
                  <td>
                    <span
                      className={`badge badge-${(threat.severity_name || 'low').toLowerCase()}`}
                    >
                      {threat.severity_name || t('security.severityLow')}
                    </span>
                  </td>
                  <td>
                    {threat.action_name ||
                      (threat.action_success ? <Icon name="check" size={16} /> : '—')}
                  </td>
                  <td>{threat.initial_detection_time || threat.remediation_time || '—'}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </>
  );
}
