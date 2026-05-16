import { useState } from "react";
import { t } from "../hooks/useI18n";

interface Props {
  threshold: number;
  onThresholdChange: (threshold: number) => Promise<void>;
}

const LEVELS = [40, 50, 60, 70, 80] as const;

export default function ChargingThreshold({ threshold, onThresholdChange }: Props) {
  const [saving, setSaving] = useState(false);

  const handleChange = async (level: number) => {
    setSaving(true);
    try {
      await onThresholdChange(level);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="card">
      <div className="card-title">{t("charging.title")}</div>
      <p className="page-subtitle" style={{ marginBottom: 16 }}>
        {t("charging.subtitle")}
      </p>

      <div style={{ marginBottom: 8 }}>
        <div className="stat-label" style={{ marginBottom: 10 }}>{t("charging.threshold")}</div>
        <div className="threshold-options">
          {LEVELS.map((level) => (
            <button
              key={level}
              className={`threshold-btn ${threshold === level ? "active" : ""}`}
              onClick={() => void handleChange(level)}
              disabled={saving}
            >
              {level}%
              {level === 80 && (
                <span className="threshold-badge">{t("charging.recommended")}</span>
              )}
            </button>
          ))}
        </div>
      </div>

      {saving && (
        <div style={{ marginTop: 12, fontSize: 12, color: "var(--color-text-muted)" }}>
          {t("charging.applying")}
        </div>
      )}
    </div>
  );
}
