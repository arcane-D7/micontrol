import { t } from "../hooks/useI18n";
import type { PerformanceMode } from "../hooks/useHardware";

interface Props {
  current: PerformanceMode;
  onChange: (mode: PerformanceMode) => Promise<void>;
  disabled?: boolean;
  /** Whether the user has configured an AI API key (gates Smart modes) */
  aiApiKeySet?: boolean;
  /** Navigate to Settings tab to configure API key */
  onOpenSettings?: () => void;
}

const MODES: Array<{
  key: PerformanceMode;
  icon: string;
  labelKey: keyof typeof import("../i18n/en.json")["performance"]["modes"];
  descKey: keyof typeof import("../i18n/en.json")["performance"]["descriptions"];
  detailKey: keyof typeof import("../i18n/en.json")["performance"]["techDetails"]["modes"];
  requiresAi?: true;
}> = [
  { key: "silence",            icon: "🔇", labelKey: "silence",            descKey: "silence",            detailKey: "silence" },
  { key: "balance",            icon: "⚖️", labelKey: "balance",            descKey: "balance",            detailKey: "balance" },
  { key: "turbo",              icon: "⚡", labelKey: "turbo",              descKey: "turbo",              detailKey: "turbo" },
  { key: "smart",              icon: "🧠", labelKey: "smart",              descKey: "smart",              detailKey: "smart",              requiresAi: true },
  { key: "long_battery",       icon: "🍃", labelKey: "longBattery",        descKey: "longBattery",        detailKey: "longBattery" },
  { key: "smart_acceleration", icon: "🚀", labelKey: "smartAcceleration",  descKey: "smartAcceleration",  detailKey: "smartAcceleration",  requiresAi: true },
];

/** Hardware constants per mode (not translated — numbers / proper nouns) */
const MODE_SPECS: Record<PerformanceMode, { tdp: string; fan: string; windowsOverlay: string; accentColor: string }> = {
  silence:            { tdp: "~7 W PL1 / ~10 W PL2",         fan: "Off or <2 000 RPM",       windowsOverlay: "Power Saver",     accentColor: "var(--info)" },
  balance:            { tdp: "~15 W PL1 / ~25 W PL2",        fan: "Adaptive 2 000–3 500 RPM", windowsOverlay: "Balanced",        accentColor: "var(--success)" },
  turbo:              { tdp: "~25 W PL1 / ~45 W PL2",        fan: "Aggressive 4 000–5 500 RPM",windowsOverlay: "Best Performance", accentColor: "var(--warning)" },
  smart:              { tdp: "7–25 W (AI-adaptive, gradual)", fan: "Variable — follows load",  windowsOverlay: "Balanced",        accentColor: "var(--accent)" },
  long_battery:       { tdp: "~6 W PL1 / ~8 W PL2",         fan: "Off or very slow",         windowsOverlay: "Power Saver",     accentColor: "var(--success)" },
  decepticon:         { tdp: "~35 W PL1 / ~55 W+ PL2",      fan: "Max 5 500+ RPM",           windowsOverlay: "Best Performance", accentColor: "var(--error)" },
  smart_acceleration: { tdp: "7–25 W base + burst ~40 W",    fan: "Reactive — spikes on demand",windowsOverlay: "Balanced",       accentColor: "var(--accent)" },
};


