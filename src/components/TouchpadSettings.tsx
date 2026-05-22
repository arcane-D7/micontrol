import { t } from "../hooks/useI18n";
import type { TouchpadInfo, HardwareCapabilities } from "../hooks/useHardware";

interface Props {
  touchpad: TouchpadInfo | null;
  capabilities?: HardwareCapabilities;
  onSensitivityChange: (s: "low" | "medium" | "high" | "very_high") => Promise<void>;
  onHapticsChange: (enabled: boolean) => Promise<void>;
  onHapticsIntensityChange: (i: "low" | "medium" | "high") => Promise<void>;
  onGestureScreenshotChange: (enabled: boolean) => Promise<void>;
  onRepressChange: (enabled: boolean) => Promise<void>;
  onEdgeSlideChange: (enabled: boolean) => Promise<void>;
}

const SENSITIVITY_LEVELS: Array<"low" | "medium" | "high" | "very_high"> = ["low", "medium", "high", "very_high"];
const HAPTICS_LEVELS: Array<"low" | "medium" | "high"> = ["low", "medium", "high"];

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
        <div className="card-title">{t("touchpad.title")}</div>
        <div className="skeleton" style={{ height: 100 }} />
      </div>
    );
  }

  const hidUnavailable = capabilities != null && !capabilities.has_touchpad_hid;

  return (
    <div className="card">
      <div className="card-title">{t("touchpad.title")}</div>

      {/* ── Sensitivity ── */}
      <div style={{ marginBottom: 20 }}>
        <div className="stat-label" style={{ marginBottom: 10 }}>{t("touchpad.sensitivity")}</div>
        <div className="threshold-options">
          {SENSITIVITY_LEVELS.map((level) => (
            <button
              key={level}
              className={`threshold-btn ${touchpad.sensitivity === level ? "active" : ""}`}
              onClick={() => void onSensitivityChange(level)}
            >
              {t(`touchpad.levels.${level}` as Parameters<typeof t>[0])}
            </button>
          ))}
        </div>
      </div>

      {/* ── Haptic feedback on/off ── */}
      <div
        className="toggle"
        onClick={() => void onHapticsChange(!touchpad.haptics_enabled)}
      >
        <div className="toggle-info">
          <div className="toggle-name">{t("touchpad.haptics")}</div>
          <div className="toggle-desc">
            {hidUnavailable
              ? t("touchpad.hidNotAvailable")
              : t("touchpad.hapticsDesc")}
          </div>
        </div>
        <label className="toggle-switch" onClick={(e) => e.stopPropagation()}>
          <input
            type="checkbox"
            checked={touchpad.haptics_enabled}
            onChange={(e) => void onHapticsChange(e.target.checked)}
          />
          <span className="toggle-track" />
          <span className="toggle-knob" />
        </label>
      </div>

      {/* ── Haptic intensity (only visible when haptics are on) ── */}
      {touchpad.haptics_enabled && (
        <div style={{ marginBottom: 20, marginTop: 4, paddingLeft: 4 }}>
          <div className="stat-label" style={{ marginBottom: 10 }}>{t("touchpad.hapticsIntensity")}</div>
          <div className="threshold-options">
            {HAPTICS_LEVELS.map((level) => (
              <button
                key={level}
                className={`threshold-btn ${touchpad.haptics_intensity === level ? "active" : ""}`}
                onClick={() => void onHapticsIntensityChange(level)}
              >
                {t(`touchpad.levels.${level}` as Parameters<typeof t>[0])}
              </button>
            ))}
          </div>
        </div>
      )}

      {/* ── Gesture screenshot ── */}
      <div
        className="toggle"
        onClick={() => void onGestureScreenshotChange(!touchpad.gesture_screenshot)}
      >
        <div className="toggle-info">
          <div className="toggle-name">{t("touchpad.gestureScreenshot")}</div>
          <div className="toggle-desc">{t("touchpad.gestureScreenshotDesc")}</div>
        </div>
        <label className="toggle-switch" onClick={(e) => e.stopPropagation()}>
          <input
            type="checkbox"
            checked={touchpad.gesture_screenshot}
            onChange={(e) => void onGestureScreenshotChange(e.target.checked)}
          />
          <span className="toggle-track" />
          <span className="toggle-knob" />
        </label>
      </div>

      {/* ── Trackpad repress ── */}
      <div
        className="toggle"
        onClick={() => void onRepressChange(!touchpad.trackpad_repress)}
      >
        <div className="toggle-info">
          <div className="toggle-name">{t("touchpad.repress")}</div>
          <div className="toggle-desc">{t("touchpad.repressDesc")}</div>
        </div>
        <label className="toggle-switch" onClick={(e) => e.stopPropagation()}>
          <input
            type="checkbox"
            checked={touchpad.trackpad_repress}
            onChange={(e) => void onRepressChange(e.target.checked)}
          />
          <span className="toggle-track" />
          <span className="toggle-knob" />
        </label>
      </div>

      {/* ── Edge slide gestures ── */}
      <div
        className="toggle"
        onClick={() => void onEdgeSlideChange(!touchpad.edge_slide)}
      >
        <div className="toggle-info">
          <div className="toggle-name">{t("touchpad.edgeSlide")}</div>
          <div className="toggle-desc">{t("touchpad.edgeSlideDesc")}</div>
        </div>
        <label className="toggle-switch" onClick={(e) => e.stopPropagation()}>
          <input
            type="checkbox"
            checked={touchpad.edge_slide}
            onChange={(e) => void onEdgeSlideChange(e.target.checked)}
          />
          <span className="toggle-track" />
          <span className="toggle-knob" />
        </label>
      </div>
    </div>
  );
}
