import { useState, useEffect, useCallback } from 'react';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import SettingsPage from '../../components/SettingsPage';
import type { AiSettings } from './shared';

interface Props {
  ai: AiSettings;
  onTabChange: (tab: string) => void;
}

export default function SettingsTab({ ai, onTabChange }: Props) {
  const [telemetryConsent, setTelemetryConsent] = useState<'granted' | 'denied' | null>(null);

  useEffect(() => {
    void ai.getTelemetryConsent().then(setTelemetryConsent);
  }, [ai]);

  const handleRevokeConsent = useCallback(async () => {
    await ai.revokeTelemetryConsent();
    setTelemetryConsent(null);
  }, [ai]);

  const handleGrantConsent = useCallback(async () => {
    await ai.setTelemetryConsent('granted');
    setTelemetryConsent('granted');
  }, [ai]);

  const handleReplayOnboarding = useCallback(() => {
    ai.setOnboardingCompleted(false);
  }, [ai]);

  return (
    <>
      <PageHeader title={t('settings.title')} subtitle={t('settings.subtitle')} />
      <SettingsPage
        settings={ai.settings}
        onSave={ai.saveSettings}
        onTest={ai.testConnection}
        telemetryConsent={telemetryConsent}
        onRevokeConsent={handleRevokeConsent}
        onGrantConsent={handleGrantConsent}
        onOpenPrivacyPolicy={() => onTabChange('privacy')}
        onReplayOnboarding={handleReplayOnboarding}
      />
    </>
  );
}
