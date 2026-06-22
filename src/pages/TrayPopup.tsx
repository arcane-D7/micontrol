import { invoke } from '@tauri-apps/api/core';
import { useState, useEffect, useRef, useCallback } from 'react';
import { t } from '../hooks/useI18n';
import type { useHardware, PerformanceMode } from '../hooks/useHardware';
import { useSettings } from '../hooks/useSettings';
import { BRIGHTNESS_PRESETS, getActivePreset } from '../lib/brightnessPresets';

type Hardware = ReturnType<typeof useHardware>;

interface Props {
  hardware: Hardware;
}

type ModeEntry = { key: PerformanceMode; label: string; i18nKey: string };

const STANDARD_MODES: ModeEntry[] = [
  { key: 'silence', label: '🔇', i18nKey: 'silence' },
  { key: 'balance', label: '⚖️', i18nKey: 'balance' },
  { key: 'turbo', label: '⚡', i18nKey: 'turbo' },
  { key: 'long_battery', label: '🔋', i18nKey: 'longBattery' },
  { key: 'decepticon', label: '💥', i18nKey: 'decepticon' },
];

const AI_MODES: ModeEntry[] = [
  { key: 'smart', label: '🧠', i18nKey: 'smart' },
  { key: 'smart_acceleration', label: '🚀', i18nKey: 'smartAcceleration' },
];

