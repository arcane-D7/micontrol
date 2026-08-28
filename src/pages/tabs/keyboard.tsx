import { useState, useEffect, useRef } from 'react';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import { useToast } from '../../contexts/ToastContext';

// ── Types ────────────────────────────────────────────────────────────────────

type HotkeyAction =
  | { type: 'none' }
  | { type: 'focus_micontrol' }
  | { type: 'open_main_window' }
  | { type: 'open_url'; url: string }
  | { type: 'launch_app'; path: string; args: string[] }
  | { type: 'remap_to_key'; vk: number; extended: boolean }
  | { type: 'set_performance_mode'; mode: string }
  | { type: 'cycle_performance_mode'; modes: string[] }
  | { type: 'toggle_ai_brightness' }
  | { type: 'media_control'; action: string }
  | { type: 'script'; interpreter: string; path: string; args: string[] };

interface KeyBinding {
  enabled: boolean;
  action: HotkeyAction;
  label?: string;
}

interface HotkeyMap {
  ai_key: KeyBinding;
  xiaomi_key: KeyBinding;
  copilot_key: KeyBinding;
}

// ── KeyBindingRow component ──────────────────────────────────────────────────

function KeyBindingRow({
  label,
  description,
  binding,
  onChange,
}: {
  label: string;
  description: string;
  binding: KeyBinding;
  onChange: (b: KeyBinding) => void;
}) {
  const { addToast } = useToast();
  const [detecting, setDetecting] = useState(false);
  const [detectedVk, setDetectedVk] = useState<string>('');
  const pollRef = useRef<number | null>(null);

  const actionType = binding.action.type;
  const urlValue = binding.action.type === 'open_url' ? binding.action.url : '';
  const appPath = binding.action.type === 'launch_app' ? binding.action.path : '';
  const remapVk = binding.action.type === 'remap_to_key' ? binding.action.vk : 0xa3;
  const perfMode = binding.action.type === 'set_performance_mode' ? binding.action.mode : 'balance';
  const cycleModes = binding.action.type === 'cycle_performance_mode' ? binding.action.modes : [];
  const mediaAction = binding.action.type === 'media_control' ? binding.action.action : 'volume_up';
  const scriptInterp = binding.action.type === 'script' ? binding.action.interpreter || '' : '';
  const scriptPath = binding.action.type === 'script' ? binding.action.path : '';

  const REMAP_TARGETS: { vk: number; extended: boolean; label: string }[] = [
    { vk: 0xa2, extended: false, label: t('keyboard.remapLCtrl') },
    { vk: 0xa3, extended: true, label: t('keyboard.remapRCtrl') },
    { vk: 0xa4, extended: false, label: t('keyboard.remapLAlt') },
    { vk: 0xa5, extended: true, label: t('keyboard.remapRAlt') },
    { vk: 0xa0, extended: false, label: t('keyboard.remapLShift') },
    { vk: 0xa1, extended: false, label: t('keyboard.remapRShift') },
    { vk: 0x14, extended: false, label: t('keyboard.remapCapsLock') },
    { vk: 0x1b, extended: false, label: t('keyboard.remapEsc') },
    { vk: 0x2e, extended: true, label: t('keyboard.remapDelete') },
    { vk: 0x2f, extended: false, label: t('keyboard.remapHelp') },
  ];

  const PERFORMANCE_MODES = [
    { value: 'silence', label: t('performance.modes.silence') },
    { value: 'long_battery', label: t('performance.modes.longBattery') },
    { value: 'balance', label: t('performance.modes.balance') },
    { value: 'turbo', label: t('performance.modes.turbo') },
    { value: 'decepticon', label: t('performance.modes.decepticon') },
    { value: 'overdrive', label: t('performance.modes.overdrive') },
    { value: 'overdrive_high', label: t('performance.modes.overdriveHigh') },
    { value: 'overdrive_max', label: t('performance.modes.overdriveMax') },
    { value: 'smart_adaptive', label: t('performance.modes.smartAdaptive') },
    { value: 'smart', label: t('performance.modes.smart') },
    { value: 'smart_acceleration', label: t('performance.modes.smartAcceleration') },
  ];

  function setActionType(type: string) {
    const autoEnabled = type !== 'none';
    if (type === 'none') onChange({ ...binding, enabled: false, action: { type: 'none' } });
    else if (type === 'focus_micontrol')
      onChange({ ...binding, enabled: autoEnabled, action: { type: 'focus_micontrol' } });
    else if (type === 'open_main_window')
      onChange({ ...binding, enabled: autoEnabled, action: { type: 'open_main_window' } });
    else if (type === 'open_url')
      onChange({ ...binding, enabled: autoEnabled, action: { type: 'open_url', url: urlValue } });
    else if (type === 'launch_app')
      onChange({
        ...binding,
        enabled: autoEnabled,
        action: { type: 'launch_app', path: appPath, args: [] },
      });
    else if (type === 'remap_to_key') {
      const def = REMAP_TARGETS[0];
      onChange({
        ...binding,
        enabled: autoEnabled,
        action: { type: 'remap_to_key', vk: def.vk, extended: def.extended },
      });
    } else if (type === 'set_performance_mode') {
      onChange({
        ...binding,
        enabled: autoEnabled,
        action: { type: 'set_performance_mode', mode: 'balance' },
      });
    } else if (type === 'cycle_performance_mode') {
      onChange({
        ...binding,
        enabled: autoEnabled,
        action: { type: 'cycle_performance_mode', modes: ['balance', 'turbo'] },
      });
    } else if (type === 'toggle_ai_brightness') {
      onChange({ ...binding, enabled: autoEnabled, action: { type: 'toggle_ai_brightness' } });
    } else if (type === 'media_control') {
      onChange({
        ...binding,
        enabled: autoEnabled,
        action: { type: 'media_control', action: 'play_pause' },
      });
    } else if (type === 'script') {
      onChange({
        ...binding,
        enabled: autoEnabled,
        action: { type: 'script', interpreter: 'powershell', path: '', args: [] },
      });
    }
  }

  async function handleDetect() {
    try {
      const { invoke: invokeFn } = await import('@tauri-apps/api/core');
      await invokeFn('start_key_detect');
      if (pollRef.current !== null) {
        clearInterval(pollRef.current);
        pollRef.current = null;
      }
      setDetecting(true);
      setDetectedVk('…');
      let tries = 0;
      const id = window.setInterval(async () => {
        try {
          tries++;
          const vk = await invokeFn<number>('get_detected_key');
          if (vk !== 0) {
            setDetectedVk(`VK 0x${vk.toString(16).toUpperCase().padStart(2, '0')}`);
            setDetecting(false);
            clearInterval(id);
            pollRef.current = null;
          } else if (tries >= 50) {
            setDetectedVk('');
            setDetecting(false);
            clearInterval(id);
            pollRef.current = null;
          }
        } catch {
          setDetectedVk('');
          setDetecting(false);
          clearInterval(id);
          pollRef.current = null;
        }
      }, 200);
      pollRef.current = id;
    } catch (e) {
      console.error('[keyboard] start_key_detect failed:', e);
      addToast({ message: t('keyboard.loadError'), type: 'error' });
      setDetecting(false);
      setDetectedVk('');
    }
  }

  useEffect(() => {
    return () => {
      if (pollRef.current !== null) {
        clearInterval(pollRef.current);
        pollRef.current = null;
      }
    };
  }, []);

  function handleClear() {
    onChange({ ...binding, enabled: false, action: { type: 'none' } });
    setDetectedVk('');
  }

  const hasAction = actionType !== 'none';

  return (
    <div className="card" style={{ marginBottom: 10, padding: '14px 16px' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
        <label className="toggle-switch" style={{ flexShrink: 0 }}>
          <input
            type="checkbox"
            checked={binding.enabled}
            onChange={(e) => onChange({ ...binding, enabled: e.target.checked })}
          />
          <span className="toggle-track" />
          <span className="toggle-knob" />
        </label>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div className="card-title" style={{ margin: 0, fontSize: 13.5 }}>
            {label}
          </div>
          <div
            style={{
              fontSize: 11.5,
              opacity: 0.55,
              marginTop: 1,
              whiteSpace: 'nowrap',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
            }}
          >
            {description}
          </div>
        </div>
        {detectedVk && !detecting && (
          <span
            style={{
              fontSize: 10.5,
              fontFamily: 'var(--font-mono)',
              padding: '2px 8px',
              borderRadius: 6,
              background: 'var(--surface-2)',
              border: '1px solid var(--border)',
              color: 'var(--accent)',
              flexShrink: 0,
            }}
          >
            {detectedVk}
          </span>
        )}
        <button
          className="btn-secondary"
          onClick={handleDetect}
          disabled={detecting}
          style={{ fontSize: 11, padding: '3px 10px', flexShrink: 0 }}
          title="Press the physical key to detect its VK code (up to 10 s)"
        >
          {detecting
            ? detectedVk === '…'
              ? t('keyboard.detectKeyActive')
              : detectedVk
            : t('keyboard.detectKey')}
        </button>
        {hasAction && (
          <button
            onClick={handleClear}
            title="Clear this key binding"
            style={{
              flexShrink: 0,
              background: 'none',
              border: '1px solid var(--border)',
              borderRadius: 6,
              padding: '3px 8px',
              cursor: 'pointer',
              fontSize: 11,
              color: 'var(--color-warning)',
              opacity: 0.8,
              lineHeight: 1.4,
            }}
          >
            ✕
          </button>
        )}
      </div>
      <div
        style={{
          display: 'flex',
          gap: 8,
          flexWrap: 'wrap',
          alignItems: 'center',
          marginTop: 10,
          paddingTop: 10,
          borderTop: '1px solid var(--border)',
        }}
      >
        <select
          className="select-input"
          value={actionType}
          onChange={(e) => setActionType(e.target.value)}
          style={{ minWidth: 200, fontSize: 12 }}
        >
          <option value="none">{t('keyboard.actionNone')}</option>
          <option value="focus_micontrol">{t('keyboard.actionFocusMicontrol')}</option>
          <option value="open_main_window">{t('keyboard.actionOpenMainWindow')}</option>
          <option value="remap_to_key">{t('keyboard.actionRemapToKey')}</option>
          <option value="set_performance_mode">{t('keyboard.actionSetPerformanceMode')}</option>
          <option value="cycle_performance_mode">{t('keyboard.actionCyclePerformanceMode')}</option>
          <option value="toggle_ai_brightness">{t('keyboard.actionToggleAiBrightness')}</option>
          <option value="media_control">{t('keyboard.actionMediaControl')}</option>
          <option value="open_url">{t('keyboard.actionOpenUrl')}</option>
          <option value="launch_app">{t('keyboard.actionLaunchApp')}</option>
          <option value="script">{t('keyboard.actionScript')}</option>
        </select>
        {actionType === 'remap_to_key' && (
          <select
            className="select-input"
            value={remapVk}
            onChange={(e) => {
              const vk = Number(e.target.value);
              const target = REMAP_TARGETS.find((rt) => rt.vk === vk) ?? REMAP_TARGETS[0];
              onChange({
                ...binding,
                action: { type: 'remap_to_key', vk: target.vk, extended: target.extended },
              });
            }}
            style={{ fontSize: 12 }}
          >
            {REMAP_TARGETS.map((rt) => (
              <option key={rt.vk} value={rt.vk}>
                {rt.label}
              </option>
            ))}
          </select>
        )}
        {actionType === 'set_performance_mode' && (
          <select
            className="select-input"
            value={perfMode}
            onChange={(e) =>
              onChange({
                ...binding,
                action: { type: 'set_performance_mode', mode: e.target.value },
              })
            }
            style={{ fontSize: 12 }}
          >
            {PERFORMANCE_MODES.map((m) => (
              <option key={m.value} value={m.value}>
                {m.label}
              </option>
            ))}
          </select>
        )}
        {actionType === 'cycle_performance_mode' && (
          <div
            style={{
              display: 'flex',
              flexWrap: 'wrap',
              gap: 6,
              alignItems: 'center',
            }}
          >
            {PERFORMANCE_MODES.map((m) => {
              const checked = cycleModes.includes(m.value);
              return (
                <label
                  key={m.value}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 4,
                    fontSize: 11,
                    cursor: 'pointer',
                    padding: '2px 8px',
                    borderRadius: 6,
                    border: `1px solid ${checked ? 'var(--accent)' : 'var(--border)'}`,
                    background: checked ? 'var(--surface-2)' : 'transparent',
                    color: checked ? 'var(--accent)' : 'var(--text-muted)',
                  }}
                >
                  <input
                    type="checkbox"
                    checked={checked}
                    onChange={(e) => {
                      const next = e.target.checked
                        ? [...cycleModes, m.value]
                        : cycleModes.filter((v) => v !== m.value);
                      onChange({
                        ...binding,
                        action: { type: 'cycle_performance_mode', modes: next },
                      });
                    }}
                    style={{
                      margin: 0,
                      appearance: 'none',
                      WebkitAppearance: 'none',
                      width: 0,
                      height: 0,
                    }}
                  />
                  {m.label}
                </label>
              );
            })}
          </div>
        )}
        {actionType === 'media_control' && (
          <select
            className="select-input"
            value={mediaAction}
            onChange={(e) =>
              onChange({ ...binding, action: { type: 'media_control', action: e.target.value } })
            }
            style={{ fontSize: 12 }}
          >
            <option value="volume_up">{t('keyboard.mediaVolUp')}</option>
            <option value="volume_down">{t('keyboard.mediaVolDown')}</option>
            <option value="mute">{t('keyboard.mediaMute')}</option>
            <option value="play_pause">{t('keyboard.mediaPlayPause')}</option>
            <option value="next">{t('keyboard.mediaNext')}</option>
            <option value="prev">{t('keyboard.mediaPrev')}</option>
          </select>
        )}
        {actionType === 'open_url' && (
          <input
            className="text-input"
            type="text"
            placeholder={t('keyboard.urlPlaceholder')}
            value={urlValue}
            onChange={(e) =>
              onChange({ ...binding, action: { type: 'open_url', url: e.target.value } })
            }
            style={{ flex: 1, minWidth: 200, fontSize: 12 }}
          />
        )}
        {actionType === 'launch_app' && (
          <input
            className="text-input"
            type="text"
            placeholder={t('keyboard.appPlaceholder')}
            value={appPath}
            onChange={(e) =>
              onChange({
                ...binding,
                action: { type: 'launch_app', path: e.target.value, args: [] },
              })
            }
            style={{ flex: 1, minWidth: 200, fontSize: 12 }}
          />
        )}
        {actionType === 'script' && (
          <select
            className="select-input"
            value={scriptInterp}
            onChange={(e) =>
              onChange({
                ...binding,
                action: { type: 'script', interpreter: e.target.value, path: scriptPath, args: [] },
              })
            }
            style={{ fontSize: 12 }}
          >
            <option value="">{t('keyboard.scriptInterpreterDirect')}</option>
            <option value="powershell">{t('keyboard.scriptInterpreterPowershell')}</option>
            <option value="cmd">{t('keyboard.scriptInterpreterCmd')}</option>
          </select>
        )}
        {actionType === 'script' && (
          <input
            className="text-input"
            type="text"
            placeholder={t('keyboard.scriptPathPlaceholder')}
            value={scriptPath}
            onChange={(e) =>
              onChange({
                ...binding,
                action: {
                  type: 'script',
                  interpreter: scriptInterp,
                  path: e.target.value,
                  args: [],
                },
              })
            }
            style={{ flex: 1, minWidth: 200, fontSize: 12 }}
          />
        )}
      </div>
    </div>
  );
}

