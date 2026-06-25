import { useState } from 'react';
import { t } from '../hooks/useI18n';
import type { AppSettings } from '../hooks/useSettings';
import { DEFAULT_SETTINGS } from '../hooks/useSettings';

interface AiConfigFormProps {
  settings: AppSettings;
  onUpdate: (patch: Partial<AppSettings>) => void;
  onTestConnection: () => Promise<void>;
}

const PRESET_MODELS = [
  { value: 'gpt-4o-mini', label: 'GPT-4o Mini (fast, cheap)' },
  { value: 'gpt-4o', label: 'GPT-4o (best quality)' },
  { value: 'gpt-4-turbo', label: 'GPT-4 Turbo' },
  { value: 'custom', label: 'Custom (type below)' },
];

function FieldRow({
  label,
  hint,
  htmlFor,
  children,
}: {
  label: string;
  hint?: string;
  htmlFor?: string;
  children: React.ReactNode;
}) {
  return (
    <div style={{ marginBottom: 18 }}>
      <label
        htmlFor={htmlFor}
        className="text-sm"
        style={{ display: 'block', fontWeight: 600, marginBottom: 4 }}
      >
        {label}
      </label>
      {children}
      {hint && (
        <div className="text-xs" style={{ color: 'var(--color-text-muted)', marginTop: 4 }}>
          {hint}
        </div>
      )}
    </div>
  );
}

export default function AiConfigForm({ settings, onUpdate, onTestConnection }: AiConfigFormProps) {
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
    onUpdate(draft);
    setDirty(false);
  }

  async function handleTest() {
    handleSave();
    setTestStatus('testing');
    setTestMsg('');
    try {
      await onTestConnection();
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
    <div className="card">
      <div className="card-title">{t('settings.aiTitle')}</div>
      <p className="text-sm" style={{ color: 'var(--color-text-muted)', marginBottom: 20 }}>
        {t('settings.aiSubtitle')}
      </p>

      {/* API Key */}
      <FieldRow
        label={t('settings.apiKey')}
        hint={t('settings.apiKeyHint')}
        htmlFor="settings-api-key"
      >
        <div style={{ display: 'flex', gap: 8 }}>
          <input
            id="settings-api-key"
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
            aria-label={showKey ? t('settings.hideKey') : t('settings.showKey')}
          >
            {showKey ? '🙈' : '👁'}
          </button>
        </div>
      </FieldRow>

      {/* Base URL */}
      <FieldRow
        label={t('settings.baseUrl')}
        hint={t('settings.baseUrlHint')}
        htmlFor="settings-base-url"
      >
        <input
          id="settings-base-url"
          type="text"
          value={draft.openai_base_url}
          onChange={(e) => update('openai_base_url', e.target.value)}
          placeholder="https://api.openai.com/v1"
        />
      </FieldRow>

      {/* Custom endpoint privacy warning */}
      {!draft.openai_base_url.includes('openai.com') && draft.openai_base_url && (
        <div className="warning-banner" role="alert">
          <p>{t('settings.customEndpointWarning')}</p>
        </div>
      )}

      {/* Model */}
      <FieldRow
        label={t('settings.model')}
        hint={t('settings.modelHint')}
        htmlFor="settings-custom-model"
      >
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
            id="settings-custom-model"
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
          role={testStatus === 'error' ? 'alert' : 'status'}
          style={{
            marginTop: 12,
            padding: '8px 12px',
            borderRadius: 6,
            fontSize: 13,
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
        className="text-xs"
        style={{ color: 'var(--color-text-muted)', marginTop: 16, marginBottom: 0 }}
      >
        🔒 {t('settings.storageNote')}
      </p>
    </div>
  );
}
