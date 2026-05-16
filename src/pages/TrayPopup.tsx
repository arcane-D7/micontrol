import { invoke } from "@tauri-apps/api/core";
import { useState, useEffect } from "react";
import { t } from "../hooks/useI18n";
import type { useHardware, PerformanceMode } from "../hooks/useHardware";
import { BRIGHTNESS_PRESETS, getActivePreset } from "../lib/brightnessPresets";

type Hardware = ReturnType<typeof useHardware>;

interface Props {
  hardware: Hardware;
}

const QUICK_MODES: Array<{ key: PerformanceMode; label: string }> = [
  { key: "silence", label: "🔇" },
  { key: "balance", label: "⚖️" },
  { key: "turbo", label: "⚡" },
  { key: "smart", label: "🧠" },
];

export default function TrayPopup({ hardware }: Props) {
  const {
    battery,
    performanceMode,
    setPerformanceMode,
    loading,
    display,
    fan,
    setBrightness,
    setAiBrightness,
    setAiBrightnessConfig,
    setFanMode,
  } = hardware;

  const [showDisplay, setShowDisplay] = useState(false);
  const [showFan, setShowFan] = useState(false);
  const [localBrightness, setLocalBrightness] = useState(display?.brightness ?? 80);

  // Sync local brightness when hardware updates
  useEffect(() => {
    if (display?.brightness !== undefined) setLocalBrightness(display.brightness);
  }, [display?.brightness]);

  const openMainWindow = async () => {
    await invoke("open_main_window");
  };

  return (
    <div className="tray-popup">
      <div className="tray-header">
        <span className="tray-title">MiControl</span>
        <button
          className="btn btn-secondary"
          style={{ padding: "4px 10px", fontSize: 12 }}
          onClick={() => void openMainWindow()}
        >
          {t("tray.openApp")}
        </button>
      </div>

      <div className="tray-body">
        {/* Performance mode */}
        <div className="tray-section">
          <div className="tray-section-label">{t("tray.performance")}</div>
          <div className="tray-mode-row">
            {QUICK_MODES.map((m) => (
              <button
                key={m.key}
                className={`tray-mode-btn ${performanceMode === m.key ? "active" : ""}`}
                onClick={() => void setPerformanceMode(m.key)}
                disabled={loading}
                title={t(`performance.modes.${m.key === "silence" ? "silence" : m.key === "balance" ? "balance" : m.key === "turbo" ? "turbo" : "smart"}` as Parameters<typeof t>[0])}
              >
                {m.label}
              </button>
            ))}
          </div>
        </div>

        {/* Battery */}
        {battery && (
          <div className="tray-section">
            <div className="tray-section-label">{t("tray.battery")}</div>
            <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
              <div style={{ flex: 1 }}>
                <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 6 }}>
                  <span style={{ fontSize: 13, fontWeight: 600 }}>{battery.level}%</span>
                  <span className={`badge ${battery.is_charging ? "success" : "warning"}`}>
                    {battery.is_charging ? t("battery.charging") : t("battery.discharging")}
                  </span>
                </div>
                <div className="progress-bar">
                  <div
                    className={`progress-fill ${battery.level > 30 ? "battery" : battery.level > 15 ? "battery warning" : "battery critical"}`}
                    style={{ width: `${battery.level}%` }}
                  />
                </div>
              </div>
            </div>
          </div>
        )}

        {/* Quick actions */}
        <div className="tray-section">
          <div className="tray-section-label">{t("tray.quickActions")}</div>
          <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>

            {/* ── Display Settings ─────────────────────────── */}
            <button
              className="btn btn-secondary"
              style={{ width: "100%", justifyContent: "space-between" }}
              onClick={() => setShowDisplay((v) => !v)}
            >
              <span>🖥️ {t("tray.displaySettings")}</span>
              <span style={{ fontSize: 10, opacity: 0.6 }}>{showDisplay ? "▲" : "▼"}</span>
            </button>

            {showDisplay && (
              <div style={{
                padding: "10px 12px",
                background: "var(--surface-2, rgba(255,255,255,0.04))",
                borderRadius: "var(--r-sm)",
                display: "flex",
                flexDirection: "column",
                gap: 10,
              }}>
                {/* Brightness slider */}
                <div>
                  <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 4, fontSize: 11, color: "var(--text-muted)" }}>
                    <span>{t("display.brightness")}</span>
                    <span style={{ fontFamily: "var(--font-mono)", color: "var(--text)" }}>{localBrightness}%</span>
                  </div>
                  <input
                    type="range"
                    min={10}
                    max={100}
                    value={localBrightness}
                    onChange={(e) => setLocalBrightness(Number(e.target.value))}
                    onMouseUp={() => void setBrightness(localBrightness)}
                    onTouchEnd={() => void setBrightness(localBrightness)}
                    style={{ width: "100%" }}
                  />
                </div>

                {/* Auto-brightness toggle */}
                <div
                  style={{ display: "flex", justifyContent: "space-between", alignItems: "center", cursor: "pointer" }}
                  onClick={() => void setAiBrightness(!(display?.ai_brightness ?? false))}
                >
                  <span style={{ fontSize: 12 }}>{t("tray.autoBrightness")}</span>
                  <label className="toggle-switch" onClick={(e) => e.stopPropagation()}>
                    <input
                      type="checkbox"
                      checked={display?.ai_brightness ?? false}
                      onChange={(e) => void setAiBrightness(e.target.checked)}
                    />
                    <span className="toggle-track" />
                    <span className="toggle-knob" />
                  </label>
                </div>

                {/* Brightness presets — only when auto-brightness is on */}
                {display?.ai_brightness && display.ai_brightness_config && (
                  <div>
                    <div style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 6 }}>{t("tray.brightnessPresets")}</div>
                    <div className="tray-mode-row">
                      {BRIGHTNESS_PRESETS.map((p) => {
                        const cfg = display.ai_brightness_config;
                        const isActive = getActivePreset(cfg) === p.key;
                        return (
                          <button
                            key={p.key}
                            className={`tray-mode-btn${isActive ? " active" : ""}`}
                            title={p.hint}
                            onClick={() => void setAiBrightnessConfig({
                              enabled: true,
                              ...p.config,
                            })}
                          >
                            {p.icon} {p.label}
                          </button>
                        );
                      })}
                    </div>
                  </div>
                )}
              </div>
            )}

            {/* ── Fan Control ──────────────────────────────── */}
            <button
              className="btn btn-secondary"
              style={{ width: "100%", justifyContent: "space-between" }}
              onClick={() => setShowFan((v) => !v)}
            >
              <span>💨 {t("tray.fanControl")}</span>
              <span style={{ fontSize: 10, opacity: 0.6 }}>{showFan ? "▲" : "▼"}</span>
            </button>

            {showFan && (
              <div style={{
                padding: "10px 12px",
                background: "var(--surface-2, rgba(255,255,255,0.04))",
                borderRadius: "var(--r-sm)",
              }}>
                <div style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 6 }}>{t("tray.fanMode")}</div>
                <div className="tray-mode-row">
                  {(["auto", "fixed", "off"] as const).map((mode) => {
                    const shortLabel = mode === "auto" ? "Auto" : mode === "fixed" ? "Fixed" : "Off";
                    return (
                      <button
                        key={mode}
                        className={`tray-mode-btn${fan?.mode === mode ? " active" : ""}`}
                        onClick={() => void setFanMode(mode)}
                        disabled={loading}
                        title={t(`fan.modes.${mode}` as Parameters<typeof t>[0])}
                      >
                        {mode === "auto" ? "🔄" : mode === "fixed" ? "🔧" : "🔇"} {shortLabel}
                      </button>
                    );
                  })}
                </div>
                {fan && (
                  <div style={{ marginTop: 8, fontSize: 11, color: "var(--text-muted)", display: "flex", gap: 12 }}>
                    <span>{fan.speed_rpm} RPM</span>
                    <span>{fan.gpu_temp_celsius}°C GPU</span>
                  </div>
                )}
              </div>
            )}

          </div>
        </div>
      </div>
    </div>
  );
}