// ── KeyboardTab ──────────────────────────────────────────────────────────────

export default function KeyboardTab() {
  const [config, setConfig] = useState<HotkeyMap | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [hookActive, setHookActive] = useState<boolean | null>(null);
  const [fnKeyMode, setFnKeyMode] = useState<'multimedia' | 'function_key' | null>(null);
  const [fnKeyToggling, setFnKeyToggling] = useState(false);
  const { addToast } = useToast();
  const timeoutRefs = useRef<number[]>([]);

  useEffect(() => {
    return () => {
      timeoutRefs.current.forEach((id) => clearTimeout(id));
      timeoutRefs.current = [];
    };
  }, []);

  useEffect(() => {
    import('@tauri-apps/api/core')
      .then(({ invoke: invokeFn }) => {
        invokeFn<HotkeyMap>('get_hotkey_config')
          .then(setConfig)
          .catch((e) => {
            console.error('get_hotkey_config', e);
            addToast({ message: t('keyboard.loadError'), type: 'error' });
          });
        invokeFn<boolean>('is_hook_active')
          .then(setHookActive)
          .catch(() => setHookActive(false));
        invokeFn<{ mode: 'multimedia' | 'function_key'; fn_lock_enabled: boolean }>(
          'get_function_key',
        )
          .then((s) => setFnKeyMode(s.mode))
          .catch(() => setFnKeyMode(null));
      })
      .catch((e) => {
        console.error('Failed to import Tauri core:', e);
        addToast({ message: t('keyboard.loadError'), type: 'error' });
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function save() {
    if (!config) return;
    setSaving(true);
    try {
      const { invoke: invokeFn } = await import('@tauri-apps/api/core');
      await invokeFn('set_hotkey_config', { config });
      setSaved(true);
      addToast({ message: t('keyboard.saved'), type: 'success' });
      const savedTimeout = window.setTimeout(() => setSaved(false), 2000);
      timeoutRefs.current.push(savedTimeout);
    } catch (e) {
      console.error('set_hotkey_config', e);
      addToast({
        message: `${t('keyboard.saveError')}: ${String(e)}`,
        type: 'error',
        onRetry: save,
      });
    } finally {
      setSaving(false);
    }
  }

  if (!config) {
    return (
      <>
        <PageHeader title={t('keyboard.title')} subtitle={t('keyboard.subtitle')} />
        <div className="card" style={{ opacity: 0.6 }}>
          {t('keyboard.loading')}
        </div>
      </>
    );
  }

  return (
    <>
      <PageHeader title={t('keyboard.title')} subtitle={t('keyboard.subtitle')} />
      {hookActive !== null && (
        <div
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 6,
            fontSize: 12,
            marginBottom: 12,
            opacity: 0.8,
            color: hookActive ? 'var(--color-success)' : 'var(--color-warning)',
          }}
        >
          <span
            style={{
              width: 8,
              height: 8,
              borderRadius: '50%',
              background: 'currentColor',
              display: 'inline-block',
            }}
          />
          {hookActive ? t('keyboard.hookActive') : t('keyboard.hookInactive')}
        </div>
      )}
      <KeyBindingRow
        label={t('keyboard.aiKey')}
        description={t('keyboard.aiKeyDesc')}
        binding={config.ai_key}
        onChange={(b) => setConfig({ ...config, ai_key: b })}
      />
      <KeyBindingRow
        label={t('keyboard.xiaomiKey')}
        description={t('keyboard.xiaomiKeyDesc')}
        binding={config.xiaomi_key}
        onChange={(b) => setConfig({ ...config, xiaomi_key: b })}
      />
      <KeyBindingRow
        label={t('keyboard.copilotKey')}
        description={t('keyboard.copilotKeyDesc')}
        binding={config.copilot_key}
        onChange={(b) => setConfig({ ...config, copilot_key: b })}
      />

      {/* Fn Key Lock Card */}
      <div className="card" style={{ marginTop: 16 }}>
        <h3>⌨️ {t('keyboard.fnKeyTitle')}</h3>
        <p className="text-muted" style={{ fontSize: 12, marginBottom: 12 }}>
          {t('keyboard.fnKeyDesc')}
        </p>
        {fnKeyMode === null ? (
          <p className="text-muted">{t('common.loading')}</p>
        ) : (
          <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap' }}>
            <button
              className={fnKeyMode === 'multimedia' ? 'btn btn-primary' : 'btn btn-secondary'}
              onClick={async () => {
                setFnKeyToggling(true);
                try {
                  const { invoke: invokeFn } = await import('@tauri-apps/api/core');
                  await invokeFn('set_function_key', { mode: 'multimedia' });
                  setFnKeyMode('multimedia');
                } catch (e) {
                  addToast({ message: String(e), type: 'error' });
                }
                setFnKeyToggling(false);
              }}
              disabled={fnKeyToggling}
            >
              🎵 {t('keyboard.fnKeyMultimedia')}
            </button>
            <button
              className={fnKeyMode === 'function_key' ? 'btn btn-primary' : 'btn btn-secondary'}
              onClick={async () => {
                setFnKeyToggling(true);
                try {
                  const { invoke: invokeFn } = await import('@tauri-apps/api/core');
                  await invokeFn('set_function_key', { mode: 'function_key' });
                  setFnKeyMode('function_key');
                } catch (e) {
                  addToast({ message: String(e), type: 'error' });
                }
                setFnKeyToggling(false);
              }}
              disabled={fnKeyToggling}
            >
              ⌨️ {t('keyboard.fnKeyFunction')}
            </button>
          </div>
        )}
        <p className="text-muted" style={{ fontSize: 12, marginTop: 8 }}>
          {fnKeyMode === 'multimedia'
            ? t('keyboard.fnKeyMultimediaDesc')
            : fnKeyMode === 'function_key'
              ? t('keyboard.fnKeyFunctionDesc')
              : ''}
        </p>
      </div>

      <div style={{ display: 'flex', gap: 8, marginTop: 4 }}>
        <button className="btn-primary" onClick={save} disabled={saving} style={{ minWidth: 100 }}>
          {saving ? t('keyboard.saving') : saved ? t('keyboard.saved') : t('keyboard.save')}
        </button>
      </div>
    </>
  );
}