export default function PerformanceModeSelector({
  current, onChange, disabled, aiApiKeySet = false, onOpenSettings,
}: Props) {
  const spec = MODE_SPECS[current];
  const showSmartDiff = current === "smart" || current === "smart_acceleration";

  return (
    <div>
      <div className="mode-grid">
        {MODES.map((m) => {
          const aiLocked = !!m.requiresAi && !aiApiKeySet;
          return (
            <button
              key={m.key}
              className={`mode-btn ${current === m.key ? "active" : ""} ${aiLocked ? "ai-locked" : ""}`}
              onClick={() => {
                if (aiLocked) { onOpenSettings?.(); return; }
                void onChange(m.key);
              }}
              disabled={disabled && !aiLocked}
              title={
                aiLocked
                  ? t("performance.techDetails.aiLockedMsg")
                  : t(`performance.descriptions.${m.descKey}` as Parameters<typeof t>[0])
              }
            >
              <span className="mode-btn-icon">{m.icon}</span>
              <span className="mode-btn-name">
                {t(`performance.modes.${m.labelKey}` as Parameters<typeof t>[0])}
                {aiLocked && (
                  <span
                    style={{ marginLeft: 4, fontSize: 10, color: "var(--text-dim)", verticalAlign: "middle" }}
                    title={t("performance.techDetails.aiLockedMsg")}
                  >🔒</span>
                )}
              </span>
              <span className="mode-btn-desc">
                {aiLocked
                  ? t("performance.techDetails.requiresApiKey")
                  : t(`performance.descriptions.${m.descKey}` as Parameters<typeof t>[0])
                }
              </span>
            </button>
          );
        })}
      </div>

      {/* Technical details for the active mode */}
      {spec && (
        <div
          style={{
            marginTop: 16,
            padding: "16px 18px",
            background: "var(--surface-2)",
            borderRadius: "var(--r-sm)",
            borderLeft: `3px solid ${spec.accentColor}`,
          }}
        >
          <div
            style={{
              fontSize: 11, fontWeight: 600, color: "var(--text-dim)",
              textTransform: "uppercase", letterSpacing: "0.08em", marginBottom: 12,
            }}
          >
            {t("performance.techDetails.title")}
          </div>

          {/* Spec row: TDP / Fan / Windows overlay */}
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: "10px 16px", marginBottom: 12 }}>
            <div>
              <div style={{ fontSize: 10, color: "var(--text-muted)", marginBottom: 2 }}>
                {t("performance.techDetails.tdp")}
              </div>
              <div style={{ fontSize: 13, fontWeight: 600, color: "var(--text)", fontFamily: "var(--font-mono)" }}>
                {spec.tdp}
              </div>
            </div>
            <div>
              <div style={{ fontSize: 10, color: "var(--text-muted)", marginBottom: 2 }}>
                {t("performance.techDetails.fanBehavior")}
              </div>
              <div style={{ fontSize: 13, fontWeight: 600, color: "var(--text)" }}>{spec.fan}</div>
            </div>
            <div>
              <div style={{ fontSize: 10, color: "var(--text-muted)", marginBottom: 2 }}>
                {t("performance.techDetails.windowsOverlay")}
              </div>
              <div style={{ fontSize: 13, fontWeight: 600, color: spec.accentColor }}>
                {spec.windowsOverlay}
              </div>
            </div>
          </div>

          {/* Detailed description — now pulled from i18n */}
          <div style={{ fontSize: 12, color: "var(--text-muted)", lineHeight: 1.6 }}>
            {t(`performance.techDetails.modes.${MODES.find(m => m.key === current)?.detailKey ?? "balance"}` as Parameters<typeof t>[0])}
          </div>

          {/* Windows overlay note */}
          <div style={{ marginTop: 10, fontSize: 11, color: "var(--text-dim)", display: "flex", alignItems: "flex-start", gap: 6 }}>
            <span style={{ flexShrink: 0 }}>ℹ️</span>
            <span>{t("performance.techDetails.overlayNote")}</span>
          </div>

          {/* Smart vs Smart Acceleration comparison */}
          {showSmartDiff && (
            <div
              style={{
                marginTop: 12, padding: "10px 14px", background: "var(--surface-3, var(--surface))",
                borderRadius: "var(--r-sm)", borderLeft: "3px solid var(--accent)",
                fontSize: 12, color: "var(--text-muted)", lineHeight: 1.55,
              }}
            >
              <span style={{ fontWeight: 600, color: "var(--accent)" }}>Smart vs Smart Acceleration — </span>
              {t("performance.techDetails.smartDiff")}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
