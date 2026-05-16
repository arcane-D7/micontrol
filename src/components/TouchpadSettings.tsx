import { t } from "../hooks/useI18n";
import type { TouchpadInfo, HardwareCapabilities } from "../hooks/useHardware";

interface Props {
  touchpad: TouchpadInfo | null;
  capabilities?: HardwareCapabilities;
  onSensitivityChange: (s: "low" | "medium" | "high") => Promise<void>;
  onHapticsChange: (enabled: boolean) => Promise<void>;
}

const SENSITIVITY_LEVELS: Array<"low" | "medium" | "high"> = ["low", "medium", "high"];

export default function TouchpadSettings({ touchpad, capabilities, onSensitivityChange, onHapticsChange }: Props) {
  if (!touchpad) {
    return (
      <div className="card">
        <div className="card-title">{t("touchpad.title")}</div>
        <div className="skeleton" style={{ height: 100 }} />
      </div>
    );
  }

  return (
    <div className="card">
      <div className="card-title">{t("touchpad.title")}</div>

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

      <div
        className="toggle"
        onClick={() => void onHapticsChange(!touchpad.haptics_enabled)}
      >
        <div className="toggle-info">
          <div className="toggle-name">{t("touchpad.haptics")}</div>
          <div className="toggle-desc">
            {capabilities != null && !capabilities.has_touchpad_hid
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
    </div>
  );
}
