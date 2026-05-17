import { useState, useEffect, useRef } from "react";
import { t } from "../hooks/useI18n";
import type { DisplayInfo, HardwareCapabilities, AiBrightnessConfig } from "../hooks/useHardware";
import { BRIGHTNESS_PRESETS, getActivePreset, getDefaultPresetConfig } from "../lib/brightnessPresets";
import { useToast } from "../contexts/ToastContext";

interface Props {
  display: DisplayInfo | null;
  capabilities?: HardwareCapabilities;
  onBrightnessChange: (level: number) => Promise<void>;
  onHdrChange: (enabled: boolean) => Promise<void>;
  onAiBrightnessChange: (enabled: boolean) => Promise<void>;
  onAiBrightnessConfigChange: (config: AiBrightnessConfig) => Promise<void>;
  onRefreshRateChange?: (hz: number) => Promise<void>;
  onAdaptiveRefreshRateChange?: (enabled: boolean) => Promise<void>;
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

export default function DisplaySettings({ display, capabilities, onBrightnessChange, onHdrChange, onAiBrightnessChange, onAiBrightnessConfigChange, onRefreshRateChange, onAdaptiveRefreshRateChange }: Props) {
  const { addToast } = useToast();
  const [brightness, setBrightness] = useState(display?.brightness ?? 80);
  const isDragging = useRef(false);

  const handleAsync = async (label: string, fn: () => Promise<void>) => {
    try {
      await fn();
    } catch (e) {
      addToast(`${label}: ${String(e)}`, "error");
    }
  };

  // Keep slider in sync with external changes (Fn keys, OS events).
  // We only update while the user is NOT actively dragging the slider.
  useEffect(() => {
    if (!isDragging.current && display?.brightness != null) {
      setBrightness(display.brightness);
    }
  }, [display?.brightness]);
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
          onMouseDown={() => { isDragging.current = true; }}
          onTouchStart={() => { isDragging.current = true; }}
          onMouseUp={() => { isDragging.current = false; void handleAsync("Brightness", () => onBrightnessChange(brightness)); }}
          onTouchEnd={() => { isDragging.current = false; void handleAsync("Brightness", () => onBrightnessChange(brightness)); }}
        />
        <span className="slider-value">{brightness}%</span>
      </div>

      <ToggleRow
        label={t("display.hdr")}
        desc={capabilities != null && !capabilities.has_igcl
          ? t("display.igclNotAvailable")
          : display.hdr_enabled ? t("display.hdrEnabled") : t("display.hdrDisabled")}
        checked={display.hdr_enabled}
        onChange={(v) => void handleAsync("HDR", () => onHdrChange(v))}
      />
      <ToggleRow
        label={t("display.aiAdaptiveBrightness")}
        desc={capabilities != null && !capabilities.has_igcl ? t("display.igclNotAvailable") : undefined}
        checked={display.ai_brightness}
        onChange={(v) => void handleAsync("Adaptive brightness", () => onAiBrightnessChange(v))}
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

              {/* Live ambient lux reading */}
              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 14, padding: "6px 8px", background: "var(--color-surface-sunken, rgba(0,0,0,0.15))", borderRadius: 6 }}>
                <span style={{ fontSize: 12, color: "var(--color-text-dim)" }}>☀ {t("display.luxSensor")}</span>
                <span style={{ fontSize: 12, fontWeight: 600 }}>
                  {display.ambient_lux != null
                    ? `${display.ambient_lux.toFixed(0)} lux`
                    : t("display.luxNotAvailable")}
                </span>
              </div>

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

              {/* How the settings work */}
              <div style={{ fontSize: 11, color: "var(--color-text-dim)", background: "var(--color-surface-sunken, rgba(0,0,0,0.15))", borderRadius: 6, padding: "8px 10px", marginBottom: 14, lineHeight: 1.6 }}>
                <div style={{ fontWeight: 600, marginBottom: 4 }}>ℹ {t("display.sensitivityHowItWorks")}</div>
                <div style={{ marginBottom: 2 }}>· {t("display.sensitivityHelpMin")}</div>
                <div style={{ marginBottom: 2 }}>· {t("display.sensitivityHelpLevel", { lux: String(Math.round(200000 / editCfg.sensitivity)) })}</div>
                <div>· {t("display.sensitivityHelpSmoothing")}</div>
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

      {/* Refresh rate selector */}
      {display.available_refresh_rates && display.available_refresh_rates.length > 1 ? (
        <div className="stat-row" style={{ marginTop: 8, alignItems: "center" }}>
          <span className="stat-label">{t("display.refreshRate")}</span>
          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
            {display.available_refresh_rates.map((hz) => {
              const isActive = hz === display.refresh_rate_hz;
              const isMax = hz === Math.max(...display.available_refresh_rates);
              return (
                <button
                  key={hz}
                  className={`chip-btn${isActive ? " active" : ""}`}
                  title={isMax && hz >= 90 ? `${hz} Hz — Windows 11 Dynamic Refresh Rate activates automatically at max Hz (switches 60\u2194${hz} Hz based on content)` : `${hz} Hz`}
                  onClick={() => { if (!isActive) void handleAsync("Refresh rate", () => onRefreshRateChange?.(hz) ?? Promise.resolve()); }}
                  style={{ minWidth: 52 }}
                >
                  {hz} Hz{isMax && display.dynamic_refresh_rate_capable && " ★"}
                </button>
              );
            })}
          </div>
        </div>
      ) : (
        <div className="stat-row" style={{ marginTop: 8 }}>
          <span className="stat-label">{t("display.refreshRate")}</span>
          <span className="stat-value">{display.refresh_rate_hz} {t("display.hz")}</span>
        </div>
      )}
      {display.dynamic_refresh_rate_capable && (
        <div style={{ fontSize: 11, color: "var(--color-text-dim)", marginTop: 4, lineHeight: 1.4 }}>
          ★ Dynamic Refresh Rate active — Windows 11 automatically switches to lower Hz to save power when high frame rate isn’t needed.
        </div>
      )}
      {/* Intel PSR2 DRRS toggle */}
      {onAdaptiveRefreshRateChange && (
        <ToggleRow
          label="Intel Adaptive Refresh Rate (PSR2 DRRS)"
          desc={
            display.adaptive_refresh_rate
              ? `Active — the Intel driver is switching the panel between ~48 Hz at idle and ${Math.max(...(display.available_refresh_rates ?? [120]))} Hz when content is moving. ` +
                "Windows always reports the configured Hz (e.g. 120 Hz) — this is expected: PSR2 operates below the OS level."
              : `Disabled — panel locked at ${display.refresh_rate_hz} Hz. ` +
                "Note: This is separate from Windows Dynamic Refresh Rate (DRR). PSR2 DRRS switches at the panel hardware level, invisible to Windows."
          }
          checked={display.adaptive_refresh_rate}
          onChange={(v) => {
            void (async () => {
              try {
                await onAdaptiveRefreshRateChange(v);
                addToast(
                  v
                    ? "PSR2 DRRS enabled. Reboot required. Note: Windows will still report the configured Hz — PSR2 operates below the OS."
                    : "PSR2 DRRS disabled. Reboot required.",
                  "info"
                );
              } catch (e) {
                addToast(`Adaptive refresh rate: ${String(e)}`, "error");
              }
            })();
          }}
        />
      )}
      {onAdaptiveRefreshRateChange && (
        <div style={{ fontSize: 11, color: "var(--color-text-dim)", marginTop: 4, lineHeight: 1.5, paddingLeft: 2 }}>
          💡 PSR2 DRRS ≠ Windows "Dynamic Refresh Rate". Windows DRR requires a VRR-capable display.
          PSR2 DRRS is an Intel driver feature that switches the panel refresh rate internally —
          Windows always reports the max configured Hz (currently {display.refresh_rate_hz} Hz), even when PSR2 is active.
        </div>
      )}    </div>
  );
}
