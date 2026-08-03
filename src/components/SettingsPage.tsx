import { useState, useEffect, useCallback } from 'react';
import { t, useLanguage } from '../hooks/useI18n';
import type { AppSettings } from '../types/settings';
import { invoke } from '@tauri-apps/api/core';
import AiConfigForm from './AiConfigForm';
import PrivacyConsentSection from './PrivacyConsentSection';

interface DeleteDataReport {
  logs_deleted: boolean;
  credentials_deleted: boolean;
  schedule_deleted: boolean;
  consent_deleted: boolean;
  errors: string[];
}

interface Props {
  settings: AppSettings;
  onSave: (s: AppSettings) => void;
  onTest: () => Promise<void>;
  telemetryConsent: 'granted' | 'denied' | null;
  onRevokeConsent: () => Promise<void>;
  onGrantConsent: () => Promise<void>;
  onOpenPrivacyPolicy: () => void;
  onReplayOnboarding: () => void;
}

export default function SettingsPage({
  settings,
  onSave,
  onTest,
  telemetryConsent,
  onRevokeConsent,
  onGrantConsent,
  onOpenPrivacyPolicy,
  onReplayOnboarding,
}: Props) {
  const { locale, setLanguage, supported } = useLanguage();
  const [isDeleting, setIsDeleting] = useState(false);
  const [deleteResult, setDeleteResult] = useState<DeleteDataReport | null>(null);
  const [errorLogEnabled, setErrorLogEnabled] = useState(true);
  const [errorLogPath, setErrorLogPath] = useState('');
  const [showErrorLog, setShowErrorLog] = useState(false);
  const [errorLogContent, setErrorLogContent] = useState('');

  // Fetch error log config on mount
  useEffect(() => {
    void invoke<{ enabled: boolean; retention_days: number; log_path: string }>(
      'get_error_log_config',
    )
      .then((cfg) => {
        setErrorLogEnabled(cfg.enabled);
        setErrorLogPath(cfg.log_path);
      })
      .catch(() => {});
  }, []);

  const handleToggleErrorLog = useCallback(async (enabled: boolean) => {
    setErrorLogEnabled(enabled);
    try {
      await invoke('set_error_logging_enabled', { enabled });
    } catch (e) {
      console.error('Failed to toggle error logging:', e);
      setErrorLogEnabled(!enabled);
    }
  }, []);

  const handleViewErrorLog = useCallback(async () => {
    try {
      const content = await invoke<string>('read_error_log', { maxLines: 500 });
      setErrorLogContent(content || 'No errors logged.');
      setShowErrorLog(true);
    } catch (e) {
      setErrorLogContent(`Failed to read error log: ${String(e)}`);
      setShowErrorLog(true);
    }
  }, []);

  const handleClearErrorLog = useCallback(async () => {
    try {
      await invoke('clear_error_log');
      setErrorLogContent('');
    } catch (e) {
      console.error('Failed to clear error log:', e);
    }
  }, []);

  const handleDeleteAllData = async () => {
    if (!confirm(t('settings.confirmDelete'))) return;
    setIsDeleting(true);
    try {
      const result = await invoke<DeleteDataReport>('delete_all_user_data');
      setDeleteResult(result);
      localStorage.clear();
    } catch (e) {
      console.error('Failed to delete data:', e);
      setDeleteResult({
        logs_deleted: false,
        credentials_deleted: false,
        schedule_deleted: false,
        consent_deleted: false,
        errors: [String(e)],
      });
    } finally {
      setIsDeleting(false);
    }
  };

  return (
    <>
      {/* Language selector */}
      <div className="card" style={{ marginBottom: 16 }}>
        <div className="card-title">{t('settings.language')}</div>
        <p className="text-sm" style={{ color: 'var(--color-text-muted)', marginBottom: 12 }}>
          {t('settings.languageDesc')}
        </p>
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
          {supported.map((loc) => (
            <button
              key={loc.code}
              className={`chip-btn ${locale === loc.code ? 'active' : ''}`}
              onClick={() => setLanguage(loc.code)}
            >
              {loc.nativeLabel}
            </button>
          ))}
        </div>
      </div>

      <AiConfigForm
        settings={settings}
        onUpdate={(patch) => onSave({ ...settings, ...patch })}
        onTestConnection={onTest}
      />

      <PrivacyConsentSection
        consent={telemetryConsent}
        onGrant={onGrantConsent}
        onRevoke={onRevokeConsent}
        onOpenPrivacyPolicy={onOpenPrivacyPolicy}
        onDeleteAllData={handleDeleteAllData}
        deleteResult={deleteResult}
        isDeleting={isDeleting}
      />

      {/* Error Logging */}
      <div className="card" style={{ marginBottom: 16 }}>
        <div className="card-title">{t('settings.errorLogging')}</div>
        <p className="text-sm" style={{ color: 'var(--color-text-muted)', marginBottom: 12 }}>
          {t('settings.errorLoggingDesc')}
        </p>
        <div className="stat-row">
          <span className="stat-label">{t('settings.errorLoggingEnable')}</span>
          <label className="toggle-switch">
            <input
              type="checkbox"
              checked={errorLogEnabled}
              onChange={(e) => void handleToggleErrorLog(e.target.checked)}
            />
            <span className="toggle-track" />
            <span className="toggle-knob" />
          </label>
        </div>
        {errorLogPath && (
          <div
            style={{
              fontSize: 11,
              color: 'var(--text-dim)',
              marginTop: 8,
              fontFamily: 'var(--font-mono)',
            }}
          >
            {errorLogPath}
          </div>
        )}
        <div style={{ display: 'flex', gap: 8, marginTop: 12 }}>
          <button className="btn btn-secondary" onClick={() => void handleViewErrorLog()}>
            📋 {t('settings.errorLogView')}
          </button>
          <button className="btn btn-secondary" onClick={() => void handleClearErrorLog()}>
            🗑 {t('settings.errorLogClear')}
          </button>
        </div>
        {showErrorLog && (
          <div style={{ marginTop: 12 }}>
            <pre
              style={{
                fontSize: 11,
                whiteSpace: 'pre-wrap',
                wordBreak: 'break-all',
                color: 'var(--text-muted)',
                maxHeight: 300,
                overflow: 'auto',
                background: 'var(--surface-2)',
                borderRadius: 'var(--r-sm)',
                padding: 12,
                border: '1px solid var(--border)',
                fontFamily: 'var(--font-mono)',
              }}
            >
              {errorLogContent}
            </pre>
            <button
              className="btn btn-secondary"
              style={{ marginTop: 8 }}
              onClick={() => setShowErrorLog(false)}
            >
              {t('common.close')}
            </button>
          </div>
        )}
      </div>

      {/* Replay onboarding */}
      <div className="card" style={{ marginBottom: 16 }}>
        <div className="card-title">{t('settings.onboarding')}</div>
        <p className="text-sm" style={{ color: 'var(--color-text-muted)', marginBottom: 12 }}>
          {t('settings.onboardingDesc')}
        </p>
        <button className="btn btn-secondary" onClick={onReplayOnboarding}>
          {t('settings.replayOnboarding')}
        </button>
      </div>
    </>
  );
}
