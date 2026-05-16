import { useState } from "react";
import { t } from "../hooks/useI18n";
import type { DisplayInfo, HardwareCapabilities, AiBrightnessConfig } from "../hooks/useHardware";
import { BRIGHTNESS_PRESETS, getActivePreset, getDefaultPresetConfig } from "../lib/brightnessPresets";

interface Props {
  display: DisplayInfo | null;
  capabilities?: HardwareCapabilities;
  onBrightnessChange: (level: number) => Promise<void>;
  onHdrChange: (enabled: boolean) => Promise<void>;
  onAiBrightnessChange: (enabled: boolean) => Promise<void>;
  onAiBrightnessConfigChange: (config: AiBrightnessConfig) => Promise<void>;
}

function ToggleRow({
  label,
  desc,
  checked,
  onChange,
}: {
  label: string;
  desc?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className="toggle" onClick={() => onChange(!checked)}>
      <div className="toggle-info">
        <div className="toggle-name">{label}</div>
        {desc && <div className="toggle-desc">{desc}</div>}
      </div>
      <label className="toggle-switch" onClick={(e) => e.stopPropagation()}>
        <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
        <span className="toggle-track" />
        <span className="toggle-knob" />
      </label>
    </div>
  );
}

export default function DisplaySettings({ display, capabilities, onBrightnessChange, onHdrChange, onAiBrightnessChange, onAiBrightnessConfigChange }: Props) {
  const [brightness, setBrightness] = useState(display?.brightness ?? 80);
  const [sensitivityOpen, setSensitivityOpen] = useState(false);
  const cfg = display?.ai_brightness_config;
  const [localCfg, setLocalCfg] = useState<AiBrightnessConfig | null>(null);
  const editCfg = localCfg ?? cfg;
  const isDirty = localCfg !== null;

  if (!display) {
    return (
      <div className="card">
        <div className="card-title">{t("display.title")}</div>
        <div className="skeleton" style={{ height: 80 }} />
      </div>
    );
  }

  return (
    <div className="card">
      <div className="card-title">{t("display.title")}</div>

      <div className="slider-row" style={{ marginBottom: 20 }}>
        <span className="slider-label">{t("display.brightness")}</span>
        <input
          type="range"
          min={10}
          max={100}
          value={brightness}
          onChange={(e) => setBrightness(Number(e.target.value))}
          onMouseUp={() => void onBrightnessChange(brightness)}
          onTouchEnd={() => void onBrightnessChange(brightness)}
        />
        <span className="slider-value">{brightness}%</span>
      </div>

      <ToggleRow
        label={t("display.hdr")}
        desc={capabilities != null && !capabilities.has_igcl
          ? t("display.igclNotAvailable")
          : display.hdr_enabled ? t("display.hdrEnabled") : t("display.hdrDisabled")}
        checked={display.hdr_enabled}
        onChange={(v) => void onHdrChange(v)}
      />
      <ToggleRow
        label={t("display.aiAdaptiveBrightness")}
        desc={capabilities != null && !capabilities.has_igcl ? t("display.igclNotAvailable") : undefined}
        checked={display.ai_brightness}
        onChange={(v) => void onAiBrightnessChange(v)}
      />

      {display.ai_brightness && editCfg && (
        <div style={{ marginTop: 2, marginBottom: 8 }}>
          <button
            className="link-btn"
            style={{ fontSize: 12, color: "var(--color-text-dim)" }}
            onClick={() => setSensitivityOpen((o) => !o)}
          >
            {sensitivityOpen ? "▲" : "▼"} {t("display.sensitivityConfigure")}
          </button>

          {sensitivityOpen && (
            <div style={{ marginTop: 10, padding: "12px 14px", background: "var(--color-surface-raised, rgba(255,255,255,0.04))", borderRadius: 8 }}>

              {/* Preset chips */}
              <div style={{ marginBottom: 14 }}>
                <div className="tray-section-label" style={{ marginBottom: 6 }}>{t("display.presets")}</div>
                <div style={{ display: "flex", gap: 6 }}>
                  {BRIGHTNESS_PRESETS.map((p) => {
                    const isActive = getActivePreset(editCfg) === p.key;
                    return (
                      <button
                        key={p.key}
                        className={`chip-btn${isActive ? " active" : ""}`}
                        title={p.hint}
                        onClick={() => setLocalCfg({ ...editCfg, ...p.config })}
                      >
                        {p.icon} {p.label}
                      </button>
                    );
                  })}
                  {getActivePreset(editCfg) === null && (
                    <span className="chip-btn active" style={{ pointerEvents: "none", opacity: 0.7 }}>
                      ✏️ {t("display.presetCustom")}
                    </span>
                  )}
                </div>
              </div>
              <div className="slider-row" style={{ marginBottom: 12 }}>
                <span className="slider-label" style={{ fontSize: 12 }}>{t("display.sensitivityMin")}</span>
                <input type="range" min={5} max={80} step={5}
                  value={editCfg.min_brightness}
                  onChange={(e) => setLocalCfg({ ...editCfg, min_brightness: Number(e.target.value) })} />
                <span className="slider-value" style={{ fontSize: 12 }}>{editCfg.min_brightness}%</span>
              </div>

              {/* Max brightness */}
              <div className="slider-row" style={{ marginBottom: 12 }}>
                <span className="slider-label" style={{ fontSize: 12 }}>{t("display.sensitivityMax")}</span>
                <input type="range" min={20} max={100} step={5}
                  value={editCfg.max_brightness}
                  onChange={(e) => setLocalCfg({ ...editCfg, max_brightness: Number(e.target.value) })} />
                <span className="slider-value" style={{ fontSize: 12 }}>{editCfg.max_brightness}%</span>
              </div>

              {/* Sensitivity */}
              <div className="slider-row" style={{ marginBottom: 4 }}>
                <span className="slider-label" style={{ fontSize: 12 }}>{t("display.sensitivityLevel")}</span>
                <input type="range" min={10} max={200} step={10}
                  value={editCfg.sensitivity}
                  onChange={(e) => setLocalCfg({ ...editCfg, sensitivity: Number(e.target.value) })} />
                <span className="slider-value" style={{ fontSize: 12 }}>{editCfg.sensitivity}</span>
              </div>
              <div style={{ display: "flex", justifyContent: "space-between", fontSize: 10, color: "var(--color-text-dim)", marginBottom: 12 }}>
                <span>{t("display.sensitivitySubtle")}</span>
                <span>{t("display.sensitivityAggressive")}</span>
              </div>

              {/* Smoothing */}
              <div className="slider-row" style={{ marginBottom: 4 }}>
                <span className="slider-label" style={{ fontSize: 12 }}>{t("display.sensitivitySmoothing")}</span>
                <input type="range" min={0} max={90} step={10}
                  value={editCfg.smoothing}
                  onChange={(e) => setLocalCfg({ ...editCfg, smoothing: Number(e.target.value) })} />
                <span className="slider-value" style={{ fontSize: 12 }}>{editCfg.smoothing}</span>
              </div>
              <div style={{ display: "flex", justifyContent: "space-between", fontSize: 10, color: "var(--color-text-dim)", marginBottom: 14 }}>
                <span>{t("display.sensitivityInstant")}</span>
                <span>{t("display.sensitivityGradual")}</span>
              </div>

              <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
                <button
                  className="btn-primary"
                  disabled={!isDirty}
                  onClick={() => {
                    if (!localCfg) return;
                    void onAiBrightnessConfigChange(localCfg).then(() => setLocalCfg(null));
                  }}
                >
                  {t("display.sensitivitySave")}
                </button>
                <button
                  className="btn-ghost btn-sm"
                  title={t("display.resetDefaultDesc")}
                  onClick={() => {
                    const defaults = getDefaultPresetConfig();
                    const newCfg: AiBrightnessConfig = { enabled: display.ai_brightness, ...defaults };
                    void onAiBrightnessConfigChange(newCfg).then(() => setLocalCfg(null));
                  }}
                >
                  ↺ {t("display.resetDefault")}
                </button>
              </div>
            </div>
          )}
        </div>
      )}

      <div className="stat-row" style={{ marginTop: 8 }}>
        <span className="stat-label">{t("display.refreshRate")}</span>
        <span className="stat-value">{display.refresh_rate_hz} {t("display.hz")}</span>
      </div>
    </div>
  );
}
