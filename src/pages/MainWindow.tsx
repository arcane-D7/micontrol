import { useState, useEffect, useRef, useCallback } from "react";
import type { ThemeMode } from "../App";
import { t } from "../hooks/useI18n";
import type { useHardware } from "../hooks/useHardware";
import TrayPopup from "./TrayPopup";
import { useSettings } from "../hooks/useSettings";
import PerformanceModeSelector from "../components/PerformanceModeSelector";
import PerformanceMonitor from "../components/PerformanceMonitor";
import BatteryInfoCard from "../components/BatteryInfo";
import ChargingThreshold from "../components/ChargingThreshold";
import DisplaySettings from "../components/DisplaySettings";
import FanControl from "../components/FanControl";
import TouchpadSettings from "../components/TouchpadSettings";
import StartupManager from "../components/StartupManager";
import SystemInfoCard from "../components/SystemInfoCard";
import UpdateManager from "../components/UpdateManager";
import HardwareDiscovery from "../components/HardwareDiscovery";
import AiAdvisor from "../components/AiAdvisor";
import SettingsPage from "../components/SettingsPage";
import { MiControlIcon } from "../components/MiControlIcon";

type Hardware = ReturnType<typeof useHardware>;

interface Props {
  hardware: Hardware;
  activeTab: string;
  onTabChange: (tab: string) => void;
  themeMode: ThemeMode;
  toggleTheme: () => void;
}

const NAV_ITEMS = [
  { id: "overview", icon: "📊", label: "nav.overview" },
  { id: "performance", icon: "⚡", label: "nav.performance" },
  { id: "battery", icon: "🔋", label: "nav.battery" },
  { id: "display", icon: "🖥️", label: "nav.display" },
  { id: "fan", icon: "💨", label: "nav.fan" },
  { id: "touchpad", icon: "🖱️", label: "nav.touchpad" },
  { id: "startup", icon: "🚀", label: "nav.startup" },
  { id: "updates", icon: "🔄", label: "nav.updates" },
  { id: "keyboard", icon: "⌨️", label: "nav.keyboard" },
  { id: "setup", icon: "🔍", label: "nav.setup" },
  { id: "settings", icon: "⚙️", label: "nav.settings" },
  { id: "about", icon: "ℹ️", label: "nav.about" },
] as const;

function PageHeader({ title, subtitle }: { title: string; subtitle?: string }) {
  return (
    <div className="page-header">
      <div className="page-title">{title}</div>
      {subtitle && <div className="page-subtitle">{subtitle}</div>}
    </div>
  );
}

type AiSettings = ReturnType<typeof useSettings>;

function OverviewTab({ hw, ai, onOpenSettings }: { hw: Hardware; ai: AiSettings; onOpenSettings: () => void }) {
  return (
    <>
      <PageHeader title={t("overview.title")} />
      <div className="grid-2">
        <SystemInfoCard info={hw.systemInfo} getProcessList={hw.getProcessList} />
        <BatteryInfoCard battery={hw.battery} />
      </div>
      <div className="card">
        <div className="card-title">{t("nav.performance")}</div>
        <PerformanceModeSelector
          current={hw.performanceMode}
          onChange={hw.setPerformanceMode}
          disabled={hw.loading}
        />
      </div>
      <AiAdvisor hw={hw} ai={ai} onOpenSettings={onOpenSettings} />
    </>
  );
}

