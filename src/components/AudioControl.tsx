import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useToast } from '../contexts/ToastContext';
import { t } from '../hooks/useI18n';
import type { AudioVolumeResult } from '../hooks/useHardware';

interface AudioDevice {
  name: string;
  id: string;
  direction: string;
  is_default: boolean;
  volume: number;
  muted: boolean;
}

interface AudioDeviceList {
  playback: AudioDevice[];
  capture: AudioDevice[];
}

interface AudioControlProps {
  audioState: AudioVolumeResult | null;
  loading: boolean;
  onVolumeChange: (volumeFraction: number) => Promise<void>;
  onMuteToggle: (muted: boolean) => Promise<void>;
}

export default function AudioControl({
  audioState,
  loading,
  onVolumeChange,
  onMuteToggle,
}: AudioControlProps) {
  const [devices, setDevices] = useState<AudioDeviceList | null>(null);
  const { addToast } = useToast();

  // Derive local state from the batched hardware poll (useHardware)
  const volume = audioState?.volume ?? 50;
  const muted = audioState?.muted ?? false;

  const loadAudioDevices = async () => {
    try {
      const list = await invoke<AudioDeviceList>('get_audio_devices');
      setDevices(list);
    } catch (e) {
      console.error('Failed to load audio devices:', e);
      addToast({ message: t('audio.loadDevicesError'), type: 'error', onRetry: loadAudioDevices });
    }
  };

  // Load device list once on mount
  useEffect(() => {
    void loadAudioDevices();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleVolumeChange = async (newVolume: number) => {
    try {
      // onVolumeChange expects 0-1 fraction (setMasterVolume contract)
      await onVolumeChange(newVolume / 100);
    } catch (e) {
      addToast(`Volume error: ${String(e)}`, 'error');
    }
  };

  const handleMuteToggle = async () => {
    try {
      await onMuteToggle(!muted);
    } catch (e) {
      addToast(`Mute error: ${String(e)}`, 'error');
    }
  };

  const volumeIcon = muted ? '🔇' : volume > 66 ? '🔊' : volume > 33 ? '🔉' : '🔈';

  return (
    <div className="card">
      <div className="card-title">{t('audio.title')}</div>
      <p className="page-subtitle">{t('audio.subtitle')}</p>

      {/* Volume Slider */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 16 }}>
        <button
          className="mute-btn"
          onClick={handleMuteToggle}
          style={{
            background: 'none',
            border: 'none',
            cursor: 'pointer',
            fontSize: 24,
            padding: 4,
            borderRadius: 'var(--r-xs)',
          }}
          title={muted ? t('audio.unmute') : t('audio.mute')}
          aria-label={muted ? t('audio.unmute') : t('audio.mute')}
        >
          <span aria-hidden="true">{volumeIcon}</span>
        </button>
        <input
          type="range"
          min={0}
          max={100}
          value={muted ? 0 : volume}
          onChange={(e) => handleVolumeChange(Number(e.target.value))}
          disabled={loading}
          style={{ flex: 1, accentColor: 'var(--accent)' }}
        />
        <span style={{ minWidth: 40, textAlign: 'right', fontVariantNumeric: 'tabular-nums' }}>
          {muted ? t('audio.muted') : `${volume}%`}
        </span>
      </div>

      {/* Device List — loading state */}
      {devices === null && (
        <div className="loading-container" role="status" aria-live="polite">
          <span className="spinner" />
          <span>{t('audio.loadingDevices')}</span>
          <span className="sr-only">{t('common.loading')}</span>
        </div>
      )}

      {/* Device List — loaded */}
      {devices !== null && devices.playback.length > 0 && (
        <div style={{ marginTop: 12 }}>
          <div style={{ fontWeight: 600, marginBottom: 8, color: 'var(--text-dim)', fontSize: 13 }}>
            {t('audio.playbackDevices')}
          </div>
          {devices.playback.slice(0, 5).map((d) => (
            <div
              key={d.id}
              className="stat-row"
              style={{
                padding: '6px 8px',
                borderRadius: 'var(--r-xs)',
                background: d.is_default ? 'var(--bg-hover)' : 'transparent',
                marginBottom: 4,
              }}
            >
              <span style={{ flex: 1, fontSize: 13 }}>{d.name}</span>
              <span style={{ fontSize: 11, color: 'var(--text-dim)' }}>
                {d.is_default ? `✓ ${t('audio.defaultDevice')}` : ''}
              </span>
            </div>
          ))}
        </div>
      )}

      {/* Device List — empty state */}
      {devices !== null && devices.playback.length === 0 && (
        <p style={{ marginTop: 12, color: 'var(--text-muted)' }}>{t('audio.noDevices')}</p>
      )}
    </div>
  );
}
