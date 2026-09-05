import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { t } from '../hooks/useI18n';

interface PrivacyConsentSectionProps {
  consent: 'granted' | 'denied' | null;
  onGrant: () => Promise<void>;
  onRevoke: () => Promise<void>;
  onOpenPrivacyPolicy: () => void;
  onDeleteAllData?: () => Promise<void>;
  deleteResult?: { errors: string[] } | null;
  isDeleting?: boolean;
}

export default function PrivacyConsentSection({
  consent,
  onGrant,
  onRevoke,
  onOpenPrivacyPolicy,
  onDeleteAllData,
  deleteResult,
  isDeleting,
}: PrivacyConsentSectionProps) {
  const [isExporting, setIsExporting] = useState(false);
  const [exportError, setExportError] = useState<string | null>(null);

  // ── S49: Crash reports (Sentry) dedicated card ──────────────────────────
  // The consent state is shared with the AI features (same store) but this
  // card speaks plainly about crash reports only, with a real toggle. The
  // Sentry init in App.tsx listens for `micontrol:consent-changed` and
  // enables/disables capture live — no restart needed.
  const [crashReports, setCrashReports] = useState<'granted' | 'denied' | null>(consent);
  const [crashBusy, setCrashBusy] = useState(false);
  useEffect(() => {
    setCrashReports(consent);
  }, [consent]);

  const handleCrashToggle = async (next: boolean) => {
    setCrashBusy(true);
    try {
      if (next) {
        await onGrant();
        setCrashReports('granted');
      } else {
        await onRevoke();
        setCrashReports('denied');
      }
    } finally {
      setCrashBusy(false);
    }
  };

  const handleExportData = async () => {
    setIsExporting(true);
    setExportError(null);
    try {
      const zipPath = await invoke<string>('export_user_data');
      await invoke('reveal_in_explorer', { path: zipPath });
    } catch (e) {
      setExportError(String(e));
    } finally {
      setIsExporting(false);
    }
  };
  return (
    <>
      {/* ── S49: Crash Reports (Sentry) — dedicated, plain-language card ── */}
      <div className="card">
        <div className="card-title">{t('consent.dialog.settingsTitle')}</div>
        <p className="text-sm" style={{ color: 'var(--color-text-muted)', marginBottom: 14 }}>
          {t('consent.dialog.settingsDesc')}
        </p>
        <label className="toggle" style={{ padding: '4px 0 14px' }}>
          <span className="toggle-info">
            <span className="toggle-name">{t('consent.dialog.settingsEnable')}</span>
            <span className="toggle-desc">
              {crashReports === 'granted'
                ? t('consent.dialog.settingsStatusOn')
                : crashReports === 'denied'
                  ? t('consent.dialog.settingsStatusOff')
                  : t('consent.dialog.settingsStatusUndecided')}
            </span>
          </span>
          <span className="toggle-switch">
            <input
              type="checkbox"
              checked={crashReports === 'granted'}
              disabled={crashBusy}
              onChange={(e) => void handleCrashToggle(e.target.checked)}
            />
            <span className="toggle-track" />
            <span className="toggle-knob" />
          </span>
        </label>
        <div className="badge" style={{ alignSelf: 'flex-start' }}>
          <span className={`badge ${crashReports === 'granted' ? 'success' : 'info'}`}>
            {crashReports === 'granted'
              ? `✓ ${t('consent.dialog.settingsEnabledAt')}`
              : crashReports === 'denied'
                ? `⊘ ${t('consent.dialog.settingsDisabledAt')}`
                : `? ${t('privacy.consentNotSet')}`}
          </span>
        </div>
      </div>

      {/* Privacy & Consent */}
      <div className="card" style={{ marginTop: 16 }}>
        <div className="card-title">{t('settings.privacy')}</div>
        <p className="text-sm" style={{ color: 'var(--color-text-muted)', marginBottom: 16 }}>
          {t('settings.consentStatus')}:{' '}
          <strong
            style={{
              color:
                consent === 'granted'
                  ? 'var(--color-success, #4ade80)'
                  : consent === 'denied'
                    ? 'var(--color-danger, #f87171)'
                    : 'var(--color-text-muted)',
            }}
          >
            {consent === 'granted'
              ? t('privacy.consentGranted')
              : consent === 'denied'
                ? t('privacy.consentDenied')
                : t('privacy.consentNotSet')}
          </strong>
        </p>
        <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap' }}>
          <button className="btn-ghost btn-sm" onClick={() => void onOpenPrivacyPolicy()}>
            📄 {t('settings.privacyPolicy')}
          </button>
          {consent === 'granted' && (
            <button
              className="btn-ghost btn-sm"
              onClick={() => void onRevoke()}
              style={{
                color: 'var(--color-danger, #f87171)',
                borderColor: 'var(--color-danger, #f87171)',
              }}
            >
              🛑 {t('settings.revokeConsent')}
            </button>
          )}
          {consent === 'denied' && (
            <button className="btn-primary btn-sm" onClick={() => void onGrant()}>
              ✅ {t('settings.grantConsent')}
            </button>
          )}
        </div>
        <p
          className="text-xs"
          style={{ color: 'var(--color-text-muted)', marginTop: 12, marginBottom: 0 }}
        >
          🔒 {t('settings.consentStatus')}{' '}
          {consent === 'granted'
            ? t('settings.consentGranted')
            : consent === 'denied'
              ? t('settings.consentDenied')
              : t('settings.consentNotSet')}
        </p>
      </div>

      {/* Data Deletion — GDPR Art.17 */}
      {onDeleteAllData && (
        <div className="card" style={{ marginTop: 16 }}>
          <div className="card-title">{t('settings.dataDeletion')}</div>
          <p className="text-sm" style={{ color: 'var(--color-text-muted)', marginBottom: 16 }}>
            {t('settings.dataDeletionDesc')}
          </p>
          <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap', alignItems: 'center' }}>
            <button
              className="btn-ghost btn-sm"
              onClick={() => void onDeleteAllData()}
              disabled={isDeleting}
              style={{
                color: 'var(--color-danger, #f87171)',
                borderColor: 'var(--color-danger, #f87171)',
              }}
            >
              {isDeleting ? t('settings.deleting') : `🗑 ${t('settings.deleteAllData')}`}
            </button>
          </div>
          {deleteResult && (
            <div
              role="alert"
              className="text-sm"
              style={{
                marginTop: 12,
                padding: '8px 12px',
                borderRadius: 6,
                background:
                  deleteResult.errors.length > 0 ? 'rgba(248,113,113,0.1)' : 'rgba(74,222,128,0.1)',
                color:
                  deleteResult.errors.length > 0
                    ? 'var(--color-danger, #f87171)'
                    : 'var(--color-success, #4ade80)',
                border: `1px solid ${deleteResult.errors.length > 0 ? 'rgba(248,113,113,0.3)' : 'rgba(74,222,128,0.3)'}`,
              }}
            >
              {deleteResult.errors.length > 0
                ? `✗ ${t('settings.deleteError')}`
                : `✓ ${t('settings.deleteSuccess')}`}
            </div>
          )}
        </div>
      )}

      {/* Data Export — GDPR Art.20 (S19-16) */}
      <div className="card" style={{ marginTop: 16 }}>
        <div className="card-title">{t('settings.dataExport')}</div>
        <p className="text-sm" style={{ color: 'var(--color-text-muted)', marginBottom: 16 }}>
          {t('settings.dataExportDesc')}
        </p>
        <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap', alignItems: 'center' }}>
          <button
            className="btn-ghost btn-sm"
            onClick={() => void handleExportData()}
            disabled={isExporting}
          >
            {isExporting ? t('settings.exporting') : `📦 ${t('settings.downloadData')}`}
          </button>
        </div>
        {exportError && (
          <div
            role="alert"
            className="text-sm"
            style={{
              marginTop: 12,
              padding: '8px 12px',
              borderRadius: 6,
              background: 'rgba(248,113,113,0.1)',
              color: 'var(--color-danger, #f87171)',
              border: '1px solid rgba(248,113,113,0.3)',
            }}
          >
            ✗ {exportError}
          </div>
        )}
      </div>
    </>
  );
}
