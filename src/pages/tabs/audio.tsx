import { memo, useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import ToggleSwitch from '../../components/ToggleSwitch';
import { t } from '../../hooks/useI18n';
import { getUserFriendlyMessage, parseErrorResponse, type TranslateFn } from '../../types/error';
import { PageHeader } from './PageHeader';
import AudioControl from '../../components/AudioControl';
import type { Hardware } from './shared';

// `t` from useI18n types keys as a literal union; error.ts' TranslateFn takes
// any string — wrap it (matches the pattern used across the codebase).
const translate: TranslateFn = (key) => t(key as never);

interface AudioEffectsStatus {
  mic_noise_canceling: boolean;
  speaker_noise_canceling: boolean;
  voice_focus: boolean;
  voice_clarity_available: boolean;
}

interface Props {
  hw: Hardware;
}

function AudioTab({ hw }: Props) {
  const [effects, setEffects] = useState<AudioEffectsStatus | null>(null);
  const [effectsLoading, setEffectsLoading] = useState(true);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const fetchEffects = useCallback(async () => {
    try {
      const e = await invoke<AudioEffectsStatus>('get_audio_effects');
      setEffects(e);
    } catch {
      setEffects(null);
    }
    setEffectsLoading(false);
  }, []);

  useEffect(() => {
    void fetchEffects();
  }, [fetchEffects]);

  const handleToggle = async (key: keyof AudioEffectsStatus) => {
    if (!effects) return;
    const newVal = !effects[key];
    const commandMap: Record<string, string> = {
      mic_noise_canceling: 'set_mic_noise_canceling',
      speaker_noise_canceling: 'set_speaker_noise_canceling',
      voice_focus: 'set_voice_focus',
    };
    const cmd = commandMap[key];
    if (!cmd) return;
    try {
      await invoke(cmd, { enabled: newVal });
      setEffects({ ...effects, [key]: newVal });
    } catch (e) {
      setErrorMsg(getUserFriendlyMessage(parseErrorResponse(e), translate));
    }
  };

  return (
    <>
      <PageHeader title={t('audio.pageTitle')} />
      <AudioControl
        audioState={hw.audioState}
        loading={hw.loading}
        onVolumeChange={hw.setMasterVolume}
        onMuteToggle={hw.setMasterMute}
      />

      {/* Audio Effects / AI Noise Cancellation */}
      <div className="card" style={{ marginTop: 16 }}>
        <h3>🎙️ {t('audio.effectsTitle')}</h3>
        <p className="text-muted" style={{ fontSize: 12, marginBottom: 12 }}>
          {t('audio.effectsDesc')}
        </p>

        {errorMsg && (
          <div className="alert alert-error" style={{ marginBottom: 12 }}>
            ⚠ {errorMsg}
          </div>
        )}

        {effectsLoading ? (
          <p className="text-muted">{t('common.loading')}</p>
        ) : effects ? (
          <>
            {!effects.voice_clarity_available && (
              <p className="text-muted" style={{ fontSize: 12, marginBottom: 8 }}>
                {t('audio.voiceClarityUnavailable')}
              </p>
            )}
            <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
              <label
                className="toggle-row"
                style={{ display: 'flex', alignItems: 'center', gap: 8 }}
              >
                <ToggleSwitch
                  checked={effects.mic_noise_canceling}
                  onChange={() => handleToggle('mic_noise_canceling')}
                  ariaLabel={t('audio.micNoiseCanceling')}
                />
                <div>
                  <div>{t('audio.micNoiseCanceling')}</div>
                  <span className="text-muted" style={{ fontSize: 12 }}>
                    {t('audio.micNoiseCancelingDesc')}
                  </span>
                </div>
              </label>
              <label
                className="toggle-row"
                style={{ display: 'flex', alignItems: 'center', gap: 8 }}
              >
                <ToggleSwitch
                  checked={effects.speaker_noise_canceling}
                  onChange={() => handleToggle('speaker_noise_canceling')}
                  ariaLabel={t('audio.speakerNoiseCanceling')}
                />
                <div>
                  <div>{t('audio.speakerNoiseCanceling')}</div>
                  <span className="text-muted" style={{ fontSize: 12 }}>
                    {t('audio.speakerNoiseCancelingDesc')}
                  </span>
                </div>
              </label>
              <label
                className="toggle-row"
                style={{ display: 'flex', alignItems: 'center', gap: 8 }}
              >
                <ToggleSwitch
                  checked={effects.voice_focus}
                  onChange={() => handleToggle('voice_focus')}
                  ariaLabel={t('audio.voiceFocus')}
                />
                <div>
                  <div>{t('audio.voiceFocus')}</div>
                  <span className="text-muted" style={{ fontSize: 12 }}>
                    {t('audio.voiceFocusDesc')}
                  </span>
                </div>
              </label>
            </div>
          </>
        ) : (
          <p className="text-muted">{t('audio.effectsNotAvailable')}</p>
        )}
      </div>
    </>
  );
}

export default memo(AudioTab);
