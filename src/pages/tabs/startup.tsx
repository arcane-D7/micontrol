import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import StartupManager from '../../components/StartupManager';

export default function StartupTab() {
  const [autostart, setAutostart] = useState(false);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    invoke<boolean>('get_autostart')
      .then(setAutostart)
      .catch(() => setAutostart(false))
      .finally(() => setLoading(false));
  }, []);

  if (loading) return null;

  return (
    <>
      <PageHeader title={t('startup.title')} />
      <StartupManager autostart={autostart} />
    </>
  );
}
