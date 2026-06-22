import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { t } from '../hooks/useI18n';
import ToggleRow from './ToggleRow';

interface Props {
  autostart: boolean;
}

export default function StartupManager({ autostart }: Props) {
  const [enabled, setEnabled] = useState(autostart);
  const [saving, setSaving] = useState(false);

  const handleToggle = async (value: boolean) => {
    setSaving(true);
    try {
      await invoke('set_autostart', { enabled: value });
      setEnabled(value);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="card">
      <div className="card-title">{t('startup.title')}</div>

      <ToggleRow
        label={t('startup.runAtStartup')}
        desc={t('startup.description')}
        checked={enabled}
        disabled={saving}
        onChange={(v) => void handleToggle(v)}
      />
    </div>
  );
}
