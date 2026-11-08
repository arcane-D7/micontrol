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

  const [error, setError] = useState<string | null>(null);

  const handleToggle = async (value: boolean) => {
    setSaving(true);
    setError(null);
    try {
      await invoke('set_autostart', { enabled: value });
      setEnabled(value);
    } catch (e) {
      // Revert the toggle state on failure and surface the error.
      setEnabled(!value);
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      console.error('Failed to toggle autostart:', e);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="card">
      <h2 className="card-title">{t('startup.title')}</h2>

      <ToggleRow
        label={t('startup.runAtStartup')}
        desc={t('startup.description')}
        checked={enabled}
        disabled={saving}
        onChange={(v) => void handleToggle(v)}
      />
      {error && (
        <div style={{ marginTop: 8, fontSize: 12, color: 'var(--color-error, #e53935)' }}>
          {error}
        </div>
      )}
    </div>
  );
}