function PerformanceTab({ hw, ai, onOpenSettings }: { hw: Hardware; ai: AiSettings; onOpenSettings: () => void }) {
  const aiApiKeySet = !!ai.settings.openai_api_key;
  const isAiMode = hw.performanceMode === "smart" || hw.performanceMode === "smart_acceleration";

  // ── Background logger: write a snapshot every 30 s when an AI mode is active ──
  // Uses refs to always read the latest hw values without re-registering the interval.
  const fanRef = useRef(hw.fan);
  const sysRef = useRef(hw.systemInfo);
  const modeRef = useRef(hw.performanceMode);
  fanRef.current = hw.fan;
  sysRef.current = hw.systemInfo;
  modeRef.current = hw.performanceMode;

  useEffect(() => {
    if (!isAiMode || !aiApiKeySet) return;
    const writeEntry = () => {
      const f = fanRef.current;
      const s = sysRef.current;
      if (!f && !s) return;
      const entry = {
        ts: new Date().toISOString().replace("T", " ").slice(0, 19),
        mode: modeRef.current,
        cpu_temp: f?.cpu_temp_celsius ?? 0,
        gpu_temp: f?.gpu_temp_celsius ?? 0,
        tdp_watts: f?.tdp_watts ?? null,
        cpu_pct: s?.cpu_usage ?? 0,
        gpu_pct: s?.gpu_usage ?? 0,
        note: null,
      };
      void hw.writeAiPerfLog(entry);
    };
    writeEntry(); // immediate first entry
    const id = setInterval(writeEntry, 30_000);
    return () => clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isAiMode, aiApiKeySet]);

  // ── Log viewer state ──────────────────────────────────────────────────────
  const [showLogs, setShowLogs] = useState(false);
  const [logEntries, setLogEntries] = useState<import("../hooks/useHardware").AiPerfLogEntry[]>([]);
  const [loadingLogs, setLoadingLogs] = useState(false);

  const loadLogs = useCallback(async () => {
    setLoadingLogs(true);
    try {
      const entries = await hw.readAiPerfLogs(50);
      setLogEntries(entries);
    } catch { /* non-fatal */ }
    finally { setLoadingLogs(false); }
  }, [hw]);

  useEffect(() => {
    if (showLogs) void loadLogs();
  }, [showLogs, loadLogs]);

  return (
    <>
      <PageHeader title={t("performance.title")} subtitle={t("performance.subtitle")} />
      <PerformanceMonitor
        fan={hw.fan}
        systemInfo={hw.systemInfo}
        currentMode={hw.performanceMode}
        lastResult={hw.lastPerfResult}
      />
      <div className="card">
        <PerformanceModeSelector
          current={hw.performanceMode}
          onChange={hw.setPerformanceMode}
          disabled={hw.loading}
          aiApiKeySet={aiApiKeySet}
          onOpenSettings={onOpenSettings}
        />
      </div>

      {/* AI log viewer — shown only when AI modes have been used */}
      <div className="card">
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: showLogs ? 14 : 0 }}>
          <div>
            <div className="card-title" style={{ marginBottom: 2 }}>AI Mode Logs</div>
            <div style={{ fontSize: 12, color: "var(--text-muted)" }}>
              {isAiMode && aiApiKeySet
                ? "Logging active — snapshot every 30 s"
                : aiApiKeySet
                  ? "Configure a Smart mode to start logging"
                  : "Configure AI API key in Settings to enable logging"}
            </div>
          </div>
          <div style={{ display: "flex", gap: 8 }}>
            <button className="btn-secondary" style={{ fontSize: 12 }} onClick={() => { setShowLogs(v => !v); }}>
              {showLogs ? "Hide" : "View Logs"}
            </button>
            <button className="btn-secondary" style={{ fontSize: 12 }} onClick={() => void hw.openAiLogsDir()} title="Open log folder in Explorer">
              📂
            </button>
          </div>
        </div>

        {showLogs && (
          <div>
            {loadingLogs ? (
              <div style={{ textAlign: "center", color: "var(--text-dim)", padding: "16px 0", fontSize: 13 }}>Loading…</div>
            ) : logEntries.length === 0 ? (
              <div style={{ textAlign: "center", color: "var(--text-dim)", padding: "16px 0", fontSize: 13 }}>
                No log entries yet. Activate Smart or Smart Acceleration mode to start recording.
              </div>
            ) : (
              <div style={{ overflowX: "auto" }}>
                <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 12 }}>
                  <thead>
                    <tr style={{ color: "var(--text-muted)", borderBottom: "1px solid var(--border)" }}>
                      <th style={{ textAlign: "left", padding: "4px 8px", fontWeight: 500 }}>Time</th>
                      <th style={{ textAlign: "left", padding: "4px 8px", fontWeight: 500 }}>Mode</th>
                      <th style={{ textAlign: "right", padding: "4px 8px", fontWeight: 500 }}>CPU°C</th>
                      <th style={{ textAlign: "right", padding: "4px 8px", fontWeight: 500 }}>GPU°C</th>
                      <th style={{ textAlign: "right", padding: "4px 8px", fontWeight: 500 }}>TDP W</th>
                      <th style={{ textAlign: "right", padding: "4px 8px", fontWeight: 500 }}>CPU%</th>
                      <th style={{ textAlign: "right", padding: "4px 8px", fontWeight: 500 }}>GPU%</th>
                    </tr>
                  </thead>
                  <tbody>
                    {logEntries.map((e, i) => (
                      <tr key={i} style={{ borderBottom: "1px solid var(--border-faint, var(--border))" }}>
                        <td style={{ padding: "4px 8px", fontFamily: "var(--font-mono)", color: "var(--text-dim)" }}>{e.ts.slice(11)}</td>
                        <td style={{ padding: "4px 8px", color: "var(--accent)" }}>{e.mode}</td>
                        <td style={{ padding: "4px 8px", textAlign: "right", fontFamily: "var(--font-mono)" }}>{e.cpu_temp.toFixed(0)}</td>
                        <td style={{ padding: "4px 8px", textAlign: "right", fontFamily: "var(--font-mono)" }}>{e.gpu_temp.toFixed(0)}</td>
                        <td style={{ padding: "4px 8px", textAlign: "right", fontFamily: "var(--font-mono)" }}>{e.tdp_watts != null ? e.tdp_watts.toFixed(1) : "—"}</td>
                        <td style={{ padding: "4px 8px", textAlign: "right", fontFamily: "var(--font-mono)" }}>{e.cpu_pct.toFixed(0)}</td>
                        <td style={{ padding: "4px 8px", textAlign: "right", fontFamily: "var(--font-mono)" }}>{e.gpu_pct.toFixed(0)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                <div style={{ marginTop: 10, display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                  <span style={{ fontSize: 11, color: "var(--text-dim)" }}>Showing {logEntries.length} most recent entries</span>
                  <button className="btn-secondary" style={{ fontSize: 11 }} onClick={() => void loadLogs()}>↻ Refresh</button>
                </div>
              </div>
            )}
          </div>
        )}
      </div>
    </>
  );
}

function BatteryTab({ hw }: { hw: Hardware }) {
  return (
    <>
      <PageHeader title={t("battery.title")} />
      <BatteryInfoCard battery={hw.battery} />
      <ChargingThreshold
        threshold={hw.chargingThreshold}
        onThresholdChange={hw.setChargingThreshold}
      />
    </>
  );
}

function DisplayTab({ hw }: { hw: Hardware }) {
  return (
    <>
      <PageHeader title={t("display.title")} />
      <DisplaySettings
        display={hw.display}
        capabilities={hw.hardwareProfile?.capabilities}
        onBrightnessChange={hw.setBrightness}
        onHdrChange={hw.setHdr}
        onAiBrightnessChange={hw.setAiBrightness}
        onAiBrightnessConfigChange={hw.setAiBrightnessConfig}
        onRefreshRateChange={hw.setRefreshRate}
        onAdaptiveRefreshRateChange={hw.setAdaptiveRefreshRate}
      />
    </>
  );
}

function FanTab({ hw }: { hw: Hardware }) {
  return (
    <>
      <PageHeader title={t("fan.title")} />
      <FanControl fan={hw.fan} onModeChange={hw.setFanMode} />
    </>
  );
}

function TouchpadTab({ hw }: { hw: Hardware }) {
  return (
    <>
      <PageHeader title={t("touchpad.title")} />
      <TouchpadSettings
        touchpad={hw.touchpad}
        capabilities={hw.hardwareProfile?.capabilities}
        onSensitivityChange={hw.setTouchpadSensitivity}
        onHapticsChange={hw.setTouchpadHaptics}
        onHapticsIntensityChange={hw.setTouchpadHapticsIntensity}
        onGestureScreenshotChange={hw.setTouchpadGestureScreenshot}
        onRepressChange={hw.setTouchpadRepress}
        onEdgeSlideChange={hw.setTouchpadEdgeSlide}
      />
    </>
  );
}

function StartupTab() {
  return (
    <>
      <PageHeader title={t("startup.title")} />
      <StartupManager autostart={false} />
    </>
  );
}

// ── Keyboard Remapper Tab ────────────────────────────────────────────────────
// Option A: 3 fixed Xiaomi laptop keys (AI key, PCManager key, Copilot key).
// TODO (Option B — Full Keyboard Remapping Module):
//   - Replace 3 fixed rows with a dynamic list from get_hotkey_config()
//   - Add "Press to detect key" button that calls a detect_key_mode() command
//   - Add more action types: SetPerformanceMode, ToggleAiBrightness, MediaControl, Script
//   - Add modifier key support (Ctrl+VK, Alt+VK, Win+VK)
//   - Add conflict detection warnings for system-reserved keys

type HotkeyAction =
  | { type: "none" }
  | { type: "open_url"; url: string }
  | { type: "launch_app"; path: string; args: string[] };

interface KeyBinding {
  enabled: boolean;
  action: HotkeyAction;
  label?: string;
}

interface HotkeyMap {
  ai_key: KeyBinding;
  xiaomi_key: KeyBinding;
  copilot_key: KeyBinding;
}

function KeyBindingRow({
  label,
  description,
  binding,
  onChange,
}: {
  label: string;
  description: string;
  binding: KeyBinding;
  onChange: (b: KeyBinding) => void;
}) {
  const actionType = binding.action.type;
  const urlValue = binding.action.type === "open_url" ? binding.action.url : "";
  const appPath = binding.action.type === "launch_app" ? binding.action.path : "";

  function setActionType(type: string) {
    if (type === "none") onChange({ ...binding, action: { type: "none" } });
    else if (type === "open_url") onChange({ ...binding, action: { type: "open_url", url: urlValue } });
    else if (type === "launch_app") onChange({ ...binding, action: { type: "launch_app", path: appPath, args: [] } });
  }

  return (
    <div className="card" style={{ marginBottom: 12 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 10 }}>
        <label className="toggle-switch" style={{ flexShrink: 0 }}>
          <input
            type="checkbox"
            checked={binding.enabled}
            onChange={(e) => onChange({ ...binding, enabled: e.target.checked })}
          />
          <span className="toggle-slider" />
        </label>
        <div>
          <div className="card-title" style={{ margin: 0, fontSize: 14 }}>{label}</div>
          <div style={{ fontSize: 12, opacity: 0.6, marginTop: 2 }}>{description}</div>
        </div>
      </div>

      <div style={{ display: "flex", gap: 8, flexWrap: "wrap", alignItems: "center" }}>
        <select
          className="select-input"
          value={actionType}
          disabled={!binding.enabled}
          onChange={(e) => setActionType(e.target.value)}
          style={{ minWidth: 140 }}
        >
          <option value="none">{t("keyboard.actionNone")}</option>
          <option value="open_url">{t("keyboard.actionOpenUrl")}</option>
          <option value="launch_app">{t("keyboard.actionLaunchApp")}</option>
        </select>

        {actionType === "open_url" && (
          <input
            className="text-input"
            type="text"
            placeholder={t("keyboard.urlPlaceholder")}
            value={urlValue}
            disabled={!binding.enabled}
            onChange={(e) =>
              onChange({ ...binding, action: { type: "open_url", url: e.target.value } })
            }
            style={{ flex: 1, minWidth: 200 }}
          />
        )}
        {actionType === "launch_app" && (
          <input
            className="text-input"
            type="text"
            placeholder={t("keyboard.appPlaceholder")}
            value={appPath}
            disabled={!binding.enabled}
            onChange={(e) =>
              onChange({
                ...binding,
                action: { type: "launch_app", path: e.target.value, args: [] },
              })
            }
            style={{ flex: 1, minWidth: 200 }}
          />
        )}
      </div>
    </div>
  );
}

function KeyboardTab() {
  const [config, setConfig] = useState<HotkeyMap | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    import("@tauri-apps/api/core").then(({ invoke }) =>
      invoke<HotkeyMap>("get_hotkey_config")
        .then(setConfig)
        .catch((e) => console.error("get_hotkey_config", e))
    );
  }, []);

  async function save() {
    if (!config) return;
    setSaving(true);
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("set_hotkey_config", { config });
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (e) {
      console.error("set_hotkey_config", e);
    } finally {
      setSaving(false);
    }
  }

  if (!config) {
    return (
      <>
        <PageHeader title={t("keyboard.title")} subtitle={t("keyboard.subtitle")} />
        <div className="card" style={{ opacity: 0.6 }}>{t("keyboard.loading")}</div>
      </>
    );
  }

  return (
    <>
      <PageHeader title={t("keyboard.title")} subtitle={t("keyboard.subtitle")} />

      <KeyBindingRow
        label={t("keyboard.aiKey")}
        description={t("keyboard.aiKeyDesc")}
        binding={config.ai_key}
        onChange={(b) => setConfig({ ...config, ai_key: b })}
      />
      <KeyBindingRow
        label={t("keyboard.xiaomiKey")}
        description={t("keyboard.xiaomiKeyDesc")}
        binding={config.xiaomi_key}
        onChange={(b) => setConfig({ ...config, xiaomi_key: b })}
      />
      <KeyBindingRow
        label={t("keyboard.copilotKey")}
        description={t("keyboard.copilotKeyDesc")}
        binding={config.copilot_key}
        onChange={(b) => setConfig({ ...config, copilot_key: b })}
      />

      <div style={{ display: "flex", gap: 8, marginTop: 4 }}>
        <button
          className="btn-primary"
          onClick={save}
          disabled={saving}
          style={{ minWidth: 100 }}
        >
          {saving ? t("keyboard.saving") : saved ? t("keyboard.saved") : t("keyboard.save")}
        </button>
      </div>
    </>
  );
}

