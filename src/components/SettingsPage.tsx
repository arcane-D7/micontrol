import { useState, useEffect, useCallback } from 'react';
import { t, useLanguage } from '../hooks/useI18n';
import type { AppSettings } from '../types/settings';
import { invoke } from '@tauri-apps/api/core';
import AiConfigForm from './AiConfigForm';
import PrivacyConsentSection from './PrivacyConsentSection';
import { Button, Switch } from './ui';

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
  onOpenStartup: () => void;
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
  onOpenStartup,
}: Props) {
  const { locale, setLanguage, supported } = useLanguage();
  const [isDeleting, setIsDeleting] = useState(false);
  const [deleteResult, setDeleteResult] = useState<DeleteDataReport | null>(null);
  const [errorLogEnabled, setErrorLogEnabled] = useState(true);
  const [errorLogPath, setErrorLogPath] = useState('');
  const [showErrorLog, setShowErrorLog] = useState(false);
  const [errorLogContent, setErrorLogContent] = useState('');
  const [mcpEnabled, setMcpEnabled] = useState(false);
  const [mcpNotice, setMcpNotice] = useState<string | null>(null);
  const [autostart, setAutostart] = useState(false);
  const [autostartPending, setAutostartPending] = useState(false);
  const [autostartError, setAutostartError] = useState<string | null>(null);

  const handleToggleAutostart = useCallback(async (enabled: boolean) => {
    setAutostartPending(true);
    setAutostartError(null);
    try {
      await invoke('set_autostart', { enabled });
      setAutostart(enabled);
    } catch (e) {
      setAutostartError(e instanceof Error ? e.message : String(e));
    } finally {
      setAutostartPending(false);
    }
  }, []);

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
    // Fetch persisted MCP integration flag
    void invoke<boolean>('mcp_get_enabled')
      .then(setMcpEnabled)
      .catch(() => {});
    // MiControl's own autostart entry, the only startup switch that belongs
    // to Settings; everything else is on the Startup tab.
    void invoke<boolean>('get_autostart')
      .then(setAutostart)
      .catch(() => {});
  }, []);

  const handleToggleMcp = useCallback(async (enabled: boolean) => {
    setMcpEnabled(enabled);
    try {
      await invoke('mcp_set_enabled', { enabled });
      setMcpNotice(
        enabled
          ? t('settings.mcpIntegrationRestartHint') || 'Restart the app to apply.'
          : t('settings.mcpIntegrationOffHint') || 'The MCP socket will close on restart.',
      );
    } catch (e) {
      console.error('Failed to toggle MCP integration:', e);
      setMcpEnabled(!enabled);
      setMcpNotice(null);
    }
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
        <h2 className="card-title">{t('settings.language')}</h2>
        <p className="text-sm" style={{ color: 'var(--color-text-muted)', marginBottom: 12 }}>
          {t('settings.languageDesc')}
        </p>
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
          {supported.map((loc) => (
            <Button
              key={loc.code}
              size="sm"
              variant={locale === loc.code ? 'secondary' : 'ghost'}
              aria-pressed={locale === loc.code}
              onClick={() => setLanguage(loc.code)}
            >
              {loc.nativeLabel}
            </Button>
          ))}
        </div>
      </div>

      {/* General, MiControl's own behaviour */}
      <div className="card" style={{ marginBottom: 16 }}>
        <h2 className="card-title">{t('settings.general')}</h2>
        <Switch
          label={t('settings.miControlAutostart')}
          desc={t('settings.miControlAutostartDesc')}
          checked={autostart}
          pending={autostartPending}
          onChange={(next) => void handleToggleAutostart(next)}
        />
        {autostartError && (
          <p style={{ margin: '8px 0 0', fontSize: 12, color: 'var(--color-error, #e53935)' }}>
            {t('settings.startupError')} {autostartError}
          </p>
        )}
        <div style={{ marginTop: 12 }}>
          <Button variant="secondary" size="sm" icon="rocket" onClick={onOpenStartup}>
            {t('settings.openStartupManager')}
          </Button>
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
        <h2 className="card-title">{t('settings.errorLogging')}</h2>
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
          <Button
            variant="secondary"
            size="sm"
            icon="clipboard"
            onClick={() => void handleViewErrorLog()}
          >
            {t('settings.errorLogView')}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            icon="trash"
            onClick={() => void handleClearErrorLog()}
          >
            {t('settings.errorLogClear')}
          </Button>
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
            <Button variant="secondary" size="sm" onClick={() => setShowErrorLog(false)}>
              {t('common.close')}
            </Button>
          </div>
        )}
      </div>

      {/* MCP Integration (DOM control for AI agents / debug tools) */}
      <div className="card" style={{ marginBottom: 16 }}>
        <h2 className="card-title">{t('settings.mcpIntegration')}</h2>
        <p className="text-sm" style={{ color: 'var(--color-text-muted)', marginBottom: 12 }}>
          {t('settings.mcpIntegrationDesc')}
        </p>
        <div className="stat-row">
          <span className="stat-label">{t('settings.mcpIntegrationEnable')}</span>
          <label className="toggle-switch">
            <input
              type="checkbox"
              checked={mcpEnabled}
              onChange={(e) => void handleToggleMcp(e.target.checked)}
            />
            <span className="toggle-track" />
            <span className="toggle-knob" />
          </label>
        </div>
        <p
          className="text-sm"
          style={{
            color: 'var(--color-warning, #d97706)',
            marginTop: 8,
            fontSize: 11,
          }}
        >
          {t('settings.mcpIntegrationNote')}
        </p>
        {mcpNotice && (
          <div
            style={{
              fontSize: 11,
              color: 'var(--text-dim)',
              marginTop: 8,
            }}
          >
            {mcpNotice}
          </div>
        )}
      </div>

      {/* Replay onboarding */}
      <div className="card" style={{ marginBottom: 16 }}>
        <h2 className="card-title">{t('settings.onboarding')}</h2>
        <p className="text-sm" style={{ color: 'var(--color-text-muted)', marginBottom: 12 }}>
          {t('settings.onboardingDesc')}
        </p>
        <Button variant="secondary" size="sm" onClick={onReplayOnboarding}>
          {t('settings.replayOnboarding')}
        </Button>
      </div>
    </>
  );
}
