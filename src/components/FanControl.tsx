import { useState } from "react";
import { t } from "../hooks/useI18n";
import type { FanInfo } from "../hooks/useHardware";

interface Props {
  fan: FanInfo | null;
  onModeChange: (mode: "auto" | "fixed" | "off", speedPercent?: number) => Promise<void>;
}

function FanSpeedVisual({ speedPct }: { speedPct: number }) {
  const segs = 12;
  const active = Math.round((speedPct / 100) * segs);
  return (
    <div className="fan-segments">
      {Array.from({ length: segs }, (_, i) => (
        <div
          key={i}
          className={`fan-seg ${i < active ? "active" : ""}`}
          style={{ height: `${40 + (i / segs) * 60}%` }}
        />
      ))}
    </div>
  );
}

export default function FanControl({ fan, onModeChange }: Props) {
  const [speed, setSpeed] = useState(fan?.speed_percent ?? 50);
  const [mode, setMode] = useState<"auto" | "fixed" | "off">(fan?.mode ?? "auto");

  const handleModeChange = async (m: "auto" | "fixed" | "off") => {
    setMode(m);
    await onModeChange(m, m === "fixed" ? speed : undefined);
  };

  if (!fan) {
    return (
      <div className="card">
        <div className="card-title">{t("fan.title")}</div>
        <div className="skeleton" style={{ height: 120 }} />
      </div>
    );
  }

  return (
    <div className="card">
      <div className="card-title">{t("fan.title")}</div>

      <div className="grid-3" style={{ marginBottom: 20 }}>
        {(["auto", "fixed", "off"] as const).map((m) => (
          <button
            key={m}
            className={`mode-btn ${mode === m ? "active" : ""}`}
            onClick={() => void handleModeChange(m)}
            style={{ padding: "12px 8px" }}
          >
            <span className="mode-btn-icon">
              {m === "auto" ? "🔄" : m === "fixed" ? "🔧" : "🔇"}
            </span>
            <span className="mode-btn-name">
              {t(`fan.modes.${m}` as Parameters<typeof t>[0])}
            </span>
          </button>
        ))}
      </div>

      {mode === "fixed" && (
        <div className="slider-row" style={{ marginBottom: 16 }}>
          <span className="slider-label">{t("fan.speed")}</span>
          <input
            type="range"
            min={20}
            max={100}
            step={5}
            value={speed}
            onChange={(e) => setSpeed(Number(e.target.value))}
            onMouseUp={() => void onModeChange("fixed", speed)}
          />
          <span className="slider-value">{speed}%</span>
        </div>
      )}

      <div className="grid-2">
        <div>
          <div className="stat-label" style={{ marginBottom: 8 }}>{t("fan.current")}</div>
          <FanSpeedVisual speedPct={fan.speed_percent} />
          <div style={{ marginTop: 4, fontSize: 13, fontWeight: 600 }}>
            {fan.speed_rpm} {t("fan.rpm")}
          </div>
        </div>
        <div>
          <div className="stat-label" style={{ marginBottom: 8 }}>{t("fan.temperature")}</div>
          <div className="card-value" style={{ fontSize: 22 }}>
            {fan.gpu_temp_celsius}°
          </div>
        </div>
      </div>

      <p style={{ fontSize: 11, color: "var(--color-warning)", marginTop: 12 }}>
        ⚠️ {t("fan.warning")}
      </p>
    </div>
  );
}
