import { t } from "../hooks/useI18n";
import type { PerformanceMode } from "../hooks/useHardware";

interface Props {
  current: PerformanceMode;
  onChange: (mode: PerformanceMode) => Promise<void>;
  disabled?: boolean;
}

const MODES: Array<{
  key: PerformanceMode;
  icon: string;
  labelKey: keyof typeof import("../i18n/en.json")["performance"]["modes"];
  descKey: keyof typeof import("../i18n/en.json")["performance"]["descriptions"];
}> = [
  { key: "silence", icon: "🔇", labelKey: "silence", descKey: "silence" },
  { key: "balance", icon: "⚖️", labelKey: "balance", descKey: "balance" },
  { key: "turbo", icon: "⚡", labelKey: "turbo", descKey: "turbo" },
  { key: "smart", icon: "🧠", labelKey: "smart", descKey: "smart" },
  { key: "long_battery", icon: "🍃", labelKey: "longBattery", descKey: "longBattery" },
  { key: "smart_acceleration", icon: "🚀", labelKey: "smartAcceleration", descKey: "smartAcceleration" },
];

export default function PerformanceModeSelector({ current, onChange, disabled }: Props) {
  return (
    <div>
      <div className="mode-grid">
        {MODES.map((m) => (
          <button
            key={m.key}
            className={`mode-btn ${current === m.key ? "active" : ""}`}
            onClick={() => void onChange(m.key)}
            disabled={disabled}
            title={t(`performance.descriptions.${m.descKey}` as Parameters<typeof t>[0])}
          >
            <span className="mode-btn-icon">{m.icon}</span>
            <span className="mode-btn-name">
              {t(`performance.modes.${m.labelKey}` as Parameters<typeof t>[0])}
            </span>
            <span className="mode-btn-desc">
              {t(`performance.descriptions.${m.descKey}` as Parameters<typeof t>[0])}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}