export default function TrayPopup({ hardware }: Props) {
  const {
    battery,
    performanceMode,
    setPerformanceMode,
    loading,
    display,
    fan,
    audioState,
    setBrightness,
    setAiBrightness,
    setAiBrightnessConfig,
    setFanMode,
    setMasterVolume,
    setMasterMute,
  } = hardware;

  const [showDisplay, setShowDisplay] = useState(false);
  const [showFan, setShowFan] = useState(false);
  const [localBrightness, setLocalBrightness] = useState(display?.brightness ?? 80);
  const [localVolume, setLocalVolume] = useState(audioState?.volume ?? 50);
  const [isAdjustingVolume, setIsAdjustingVolume] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const { isConfigured: hasAiApi, settings, updateKey } = useSettings();

  const requestTrayResize = useCallback(() => {
    const el = rootRef.current;
    if (!el) return;
    const nextHeight = Math.ceil(el.getBoundingClientRect().height);
    if (nextHeight > 0) {
      void invoke('resize_tray_popup', { height: nextHeight });
    }
  }, []);

  // ── Tray window opacity ───────────────────────────────────────────────────
  const [opacity, setOpacityState] = useState(() =>
    Math.max(0.3, Math.min(1.0, settings.tray_opacity ?? 1.0)),
  );
  // Apply stored opacity on mount
  useEffect(() => {
    document.documentElement.style.opacity = String(opacity);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function handleOpacityChange(v: number) {
    setOpacityState(v);
    updateKey('tray_opacity', v);
    document.documentElement.style.opacity = String(v);
  }

  // Auto-resize the OS window to match content height, growing upward.
  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    let rafId = 0;
    const observer = new ResizeObserver((entries) => {
      cancelAnimationFrame(rafId);
      rafId = requestAnimationFrame(() => {
        const h = entries[0]?.contentRect.height;
        if (h && h > 0) {
          void invoke('resize_tray_popup', { height: Math.ceil(h) });
        }
      });
    });
    observer.observe(el);
    requestTrayResize();

    const onVisibilityChange = () => {
      if (document.visibilityState === 'visible') {
        requestAnimationFrame(requestTrayResize);
      }
    };
    document.addEventListener('visibilitychange', onVisibilityChange);

    return () => {
      document.removeEventListener('visibilitychange', onVisibilityChange);
      observer.disconnect();
      cancelAnimationFrame(rafId);
    };
  }, [requestTrayResize]);

  useEffect(() => {
    requestAnimationFrame(requestTrayResize);
  }, [showDisplay, showFan, requestTrayResize]);

  // Sync local brightness when hardware updates
  useEffect(() => {
    if (display?.brightness !== undefined) setLocalBrightness(display.brightness);
  }, [display?.brightness]);

  useEffect(() => {
    if (!isAdjustingVolume && audioState?.volume !== undefined) {
      setLocalVolume(audioState.volume);
    }
  }, [audioState?.volume, isAdjustingVolume]);

  useEffect(() => {
    // Audio is polled by useHardware hook every 2s — no need to duplicate here
  }, []);

  // Audio is polled by useHardware hook globally — no duplicate poll needed here

  const openMainWindow = async () => {
    await invoke('open_main_window');
  };

  const handleVolumeCommit = () => {
    setIsAdjustingVolume(false);
    void setMasterVolume(localVolume / 100);
  };

  return (
    <div className="tray-popup" ref={rootRef}>
      <div className="tray-header">
        <span className="tray-title">MiControl</span>
        <button
          className="btn btn-secondary"
          style={{ padding: '4px 10px', fontSize: 12 }}
          onClick={() => void openMainWindow()}
        >
          {t('tray.openApp')}
        </button>
      </div>

      <div className="tray-body">
        {/* Performance mode */}
        <div className="tray-section">
          <div className="tray-section-label">{t('tray.performance')}</div>
          {/* Standard modes */}
          <div className="tray-mode-row">
            {STANDARD_MODES.map((m) => (
              <button
                key={m.key}
                className={`tray-mode-btn ${performanceMode === m.key ? 'active' : ''}`}
                onClick={() => void setPerformanceMode(m.key)}
                disabled={loading}
                title={t(`performance.modes.${m.i18nKey}` as Parameters<typeof t>[0])}
              >
                {m.label}
              </button>
            ))}
          </div>
          {/* AI modes — disabled when no API key is configured */}
          <div className="tray-mode-row" style={{ marginTop: 4, opacity: hasAiApi ? 1 : 0.4 }}>
            {AI_MODES.map((m) => (
              <button
                key={m.key}
                className={`tray-mode-btn ${performanceMode === m.key ? 'active' : ''}`}
                onClick={() => void setPerformanceMode(m.key)}
                disabled={loading || !hasAiApi}
                title={
                  !hasAiApi
                    ? 'Configure API key in Settings to unlock AI modes'
                    : t(`performance.modes.${m.i18nKey}` as Parameters<typeof t>[0])
                }
              >
                {m.label}
              </button>
            ))}
          </div>
        </div>

        {/* Battery */}
        {battery && (
          <div className="tray-section">
            <div className="tray-section-label">{t('tray.battery')}</div>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <div style={{ flex: 1 }}>
                <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 6 }}>
                  <span style={{ fontSize: 13, fontWeight: 600 }}>{battery.level}%</span>
                  <span className={`badge ${battery.is_charging ? 'success' : 'warning'}`}>
                    {battery.is_charging ? t('battery.charging') : t('battery.discharging')}
                  </span>
                </div>
                <div className="progress-bar">
                  <div
                    className={`progress-fill ${battery.level > 30 ? 'battery' : battery.level > 15 ? 'battery warning' : 'battery critical'}`}
                    style={{ width: `${battery.level}%` }}
                  />
                </div>
              </div>
            </div>
          </div>
        )}

        {/* Xiaomi-style quick cards */}
        <div className="tray-section">
          <div className="tray-section-label">Cross-Device</div>
          <div className="tray-quick-grid">
            <div className="tray-quick-card">
              <div className="tray-quick-head">
                <span className="tray-quick-title">🔊 Audio</span>
                <button
                  className="tray-chip-btn"
                  onClick={() => void setMasterMute(!(audioState?.muted ?? false))}
                >
                  {(audioState?.muted ?? false) ? 'Unmute' : 'Mute'}
                </button>
              </div>
              <div className="tray-quick-meta">
                {audioState?.volume ?? 50}% • {(audioState?.muted ?? false) ? 'Muted' : 'On'}
              </div>
              <input
                type="range"
                min={0}
                max={100}
                value={localVolume}
                onPointerDown={() => setIsAdjustingVolume(true)}
                onChange={(e) => setLocalVolume(Number(e.target.value))}
                onMouseUp={handleVolumeCommit}
                onTouchEnd={handleVolumeCommit}
                onBlur={handleVolumeCommit}
                style={{ width: '100%' }}
              />
            </div>

            {/* WiFi quick status */}
            <div className="tray-quick-card">
              <div className="tray-quick-head">
                <span className="tray-quick-title">📶 WiFi</span>
              </div>
              <div className="tray-quick-meta">{t('tray.openApp')} → WiFi Manager</div>
            </div>
          </div>
        </div>

        {/* Quick actions */}
        <div className="tray-section">
          <div className="tray-section-label">{t('tray.quickActions')}</div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            {/* ── Display Settings ─────────────────────────── */}
            <button
              className="btn btn-secondary"
              style={{ width: '100%', justifyContent: 'space-between' }}
              onClick={() => setShowDisplay((v) => !v)}
            >
              <span>🖥️ {t('tray.displaySettings')}</span>
              <span style={{ fontSize: 10, opacity: 0.6 }}>{showDisplay ? '▲' : '▼'}</span>
            </button>

            {showDisplay && (
              <div
                style={{
                  padding: '10px 12px',
                  background: 'var(--surface-2, rgba(255,255,255,0.04))',
                  borderRadius: 'var(--r-sm)',
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 10,
                }}
              >
                {/* Brightness slider */}
                <div>
                  <div
                    style={{
                      display: 'flex',
                      justifyContent: 'space-between',
                      marginBottom: 4,
                      fontSize: 11,
                      color: 'var(--text-muted)',
                    }}
                  >
                    <span>{t('display.brightness')}</span>
                    <span style={{ fontFamily: 'var(--font-mono)', color: 'var(--text)' }}>
                      {localBrightness}%
                    </span>
                  </div>
                  <input
                    type="range"
                    min={10}
                    max={100}
                    value={localBrightness}
                    onChange={(e) => setLocalBrightness(Number(e.target.value))}
                    onMouseUp={() => void setBrightness(localBrightness)}
                    onTouchEnd={() => void setBrightness(localBrightness)}
                    style={{ width: '100%' }}
                  />
                </div>

                {/* Auto-brightness toggle */}
                <div
                  style={{
                    display: 'flex',
                    justifyContent: 'space-between',
                    alignItems: 'center',
                    cursor: 'pointer',
                  }}
                  onClick={() => void setAiBrightness(!(display?.ai_brightness ?? false))}
                >
                  <span style={{ fontSize: 12 }}>{t('tray.autoBrightness')}</span>
                  <label className="toggle-switch" onClick={(e) => e.stopPropagation()}>
                    <input
                      type="checkbox"
                      checked={display?.ai_brightness ?? false}
                      onChange={(e) => void setAiBrightness(e.target.checked)}
                    />
                    <span className="toggle-track" />
                    <span className="toggle-knob" />
                  </label>
                </div>

                {/* Brightness presets — only when auto-brightness is on */}
                {display?.ai_brightness && display.ai_brightness_config && (
                  <div>
                    <div style={{ fontSize: 11, color: 'var(--text-muted)', marginBottom: 6 }}>
                      {t('tray.brightnessPresets')}
                    </div>
                    <div className="tray-mode-row">
                      {BRIGHTNESS_PRESETS.map((p) => {
                        const cfg = display.ai_brightness_config;
                        const isActive = getActivePreset(cfg) === p.key;
                        return (
                          <button
                            key={p.key}
                            className={`tray-mode-btn${isActive ? ' active' : ''}`}
                            title={p.hint}
                            onClick={() =>
                              void setAiBrightnessConfig({
                                enabled: true,
                                ...p.config,
                              })
                            }
                          >
                            {p.icon} {p.label}
                          </button>
                        );
                      })}
                    </div>
                  </div>
                )}
              </div>
            )}

            {/* ── Fan Control ──────────────────────────────── */}
            <button
              className="btn btn-secondary"
              style={{ width: '100%', justifyContent: 'space-between' }}
              onClick={() => setShowFan((v) => !v)}
            >
              <span>💨 {t('tray.fanControl')}</span>
              <span style={{ fontSize: 10, opacity: 0.6 }}>{showFan ? '▲' : '▼'}</span>
            </button>

            {showFan && (
              <div
                style={{
                  padding: '10px 12px',
                  background: 'var(--surface-2, rgba(255,255,255,0.04))',
                  borderRadius: 'var(--r-sm)',
                }}
              >
                <div style={{ fontSize: 11, color: 'var(--text-muted)', marginBottom: 6 }}>
                  {t('tray.fanMode')}
                </div>
                <div className="tray-mode-row">
                  {(['auto', 'fixed', 'off'] as const).map((mode) => {
                    const shortLabel =
                      mode === 'auto' ? 'Auto' : mode === 'fixed' ? 'Fixed' : 'Off';
                    return (
                      <button
                        key={mode}
                        className={`tray-mode-btn${fan?.mode === mode ? ' active' : ''}`}
                        onClick={() => void setFanMode(mode)}
                        disabled={loading}
                        title={t(`fan.modes.${mode}` as Parameters<typeof t>[0])}
                      >
                        {mode === 'auto' ? '🔄' : mode === 'fixed' ? '🔧' : '🔇'} {shortLabel}
                      </button>
                    );
                  })}
                </div>
                {fan && (
                  <div
                    style={{
                      marginTop: 8,
                      fontSize: 11,
                      color: 'var(--text-muted)',
                      display: 'flex',
                      gap: 12,
                    }}
                  >
                    <span>{fan.speed_rpm} RPM</span>
                    <span>{fan.gpu_temp_celsius}°C GPU</span>
                  </div>
                )}
              </div>
            )}
          </div>
        </div>

        {/* Appearance — opacity slider */}
        <div className="tray-section">
          <div className="tray-section-label">🎨 {t('tray.appearance')}</div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span style={{ fontSize: 11, color: 'var(--text-muted)', whiteSpace: 'nowrap' }}>
              {t('tray.opacity')}: {Math.round(opacity * 100)}%
            </span>
            <input
              type="range"
              min={30}
              max={100}
              value={Math.round(opacity * 100)}
              onChange={(e) => handleOpacityChange(Number(e.target.value) / 100)}
              style={{ flex: 1 }}
            />
          </div>
        </div>
      </div>
    </div>
  );
}
