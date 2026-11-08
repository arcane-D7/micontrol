import { memo, useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import ToggleSwitch from '../../components/ToggleSwitch';
import { t } from '../../hooks/useI18n';
import { getUserFriendlyMessage, parseErrorResponse, type TranslateFn } from '../../types/error';
import { PageHeader } from './PageHeader';
import AudioControl from '../../components/AudioControl';
import type { Hardware } from './shared';
import { Icon } from '../../components/ui/icons';

// `t` from useI18n types keys as a literal union; error.ts' TranslateFn takes
// any string — wrap it (matches the pattern used across the codebase).
const translate: TranslateFn = (key) => t(key as never);

interface AudioEffectsStatus {
  mic_noise_canceling: boolean;
  speaker_noise_canceling: boolean;
  voice_focus: boolean;
  voice_clarity_available: boolean;
  voice_clarity_reason: string;
  applies_to_audio: boolean;
}

type EffectKey = 'mic_noise_canceling' | 'speaker_noise_canceling' | 'voice_focus';

const EFFECT_COMMANDS: Record<EffectKey, string> = {
  mic_noise_canceling: 'set_mic_noise_canceling',
  speaker_noise_canceling: 'set_speaker_noise_canceling',
  voice_focus: 'set_voice_focus',
};

/**
 * Voice Clarity copy driven by the backend's real capability probe
 * (`os-unsupported` / `no-npu` / `unknown-os` / `detected`) instead of a
 * blanket "not available" line.
 */
function clarityMessage(effects: AudioEffectsStatus): string {
  if (effects.voice_clarity_available) return t('audio.voiceClarityAvailable');
  switch (effects.voice_clarity_reason) {
    case 'no-npu':
      return t('audio.voiceClarityNoNpu');
    case 'os-unsupported':
      return t('audio.voiceClarityOsUnsupported');
    default:
      return t('audio.voiceClarityUnknown');
  }
}

interface Props {
  hw: Hardware;
}

function AudioTab({ hw }: Props) {
  const [effects, setEffects] = useState<AudioEffectsStatus | null>(null);
  const [effectsLoading, setEffectsLoading] = useState(true);
  const [pendingKey, setPendingKey] = useState<EffectKey | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const fetchEffects = useCallback(async () => {
    try {
      const e = await invoke<AudioEffectsStatus>('get_audio_effects');
      setEffects(e);
      setErrorMsg(null);
    } catch {
      setEffects(null);
    }
    setEffectsLoading(false);
  }, []);

  useEffect(() => {
    void fetchEffects();
  }, [fetchEffects]);

  const handleToggle = async (key: EffectKey, next: boolean) => {
    if (!effects || pendingKey) return;
    setPendingKey(key);
    try {
      await invoke(EFFECT_COMMANDS[key], { enabled: next });
      setEffects((prev) => (prev ? { ...prev, [key]: next } : prev));
      setErrorMsg(null);
    } catch (e) {
      setErrorMsg(getUserFriendlyMessage(parseErrorResponse(e), translate));
    } finally {
      setPendingKey(null);
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
        onSetDefaultDevice={hw.setDefaultAudioDevice}
      />

      {/* Audio effects — saved preferences, honestly labelled */}
      <div className="card" style={{ marginTop: 16 }}>
        <h3>
          <Icon name="microphone" size={16} /> {t('audio.effectsTitle')}
        </h3>
        <p className="text-muted" style={{ fontSize: 12, marginBottom: 12 }}>
          {t('audio.effectsDesc')}
        </p>

        {errorMsg && (
          <div className="alert alert-error" style={{ marginBottom: 12 }}>
            <Icon name="alert-triangle" size={16} /> {errorMsg}
          </div>
        )}

        {effectsLoading ? (
          <p className="text-muted">{t('common.loading')}</p>
        ) : effects ? (
          <>
            {!effects.applies_to_audio && (
              <div className="alert alert-warn" style={{ marginBottom: 12 }} role="status">
                <Icon name="info" size={16} /> {t('audio.preferenceOnly')}
              </div>
            )}
            <p className="text-muted" style={{ fontSize: 12, marginBottom: 8 }}>
              {clarityMessage(effects)}
            </p>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
              <label
                className="toggle-row"
                style={{ display: 'flex', alignItems: 'center', gap: 8 }}
              >
                <ToggleSwitch
                  checked={effects.mic_noise_canceling}
                  onChange={(next) => void handleToggle('mic_noise_canceling', next)}
                  disabled={pendingKey !== null}
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
                  onChange={(next) => void handleToggle('speaker_noise_canceling', next)}
                  disabled={pendingKey !== null}
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
                  onChange={(next) => void handleToggle('voice_focus', next)}
                  disabled={pendingKey !== null}
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
