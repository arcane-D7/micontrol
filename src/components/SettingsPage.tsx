import { useState } from 'react';
import { t, useLanguage } from '../hooks/useI18n';
import type { AppSettings } from '../hooks/useSettings';
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