function UpdatesTab({ hw }: { hw: Hardware }) {
  return (
    <>
      <PageHeader title={t("updates.title")} subtitle={t("updates.subtitle")} />
      <UpdateManager
        updateStatus={hw.updateStatus}
        loadingUpdate={hw.loadingUpdate}
        onRefreshUpdate={hw.refreshUpdateStatus}
      />
    </>
  );
}

function SetupTab({ hw }: { hw: Hardware }) {
  return (
    <>
      <PageHeader title={t("discovery.title")} subtitle={t("discovery.subtitle")} />
      <HardwareDiscovery
        profile={hw.hardwareProfile}
        loading={hw.loadingDiscovery}
        onRescan={hw.runHardwareDiscovery}
        onInstallDriver={hw.installDriver}
      />
    </>
  );
}

function SettingsTab({ ai }: { ai: AiSettings }) {
  return (
    <>
      <PageHeader title={t("settings.title")} subtitle={t("settings.subtitle")} />
      <SettingsPage
        settings={ai.settings}
        onSave={ai.saveSettings}
        onTest={ai.testConnection}
      />
    </>
  );
}

function AboutTab() {
  return (
    <>
      <PageHeader title={t("about.title")} />
      <div className="card">
        <div className="grid-2">
          <div>
            <div className="stat-row">
              <span className="stat-label">{t("about.appName")}</span>
              <span className="stat-value">MiControl</span>
            </div>
            <div className="stat-row">
              <span className="stat-label">{t("about.version")}</span>
              <span className="stat-value">0.1.0</span>
            </div>
            <div className="stat-row">
              <span className="stat-label">{t("about.device")}</span>
              <span className="stat-value">Xiaomi Laptop Pro</span>
            </div>
          </div>
        </div>
        <p style={{ marginTop: 16, fontSize: 12, color: "var(--color-text-muted)" }}>
          {t("about.description")}
        </p>
      </div>
    </>
  );
}

