import { t } from '../hooks/useI18n';
import type { TouchpadInfo, HardwareCapabilities } from '../hooks/useHardware';
import ToggleRow from './ToggleRow';

interface Props {
  touchpad: TouchpadInfo | null;
  capabilities?: HardwareCapabilities;
  onSensitivityChange: (s: 'low' | 'medium' | 'high' | 'very_high') => Promise<void>;
  onHapticsChange: (enabled: boolean) => Promise<void>;
  onHapticsIntensityChange: (i: 'low' | 'medium' | 'high') => Promise<void>;
  onGestureScreenshotChange: (enabled: boolean) => Promise<void>;
  onRepressChange: (enabled: boolean) => Promise<void>;
  onEdgeSlideChange: (enabled: boolean) => Promise<void>;
}

const SENSITIVITY_LEVELS: Array<'low' | 'medium' | 'high' | 'very_high'> = [
  'low',
  'medium',
  'high',
  'very_high',
];
const HAPTICS_LEVELS: Array<'low' | 'medium' | 'high'> = ['low', 'medium', 'high'];

export default function TouchpadSettings({
  touchpad,
  capabilities,
  onSensitivityChange,
  onHapticsChange,
  onHapticsIntensityChange,
  onGestureScreenshotChange,
  onRepressChange,
  onEdgeSlideChange,
}: Props) {
  if (!touchpad) {
    return (
      <div className="card">
        <div className="card-title">{t('touchpad.title')}</div>
        <div className="skeleton" style={{ height: 100 }} />
      </div>
    );
  }

  const hidUnavailable = capabilities != null && !capabilities.has_touchpad_hid;

  return (
    <div className="card">
      <div className="card-title">{t('touchpad.title')}</div>

      {/* ── Sensitivity ── */}
      <div style={{ marginBottom: 20 }}>
        <div className="stat-label" style={{ marginBottom: 10 }}>
          {t('touchpad.sensitivity')}
        </div>
        <div className="threshold-options">
          {SENSITIVITY_LEVELS.map((level) => (
            <button
              key={level}
              className={`threshold-btn ${touchpad.sensitivity === level ? 'active' : ''}`}
              onClick={() => void onSensitivityChange(level)}
            >
              {t(`touchpad.levels.${level}` as Parameters<typeof t>[0])}
            </button>
          ))}
        </div>
      </div>

      {/* ── Haptic feedback on/off ── */}
      <ToggleRow
        label={t('touchpad.haptics')}
        desc={hidUnavailable ? t('touchpad.hidNotAvailable') : t('touchpad.hapticsDesc')}
        checked={touchpad.haptics_enabled}
        onChange={(v) => void onHapticsChange(v)}
      />

      {/* ── Haptic intensity (only visible when haptics are on) ── */}
      {touchpad.haptics_enabled && (
        <div style={{ marginBottom: 20, marginTop: 4, paddingLeft: 4 }}>
          <div className="stat-label" style={{ marginBottom: 10 }}>
            {t('touchpad.hapticsIntensity')}
          </div>
          <div className="threshold-options">
            {HAPTICS_LEVELS.map((level) => (
              <button
                key={level}
                className={`threshold-btn ${touchpad.haptics_intensity === level ? 'active' : ''}`}
                onClick={() => void onHapticsIntensityChange(level)}
              >
                {t(`touchpad.levels.${level}` as Parameters<typeof t>[0])}
              </button>
            ))}
          </div>
        </div>
      )}

      {/* ── Gesture screenshot ── */}
      <ToggleRow
        label={t('touchpad.gestureScreenshot')}
        desc={t('touchpad.gestureScreenshotDesc')}
        checked={touchpad.gesture_screenshot}
        onChange={(v) => void onGestureScreenshotChange(v)}
      />

      {/* ── Trackpad repress ── */}
      <ToggleRow
        label={t('touchpad.repress')}
        desc={t('touchpad.repressDesc')}
        checked={touchpad.trackpad_repress}
        onChange={(v) => void onRepressChange(v)}
      />

      {/* ── Edge slide gestures ── */}
      <ToggleRow
        label={t('touchpad.edgeSlide')}
        desc={t('touchpad.edgeSlideDesc')}
        checked={touchpad.edge_slide}
        onChange={(v) => void onEdgeSlideChange(v)}
      />
    </div>
  );
}
