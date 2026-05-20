import { useState } from "react";
import { t } from "../hooks/useI18n";
import { useToast } from "../contexts/ToastContext";
import InfoModal, { InfoRow, InfoSection } from "./InfoModal";

interface Props {
  threshold: number;
  onThresholdChange: (threshold: number) => Promise<void>;
}

const LEVELS = [40, 50, 60, 70, 80] as const;

export default function ChargingThreshold({ threshold, onThresholdChange }: Props) {
  const [saving, setSaving] = useState(false);
  const [showInfo, setShowInfo] = useState(false);
  const { addToast } = useToast();

  // threshold === 100 means "no limit" (disabled)
  const enabled = threshold !== 100;
  // Remember the last valid level for re-enabling
  const [lastLevel, setLastLevel] = useState<number>(
    LEVELS.includes(threshold as (typeof LEVELS)[number]) ? threshold : 80
  );

  const handleChange = async (level: number) => {
    setSaving(true);
    try {
      await onThresholdChange(level);
      if (level !== 100) setLastLevel(level);
      addToast(t("charging.applied"), "success");
    } catch (e) {
      addToast(`${t("charging.error")}: ${String(e)}`, "error");
    } finally {
      setSaving(false);
    }
  };

  const handleToggle = async (checked: boolean) => {
    setSaving(true);
    try {
      const level = checked ? lastLevel : 100;
      await onThresholdChange(level);
      if (level !== 100) setLastLevel(level);
      addToast(
        checked ? t("charging.limitEnabled") : t("charging.limitDisabled"),
        "info",
      );
    } catch (e) {
      addToast(`${t("charging.error")}: ${String(e)}`, "error");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="card">
      {/* Card header with info button */}
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 12 }}>
        <div className="card-title" style={{ margin: 0 }}>{t("charging.title")}</div>
        <button
          onClick={() => setShowInfo(true)}
          title={t("charging.infoModal.title")}
          style={{
            background: "none", border: "none", cursor: "pointer",
            color: "var(--text-dim)", fontSize: 16, lineHeight: 1, padding: "2px 4px",
            borderRadius: "var(--r-xs)", transition: "color var(--t-fast)",
          }}
          onMouseEnter={(e) => (e.currentTarget.style.color = "var(--text)")}
          onMouseLeave={(e) => (e.currentTarget.style.color = "var(--text-dim)")}
        >
          ⓘ
        </button>
      </div>

      <p className="page-subtitle" style={{ marginBottom: 16 }}>
        {t("charging.subtitle")}
      </p>

      {/* Enable / disable toggle */}
      <div className="stat-row" style={{ marginBottom: 16 }}>
        <span className="stat-label">{t("charging.enableToggle")}</span>
        <label className="toggle-switch">
          <input
            type="checkbox"
            checked={enabled}
            disabled={saving}
            onChange={(e) => void handleToggle(e.target.checked)}
          />
          <span className="toggle-track" />
          <span className="toggle-knob" />
        </label>
      </div>

      {/* Threshold level buttons — only shown when enabled */}
      <div style={{ marginBottom: 8, opacity: enabled ? 1 : 0.4, pointerEvents: enabled ? "auto" : "none" }}>
        <div className="stat-label" style={{ marginBottom: 10 }}>{t("charging.threshold")}</div>
        <div className="threshold-options">
          {LEVELS.map((level) => (
            <button
              key={level}
              className={`threshold-btn ${threshold === level ? "active" : ""}`}
              onClick={() => void handleChange(level)}
              disabled={saving || !enabled}
            >
              {level}%
              {level === 80 && (
                <span className="threshold-badge">{t("charging.recommended")}</span>
              )}
            </button>
          ))}
        </div>
      </div>

      {!enabled && (
        <div style={{ marginTop: 4, fontSize: 12, color: "var(--text-muted)" }}>
          {t("charging.noLimit")}
        </div>
      )}

      {saving && (
        <div style={{ marginTop: 12, fontSize: 12, color: "var(--text-muted)" }}>
          {t("charging.applying")}
        </div>
      )}

      {/* Info modal */}
      <InfoModal open={showInfo} onClose={() => setShowInfo(false)} title={t("charging.infoModal.title")}>
        <InfoRow label={t("charging.infoModal.functionLabel")}>
          {t("charging.infoModal.functionDesc")}
        </InfoRow>
        <InfoRow label={t("charging.infoModal.requiresLabel")}>
          {t("charging.infoModal.requiresDesc")}
        </InfoRow>
        <InfoRow label={t("charging.infoModal.behaviorLabel")}>
          {t("charging.infoModal.behaviorDesc")}
        </InfoRow>
        <InfoSection>
          <div style={{
            background: "oklch(from var(--warning, #ff9800) l c h / 0.12)",
            border: "1px solid oklch(from var(--warning, #ff9800) l c h / 0.3)",
            borderRadius: "var(--r-sm)",
            padding: "10px 12px",
            fontSize: 12,
            color: "var(--text-muted)",
            lineHeight: 1.6,
          }}>
            <strong style={{ color: "var(--text)" }}>{t("charging.infoModal.warningLabel")}</strong>{" "}
            {t("charging.infoModal.warningDesc")}
          </div>
        </InfoSection>
      </InfoModal>
    </div>
  );
}