function ThemeIcon({ mode }: { mode: ThemeMode }) {
  if (mode === "light") return (
    <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
      <circle cx="8" cy="8" r="2.8" />
      <line x1="8" y1="1.5" x2="8" y2="3" />
      <line x1="8" y1="13" x2="8" y2="14.5" />
      <line x1="1.5" y1="8" x2="3" y2="8" />
      <line x1="13" y1="8" x2="14.5" y2="8" />
      <line x1="3.5" y1="3.5" x2="4.5" y2="4.5" />
      <line x1="11.5" y1="11.5" x2="12.5" y2="12.5" />
      <line x1="12.5" y1="3.5" x2="11.5" y2="4.5" />
      <line x1="4.5" y1="11.5" x2="3.5" y2="12.5" />
    </svg>
  );
  if (mode === "dark") return (
    <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
      <path d="M7.5 2a6 6 0 1 0 6.5 8.5A5 5 0 0 1 7.5 2z" />
    </svg>
  );
  // auto
  return (
    <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
      <circle cx="8" cy="8" r="5.5" />
      <path d="M8 2.5 A5.5 5.5 0 0 1 8 13.5" fill="currentColor" stroke="none" />
    </svg>
  );
}

const THEME_LABELS: Record<ThemeMode, string> = { auto: "Auto", light: "Light", dark: "Dark" };

