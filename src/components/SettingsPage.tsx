import { useState } from 'react';
import { t, useLanguage } from '../hooks/useI18n';
import type { AppSettings } from '../hooks/useSettings';
import { DEFAULT_SETTINGS } from '../hooks/useSettings';

const PRESET_MODELS = [
  { value: 'gpt-4o-mini', label: 'GPT-4o Mini (fast, cheap)' },
  { value: 'gpt-4o', label: 'GPT-4o (best quality)' },
  { value: 'gpt-4-turbo', label: 'GPT-4 Turbo' },
  { value: 'custom', label: 'Custom (type below)' },
];

interface Props {
  settings: AppSettings;
  onSave: (s: AppSettings) => void;
  onTest: () => Promise<void>;
}

function FieldRow({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div style={{ marginBottom: 18 }}>
      <label style={{ display: 'block', fontSize: 13, fontWeight: 600, marginBottom: 4 }}>
        {label}
      </label>
      {children}
      {hint && (
        <div style={{ fontSize: 11, color: 'var(--color-text-muted)', marginTop: 4 }}>{hint}</div>
      )}
    </div>
  );
}

export default function SettingsPage({ settings, onSave, onTest }: Props) {
  const { locale, setLanguage, supported } = useLanguage();
  const [draft, setDraft] = useState<AppSettings>(settings);
  const [showKey, setShowKey] = useState(false);
  const [customModel, setCustomModel] = useState(
    PRESET_MODELS.some((m) => m.value === settings.openai_model && m.value !== 'custom')
      ? ''
      : settings.openai_model,
  );
  const [selectedPreset, setSelectedPreset] = useState(
    PRESET_MODELS.some((m) => m.value === settings.openai_model) ? settings.openai_model : 'custom',
  );
  const [testStatus, setTestStatus] = useState<'idle' | 'testing' | 'ok' | 'error'>('idle');
  const [testMsg, setTestMsg] = useState('');
  const [dirty, setDirty] = useState(false);

  function update<K extends keyof AppSettings>(key: K, value: AppSettings[K]) {
    setDraft((d) => ({ ...d, [key]: value }));
    setDirty(true);
  }

  function handlePresetChange(preset: string) {
    setSelectedPreset(preset);
    if (preset !== 'custom') {
      update('openai_model', preset);
    }
  }

  function handleCustomModelChange(val: string) {
    setCustomModel(val);
    update('openai_model', val);
  }

  function handleSave() {
    onSave(draft);
    setDirty(false);
  }

  async function handleTest() {
    handleSave();
    setTestStatus('testing');
    setTestMsg('');
    try {
      await onTest();
      setTestStatus('ok');
      setTestMsg(t('settings.testOk'));
    } catch (e) {
      setTestStatus('error');
      setTestMsg(String(e).replace(/^Error:\s*/, ''));
    }
  }

  function handleReset() {
    setDraft(DEFAULT_SETTINGS);
    setSelectedPreset(DEFAULT_SETTINGS.openai_model);
    setCustomModel('');
    setDirty(true);
  }

  return (
    <>
      {/* Language selector */}
      <div className="card" style={{ marginBottom: 16 }}>
        <div className="card-title">{t('settings.language')}</div>
        <p style={{ fontSize: 13, color: 'var(--color-text-muted)', marginBottom: 12 }}>
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

      {/* AI settings */}
      <div className="card">
        <div className="card-title">{t('settings.aiTitle')}</div>
        <p style={{ fontSize: 13, color: 'var(--color-text-muted)', marginBottom: 20 }}>
          {t('settings.aiSubtitle')}
        </p>

        {/* API Key */}
        <FieldRow label={t('settings.apiKey')} hint={t('settings.apiKeyHint')}>
          <div style={{ display: 'flex', gap: 8 }}>
            <input
              type={showKey ? 'text' : 'password'}
              value={draft.openai_api_key}
              onChange={(e) => update('openai_api_key', e.target.value)}
              placeholder="sk-..."
              spellCheck={false}
              autoComplete="off"
              style={{ flex: 1, fontFamily: 'var(--font-mono)' }}
            />
            <button
              onClick={() => setShowKey((v) => !v)}
              className="btn-ghost btn-sm"
              title={showKey ? t('settings.hideKey') : t('settings.showKey')}
            >
              {showKey ? '🙈' : '👁'}
            </button>
          </div>
        </FieldRow>

        {/* Base URL */}
        <FieldRow label={t('settings.baseUrl')} hint={t('settings.baseUrlHint')}>
          <input
            type="text"
            value={draft.openai_base_url}
            onChange={(e) => update('openai_base_url', e.target.value)}
            placeholder="https://api.openai.com/v1"
          />
        </FieldRow>

        {/* Model */}
        <FieldRow label={t('settings.model')} hint={t('settings.modelHint')}>
          <div
            style={{
              display: 'flex',
              flexWrap: 'wrap',
              gap: 8,
              marginBottom: selectedPreset === 'custom' ? 8 : 0,
            }}
          >
            {PRESET_MODELS.map((m) => (
              <button
                key={m.value}
                onClick={() => handlePresetChange(m.value)}
                className={`chip-btn ${selectedPreset === m.value ? 'active' : ''}`}
              >
                {m.label}
              </button>
            ))}
          </div>
          {selectedPreset === 'custom' && (
            <input
              type="text"
              value={customModel}
              onChange={(e) => handleCustomModelChange(e.target.value)}
              placeholder="e.g. llama3, mistral, gpt-4-vision-preview"
            />
          )}
        </FieldRow>

        {/* Actions */}
        <div style={{ display: 'flex', gap: 10, alignItems: 'center', flexWrap: 'wrap' }}>
          <button className="btn-primary" disabled={!dirty} onClick={handleSave}>
            {t('settings.save')}
          </button>
          <button
            className="btn-ghost"
            disabled={!draft.openai_api_key.trim() || testStatus === 'testing'}
            onClick={() => void handleTest()}
          >
            {testStatus === 'testing' ? t('settings.testing') : t('settings.testConnection')}
          </button>
          <button className="btn-ghost btn-sm" onClick={handleReset}>
            {t('settings.reset')}
          </button>
        </div>

        {/* Test result */}
        {testMsg && (
          <div
            style={{
              marginTop: 12,
              padding: '8px 12px',
              borderRadius: 6,
              fontSize: 12,
              background: testStatus === 'ok' ? 'rgba(74,222,128,0.1)' : 'rgba(248,113,113,0.1)',
              color:
                testStatus === 'ok'
                  ? 'var(--color-success, #4ade80)'
                  : 'var(--color-danger, #f87171)',
              border: `1px solid ${testStatus === 'ok' ? 'rgba(74,222,128,0.3)' : 'rgba(248,113,113,0.3)'}`,
            }}
          >
            {testStatus === 'ok' ? '✓ ' : '✗ '}
            {testMsg}
          </div>
        )}

        {/* Storage notice */}
        <p
          style={{ fontSize: 11, color: 'var(--color-text-muted)', marginTop: 16, marginBottom: 0 }}
        >
          🔒 {t('settings.storageNote')}
        </p>
      </div>
    </>
  );
}