export default function MainWindow({ hardware, activeTab, onTabChange, themeMode, toggleTheme }: Props) {
  const aiSettings = useSettings();
  const [showTrayPreview, setShowTrayPreview] = useState(false);

  function renderTab() {
    switch (activeTab) {
      case "overview":   return <OverviewTab hw={hardware} ai={aiSettings} onOpenSettings={() => onTabChange("settings")} />;
      case "performance": return <PerformanceTab hw={hardware} ai={aiSettings} onOpenSettings={() => onTabChange("settings")} />;
      case "battery":    return <BatteryTab hw={hardware} />;
      case "display":    return <DisplayTab hw={hardware} />;
      case "fan":        return <FanTab hw={hardware} />;
      case "touchpad":   return <TouchpadTab hw={hardware} />;
      case "startup":    return <StartupTab />;
      case "updates":    return <UpdatesTab hw={hardware} />;
      case "keyboard":   return <KeyboardTab />;
      case "setup":      return <SetupTab hw={hardware} />;
      case "settings":   return <SettingsTab ai={aiSettings} />;
      case "about":      return <AboutTab />;
      default:           return <OverviewTab hw={hardware} ai={aiSettings} onOpenSettings={() => onTabChange("settings")} />;
    }
  }

  return (
    <div className="app-layout">
      <nav className="sidebar">
        <div className="sidebar-logo">
          <MiControlIcon size={22} />
          MiControl
        </div>
        {NAV_ITEMS.map((item) => (
          <button
            key={item.id}
            className={`sidebar-item ${activeTab === item.id ? "active" : ""}`}
            onClick={() => onTabChange(item.id)}
          >
            <span className="sidebar-icon">{item.icon}</span>
            {t(item.label as Parameters<typeof t>[0])}
          </button>
        ))}

        <div className="sidebar-footer">
          {hardware.error && (
            <div style={{ padding: "4px 8px", fontSize: 11, color: "var(--error)", wordBreak: "break-word" }}>
              ⚠️ {hardware.error}
            </div>
          )}
          {hardware.loading && (
            <div style={{ padding: "4px 8px", fontSize: 11, color: "var(--text-dim)" }}>
              {t("common.loading")}
            </div>
          )}
          <button className="theme-toggle" onClick={toggleTheme} title={`Theme: ${themeMode}`}>
            <ThemeIcon mode={themeMode} />
            <span>{THEME_LABELS[themeMode]}</span>
          </button>
          {import.meta.env.DEV && <button
            className="theme-toggle"
            onClick={() => setShowTrayPreview(true)}
            title="Preview tray popup"
          >
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
              <rect x="1" y="10" width="14" height="5" rx="1" />
              <rect x="1" y="1" width="14" height="7" rx="1" />
              <line x1="4" y1="12.5" x2="4" y2="12.5" strokeWidth="2" />
              <line x1="7" y1="12.5" x2="7" y2="12.5" strokeWidth="2" />
            </svg>
            <span>Tray</span>
          </button>}
        </div>
      </nav>

      <main className="content-area">
        <div className="tab-content" key={activeTab}>
          {renderTab()}
        </div>

        {/* Watermark */}
        <div style={{
          position: "absolute", bottom: 10, right: 14,
          fontSize: 10, color: "var(--color-text-muted, oklch(50% 0 0))",
          opacity: 0.55, userSelect: "none", pointerEvents: "none",
          display: "flex", alignItems: "center", gap: 4,
          fontFamily: "var(--font-mono, monospace)",
        }}>
          <span>By: Marcos Freitas</span>
          <a
            href="https://github.com/Freitas-MA"
            target="_blank"
            rel="noopener noreferrer"
            title="github.com/Freitas-MA"
            style={{
              color: "inherit", textDecoration: "none", pointerEvents: "auto",
              display: "flex", alignItems: "center",
            }}
          >
            <svg width="11" height="11" viewBox="0 0 16 16" fill="currentColor" aria-label="GitHub">
              <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38
                0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52
                -.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07
                -1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12
                0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82
                2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95
                .29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8
                c0-4.42-3.58-8-8-8z" />
            </svg>
          </a>
        </div>
      </main>

      {/* Tray popup preview overlay */}
      {showTrayPreview && (
        <div
          style={{
            position: "fixed", inset: 0, zIndex: 999,
            background: "oklch(0% 0 0 / 0.35)",
            backdropFilter: "blur(2px)",
          }}
          onClick={() => setShowTrayPreview(false)}
        >
          <div
            style={{
              position: "absolute", bottom: 48, right: 16,
              display: "flex", flexDirection: "column", alignItems: "flex-end", gap: 6,
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <div style={{ fontSize: 10, color: "oklch(90% 0 0 / 0.6)", fontFamily: "var(--font)", letterSpacing: "0.06em", textTransform: "uppercase" }}>
              Mock Preview · click outside to close
            </div>
            <TrayPopup hardware={hardware} />
            <div
              style={{
                width: 40, height: 40, borderRadius: "50%",
                background: "var(--accent)", display: "flex", alignItems: "center", justifyContent: "center",
                cursor: "pointer", boxShadow: "0 2px 12px var(--accent-glow)",
                fontSize: 18, alignSelf: "flex-end",
              }}
              title="MiControl tray icon"
            >
              🖥️
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
