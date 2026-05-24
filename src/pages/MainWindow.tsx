import { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ThemeMode } from "../App";
import { t } from "../hooks/useI18n";
import type { useHardware, EramMap, IotRegionName } from "../hooks/useHardware";
import TrayPopup from "./TrayPopup";
import { useSettings } from "../hooks/useSettings";
import { useToast } from "../contexts/ToastContext";
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
import AiAnalysis from "../components/AiAnalysis";
import SettingsPage from "../components/SettingsPage";
import { MiControlIcon } from "../components/MiControlIcon";
import { useAnalysisLogger } from "../hooks/useAnalysisLogger";

type Hardware = ReturnType<typeof useHardware>;

interface PerfDebugInfo {
  hq_wmi_instance: string | null;
  hq_wmi_works: boolean;
  hq_wmi_test_ret: string;
  vhf_device_path: string | null;
  registry_mode: string;
  overlay_mode: string;
}

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
  { id: "ai_analysis", icon: "🤖", label: "nav.aiAnalysis" },
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

  // ── Auto-switch performance mode on AC ↔ DC transition ───────────────────
  // Reads settings via refs so we never need to re-register the effect.
  const autoSwitchRef = useRef(ai.settings.auto_switch_perf);
  const acModeRef     = useRef(ai.settings.perf_mode_ac);
  const dcModeRef     = useRef(ai.settings.perf_mode_dc);
  autoSwitchRef.current = ai.settings.auto_switch_perf;
  acModeRef.current     = ai.settings.perf_mode_ac;
  dcModeRef.current     = ai.settings.perf_mode_dc;

  const prevPluggedRef = useRef<boolean | null>(null);
  useEffect(() => {
    const plugged = hw.battery?.is_plugged ?? null;
    if (plugged === null) return;
    if (prevPluggedRef.current === null) {
      prevPluggedRef.current = plugged; // initialise — don't apply on mount
      return;
    }
    if (prevPluggedRef.current === plugged) return; // no change
    prevPluggedRef.current = plugged;
    if (!autoSwitchRef.current) return;
    const targetMode = plugged ? acModeRef.current : dcModeRef.current;
    if (targetMode) void hw.setPerformanceMode(targetMode);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hw.battery?.is_plugged]);

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

  // ── Perf channel debug ─────────────────────────────────────────────────────
  const [debugInfo, setDebugInfo] = useState<PerfDebugInfo | null>(null);
  const [loadingDebug, setLoadingDebug] = useState(false);

  const runPerfDebug = useCallback(async () => {
    setLoadingDebug(true);
    try {
      const info = await invoke<PerfDebugInfo>("get_perf_debug");
      setDebugInfo(info);
    } catch (e) {
      setDebugInfo(null);
      console.error("get_perf_debug failed", e);
    } finally {
      setLoadingDebug(false);
    }
  }, []);

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

      {/* Power Profiles — per-source preferred modes */}
      <div className="card">
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 4 }}>
          <div>
            <div className="card-title" style={{ marginBottom: 2 }}>{t("performance.powerProfiles.title")}</div>
            <div style={{ fontSize: 12, color: "var(--text-muted)" }}>{t("performance.powerProfiles.subtitle")}</div>
          </div>
          <label className="toggle-switch">
            <input
              type="checkbox"
              checked={ai.settings.auto_switch_perf}
              onChange={(e) => ai.updateKey("auto_switch_perf", e.target.checked)}
            />
            <span className="toggle-track" />
            <span className="toggle-knob" />
          </label>
        </div>
        {ai.settings.auto_switch_perf && (
          <div style={{ marginTop: 14, display: "flex", flexDirection: "column", gap: 10 }}>
            {/* Plugged in */}
            <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8 }}>
              <span style={{ fontSize: 13 }}>🔌 {t("performance.powerProfiles.pluggedIn")}</span>
              <select
                className="select-input"
                style={{ minWidth: 160 }}
                value={ai.settings.perf_mode_ac ?? ""}
                onChange={(e) => ai.updateKey("perf_mode_ac", (e.target.value || null) as import("../hooks/useHardware").PerformanceMode | null)}
              >
                <option value="">{t("performance.powerProfiles.manual")}</option>
                <option value="silence">{t("performance.modes.silence")}</option>
                <option value="long_battery">{t("performance.modes.longBattery")}</option>
                <option value="balance">{t("performance.modes.balance")}</option>
                <option value="turbo">{t("performance.modes.turbo")}</option>
                <option value="decepticon">{t("performance.modes.decepticon")}</option>
                <option value="overdrive">{t("performance.modes.overdrive")}</option>
                <option value="overdrive_high">{t("performance.modes.overdriveHigh")}</option>
                <option value="overdrive_max">{t("performance.modes.overdriveMax")}</option>
                <option value="smart_adaptive">{t("performance.modes.smartAdaptive")}</option>
                <option value="smart">{t("performance.modes.smart")}</option>
                <option value="smart_acceleration">{t("performance.modes.smartAcceleration")}</option>
              </select>
            </div>
            {/* On battery */}
            <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8 }}>
              <span style={{ fontSize: 13 }}>🔋 {t("performance.powerProfiles.onBattery")}</span>
              <select
                className="select-input"
                style={{ minWidth: 160 }}
                value={ai.settings.perf_mode_dc ?? ""}
                onChange={(e) => ai.updateKey("perf_mode_dc", (e.target.value || null) as import("../hooks/useHardware").PerformanceMode | null)}
              >
                <option value="">{t("performance.powerProfiles.manual")}</option>
                <option value="silence">{t("performance.modes.silence")}</option>
                <option value="long_battery">{t("performance.modes.longBattery")}</option>
                <option value="balance">{t("performance.modes.balance")}</option>
                <option value="turbo">{t("performance.modes.turbo")}</option>
                <option value="decepticon">{t("performance.modes.decepticon")}</option>
                <option value="overdrive">{t("performance.modes.overdrive")}</option>
                <option value="overdrive_high">{t("performance.modes.overdriveHigh")}</option>
                <option value="overdrive_max">{t("performance.modes.overdriveMax")}</option>
                <option value="smart_adaptive">{t("performance.modes.smartAdaptive")}</option>
                <option value="smart">{t("performance.modes.smart")}</option>
                <option value="smart_acceleration">{t("performance.modes.smartAcceleration")}</option>
              </select>
            </div>
            {/* Status hint */}
            <div style={{ fontSize: 11, color: "var(--text-muted)", paddingTop: 4 }}>
              {hw.battery?.is_plugged
                ? `⚡ ${t("performance.powerProfiles.currentlyPluggedIn")}`
                : `🔋 ${t("performance.powerProfiles.currentlyOnBattery")}`}
            </div>
          </div>
        )}
      </div>

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

      {/* Performance channel diagnostics */}
      <div className="card">
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
          <div>
            <div className="card-title" style={{ marginBottom: 2 }}>{t("performance.channels.title")}</div>
            <div style={{ fontSize: 12, color: "var(--text-muted)" }}>
              {t("performance.channels.subtitle")}
            </div>
          </div>
          <button
            className="btn-secondary"
            style={{ fontSize: 12 }}
            onClick={() => void runPerfDebug()}
            disabled={loadingDebug}
          >
            {loadingDebug ? t("performance.channels.checking") : t("performance.channels.checkNow")}
          </button>
        </div>

        {debugInfo && (
          <div style={{ marginTop: 14, display: "flex", flexDirection: "column", gap: 8, fontSize: 13 }}>
            {/* HQ WMI */}
            <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
              <span style={{ color: "var(--text-muted)" }}>{t("performance.channels.hqWmi")}</span>
              <span style={{
                color: debugInfo.hq_wmi_works ? "var(--success, #4caf50)" : "var(--error, #f44336)",
                fontWeight: 600
              }}>
                {debugInfo.hq_wmi_works
                  ? `✓ ${t("performance.channels.functional")}`
                  : `✗ ${t("performance.channels.unavailable")}`}
              </span>
            </div>
            {debugInfo.hq_wmi_instance && (
              <div style={{ display: "flex", justifyContent: "space-between", gap: 8, fontSize: 11 }}>
                <span style={{ color: "var(--text-muted)" }}>{t("performance.channels.instance")}</span>
                <code style={{ color: "var(--text-dim)", fontFamily: "var(--font-mono)", maxWidth: 260, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {debugInfo.hq_wmi_instance}
                </code>
              </div>
            )}
            {debugInfo.hq_wmi_test_ret && (
              <div style={{ display: "flex", justifyContent: "space-between", gap: 8, fontSize: 11 }}>
                <span style={{ color: "var(--text-muted)" }}>{t("performance.channels.response")}</span>
                <code style={{ color: "var(--text-dim)", fontFamily: "var(--font-mono)" }}>
                  {debugInfo.hq_wmi_test_ret}
                </code>
              </div>
            )}
            {/* VHF */}
            <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
              <span style={{ color: "var(--text-muted)" }}>{t("performance.channels.vhf")}</span>
              <span style={{
                color: debugInfo.vhf_device_path ? "var(--success, #4caf50)" : "var(--text-dim)",
                fontWeight: 600
              }}>
                {debugInfo.vhf_device_path
                  ? `✓ ${t("performance.channels.found")}`
                  : `— ${t("performance.channels.notFound")}`}
              </span>
            </div>
            {debugInfo.vhf_device_path && (
              <div style={{ display: "flex", justifyContent: "space-between", gap: 8, fontSize: 11 }}>
                <span style={{ color: "var(--text-muted)" }}>Path</span>
                <code style={{ color: "var(--text-dim)", fontFamily: "var(--font-mono)", maxWidth: 260, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {debugInfo.vhf_device_path}
                </code>
              </div>
            )}
            {/* Registry + Overlay */}
            <div style={{ display: "flex", justifyContent: "space-between", gap: 8 }}>
              <span style={{ color: "var(--text-muted)" }}>{t("performance.channels.registry")}</span>
              <code style={{ color: "var(--text-dim)", fontFamily: "var(--font-mono)" }}>{debugInfo.registry_mode}</code>
            </div>
            <div style={{ display: "flex", justifyContent: "space-between", gap: 8 }}>
              <span style={{ color: "var(--text-muted)" }}>{t("performance.channels.overlay")}</span>
              <code style={{ color: "var(--text-dim)", fontFamily: "var(--font-mono)" }}>{debugInfo.overlay_mode}</code>
            </div>
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
//   - Add modifier key support (Ctrl+VK, Alt+VK, Win+VK)
//   - Add conflict detection warnings for system-reserved keys

type HotkeyAction =
  | { type: "none" }
  | { type: "focus_micontrol" }
  | { type: "open_main_window" }
  | { type: "open_url"; url: string }
  | { type: "launch_app"; path: string; args: string[] }
  | { type: "remap_to_key"; vk: number; extended: boolean }
  | { type: "set_performance_mode"; mode: string }
  | { type: "toggle_ai_brightness" }
  | { type: "media_control"; action: string }
  | { type: "script"; interpreter: string; path: string; args: string[] };

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
  const [detecting, setDetecting] = useState(false);
  const [detectedVk, setDetectedVk] = useState<string>("");

  const actionType = binding.action.type;
  const urlValue = binding.action.type === "open_url" ? binding.action.url : "";
  const appPath = binding.action.type === "launch_app" ? binding.action.path : "";
  const remapVk = binding.action.type === "remap_to_key" ? binding.action.vk : 0xA3;
  const perfMode = binding.action.type === "set_performance_mode" ? binding.action.mode : "balance";
  const mediaAction = binding.action.type === "media_control" ? binding.action.action : "volume_up";
  const scriptInterp = binding.action.type === "script" ? (binding.action.interpreter || "") : "";
  const scriptPath = binding.action.type === "script" ? binding.action.path : "";

  // Known remap targets (VK, extended, display label)
  const REMAP_TARGETS: { vk: number; extended: boolean; label: string }[] = [
    { vk: 0xA3, extended: true,  label: t("keyboard.remapRCtrl") },
    { vk: 0xA5, extended: true,  label: t("keyboard.remapRAlt") },
    { vk: 0xA1, extended: false, label: t("keyboard.remapRShift") },
    { vk: 0x2E, extended: true,  label: t("keyboard.remapDelete") },
    { vk: 0x2F, extended: false, label: t("keyboard.remapHelp") },
  ];

  const PERFORMANCE_MODES = [
    { value: "silence", label: t("performance.modes.silence") },
    { value: "long_battery", label: t("performance.modes.longBattery") },
    { value: "balance", label: t("performance.modes.balance") },
    { value: "turbo", label: t("performance.modes.turbo") },
    { value: "decepticon", label: t("performance.modes.decepticon") },
    { value: "overdrive", label: t("performance.modes.overdrive") },
    { value: "overdrive_high", label: t("performance.modes.overdriveHigh") },
    { value: "overdrive_max", label: t("performance.modes.overdriveMax") },
    { value: "smart_adaptive", label: t("performance.modes.smartAdaptive") },
    { value: "smart", label: t("performance.modes.smart") },
    { value: "smart_acceleration", label: t("performance.modes.smartAcceleration") },
  ];

  function setActionType(type: string) {
    const autoEnabled = type !== "none";
    if (type === "none") onChange({ ...binding, enabled: false, action: { type: "none" } });
    else if (type === "focus_micontrol") onChange({ ...binding, enabled: autoEnabled, action: { type: "focus_micontrol" } });
    else if (type === "open_main_window") onChange({ ...binding, enabled: autoEnabled, action: { type: "open_main_window" } });
    else if (type === "open_url") onChange({ ...binding, enabled: autoEnabled, action: { type: "open_url", url: urlValue } });
    else if (type === "launch_app") onChange({ ...binding, enabled: autoEnabled, action: { type: "launch_app", path: appPath, args: [] } });
    else if (type === "remap_to_key") {
      const def = REMAP_TARGETS[0];
      onChange({ ...binding, enabled: autoEnabled, action: { type: "remap_to_key", vk: def.vk, extended: def.extended } });
    } else if (type === "set_performance_mode") {
      onChange({ ...binding, enabled: autoEnabled, action: { type: "set_performance_mode", mode: "balance" } });
    } else if (type === "toggle_ai_brightness") {
      onChange({ ...binding, enabled: autoEnabled, action: { type: "toggle_ai_brightness" } });
    } else if (type === "media_control") {
      onChange({ ...binding, enabled: autoEnabled, action: { type: "media_control", action: "play_pause" } });
    } else if (type === "script") {
      onChange({ ...binding, enabled: autoEnabled, action: { type: "script", interpreter: "powershell", path: "", args: [] } });
    }
  }

  async function handleDetect() {
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("start_key_detect");
    setDetecting(true);
    setDetectedVk("…");
    let tries = 0;
    const poll = setInterval(async () => {
      tries++;
      const vk = await invoke<number>("get_detected_key");
      if (vk !== 0) {
        setDetectedVk(`VK 0x${vk.toString(16).toUpperCase().padStart(2, "0")}`);
        setDetecting(false);
        clearInterval(poll);
      } else if (tries >= 50) {
        setDetectedVk("");
        setDetecting(false);
        clearInterval(poll);
      }
    }, 200);
  }

  function handleClear() {
    onChange({ ...binding, enabled: false, action: { type: "none" } });
    setDetectedVk("");
  }

  const hasAction = actionType !== "none";

  return (
    <div className="card" style={{ marginBottom: 10, padding: "14px 16px" }}>
      {/* Header row: toggle + label + vk badge + detect + clear */}
      <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
        <label className="toggle-switch" style={{ flexShrink: 0 }}>
          <input
            type="checkbox"
            checked={binding.enabled}
            onChange={(e) => onChange({ ...binding, enabled: e.target.checked })}
          />
          <span className="toggle-track" />
          <span className="toggle-knob" />
        </label>

        <div style={{ flex: 1, minWidth: 0 }}>
          <div className="card-title" style={{ margin: 0, fontSize: 13.5 }}>{label}</div>
          <div style={{ fontSize: 11.5, opacity: 0.55, marginTop: 1, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{description}</div>
        </div>

        {detectedVk && !detecting && (
          <span style={{
            fontSize: 10.5, fontFamily: "var(--font-mono)", padding: "2px 8px",
            borderRadius: 6, background: "var(--surface-2)", border: "1px solid var(--border)",
            color: "var(--accent)", flexShrink: 0,
          }}>{detectedVk}</span>
        )}

        <button
          className="btn-secondary"
          onClick={handleDetect}
          disabled={detecting}
          style={{ fontSize: 11, padding: "3px 10px", flexShrink: 0 }}
          title="Press the physical key to detect its VK code (up to 10 s)"
        >
          {detecting ? (detectedVk === "…" ? t("keyboard.detectKeyActive") : detectedVk) : t("keyboard.detectKey")}
        </button>

        {hasAction && (
          <button
            onClick={handleClear}
            title="Clear this key binding"
            style={{
              flexShrink: 0, background: "none", border: "1px solid var(--border)",
              borderRadius: 6, padding: "3px 8px", cursor: "pointer",
              fontSize: 11, color: "var(--color-warning, oklch(75% 0.18 55))",
              opacity: 0.8, lineHeight: 1.4,
            }}
          >
            ✕
          </button>
        )}
      </div>

      {/* Action row */}
      <div style={{
        display: "flex", gap: 8, flexWrap: "wrap", alignItems: "center",
        marginTop: 10,
        paddingTop: 10,
        borderTop: "1px solid var(--border)",
      }}>
        <select
          className="select-input"
          value={actionType}
          onChange={(e) => setActionType(e.target.value)}
          style={{ minWidth: 200, fontSize: 12 }}
        >
          <option value="none">{t("keyboard.actionNone")}</option>
          <option value="focus_micontrol">{t("keyboard.actionFocusMicontrol")}</option>
          <option value="open_main_window">{t("keyboard.actionOpenMainWindow")}</option>
          <option value="remap_to_key">{t("keyboard.actionRemapToKey")}</option>
          <option value="set_performance_mode">{t("keyboard.actionSetPerformanceMode")}</option>
          <option value="toggle_ai_brightness">{t("keyboard.actionToggleAiBrightness")}</option>
          <option value="media_control">{t("keyboard.actionMediaControl")}</option>
          <option value="open_url">{t("keyboard.actionOpenUrl")}</option>
          <option value="launch_app">{t("keyboard.actionLaunchApp")}</option>
          <option value="script">{t("keyboard.actionScript")}</option>
        </select>

        {actionType === "remap_to_key" && (
          <select
            className="select-input"
            value={remapVk}
            onChange={(e) => {
              const vk = Number(e.target.value);
              const target = REMAP_TARGETS.find((rt) => rt.vk === vk) ?? REMAP_TARGETS[0];
              onChange({ ...binding, action: { type: "remap_to_key", vk: target.vk, extended: target.extended } });
            }}
            style={{ fontSize: 12 }}
          >
            {REMAP_TARGETS.map((rt) => (
              <option key={rt.vk} value={rt.vk}>{rt.label}</option>
            ))}
          </select>
        )}

        {actionType === "set_performance_mode" && (
          <select
            className="select-input"
            value={perfMode}
            onChange={(e) =>
              onChange({ ...binding, action: { type: "set_performance_mode", mode: e.target.value } })
            }
            style={{ fontSize: 12 }}
          >
            {PERFORMANCE_MODES.map((m) => (
              <option key={m.value} value={m.value}>{m.label}</option>
            ))}
          </select>
        )}

        {actionType === "media_control" && (
          <select
            className="select-input"
            value={mediaAction}
            onChange={(e) =>
              onChange({ ...binding, action: { type: "media_control", action: e.target.value } })
            }
            style={{ fontSize: 12 }}
          >
            <option value="volume_up">{t("keyboard.mediaVolUp")}</option>
            <option value="volume_down">{t("keyboard.mediaVolDown")}</option>
            <option value="mute">{t("keyboard.mediaMute")}</option>
            <option value="play_pause">{t("keyboard.mediaPlayPause")}</option>
            <option value="next">{t("keyboard.mediaNext")}</option>
            <option value="prev">{t("keyboard.mediaPrev")}</option>
          </select>
        )}

        {actionType === "open_url" && (
          <input
            className="text-input"
            type="text"
            placeholder={t("keyboard.urlPlaceholder")}
            value={urlValue}
            onChange={(e) =>
              onChange({ ...binding, action: { type: "open_url", url: e.target.value } })
            }
            style={{ flex: 1, minWidth: 200, fontSize: 12 }}
          />
        )}
        {actionType === "launch_app" && (
          <input
            className="text-input"
            type="text"
            placeholder={t("keyboard.appPlaceholder")}
            value={appPath}
            onChange={(e) =>
              onChange({
                ...binding,
                action: { type: "launch_app", path: e.target.value, args: [] },
              })
            }
            style={{ flex: 1, minWidth: 200, fontSize: 12 }}
          />
        )}
        {actionType === "script" && (
          <select
            className="select-input"
            value={scriptInterp}
            onChange={(e) =>
              onChange({ ...binding, action: { type: "script", interpreter: e.target.value, path: scriptPath, args: [] } })
            }
            style={{ fontSize: 12 }}
          >
            <option value="">{t("keyboard.scriptInterpreterDirect")}</option>
            <option value="powershell">{t("keyboard.scriptInterpreterPowershell")}</option>
            <option value="cmd">{t("keyboard.scriptInterpreterCmd")}</option>
          </select>
        )}
        {actionType === "script" && (
          <input
            className="text-input"
            type="text"
            placeholder={t("keyboard.scriptPathPlaceholder")}
            value={scriptPath}
            onChange={(e) =>
              onChange({ ...binding, action: { type: "script", interpreter: scriptInterp, path: e.target.value, args: [] } })
            }
            style={{ flex: 1, minWidth: 200, fontSize: 12 }}
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
  const [hookActive, setHookActive] = useState<boolean | null>(null);
  const { addToast } = useToast();

  useEffect(() => {
    import("@tauri-apps/api/core").then(({ invoke }) => {
      invoke<HotkeyMap>("get_hotkey_config")
        .then(setConfig)
        .catch((e) => console.error("get_hotkey_config", e));
      invoke<boolean>("is_hook_active")
        .then(setHookActive)
        .catch(() => setHookActive(false));
    });
  }, []);

  async function save() {
    if (!config) return;
    setSaving(true);
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("set_hotkey_config", { config });
      setSaved(true);
      addToast(t("keyboard.saved"), "success");
      setTimeout(() => setSaved(false), 2000);
    } catch (e) {
      console.error("set_hotkey_config", e);
      addToast(`${t("keyboard.saveError")}: ${String(e)}`, "error");
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

      {hookActive !== null && (
        <div style={{
          display: "inline-flex", alignItems: "center", gap: 6,
          fontSize: 12, marginBottom: 12, opacity: 0.8,
          color: hookActive ? "var(--color-success, #4caf50)" : "var(--color-warning, #ff9800)",
        }}>
          <span style={{ width: 8, height: 8, borderRadius: "50%", background: "currentColor", display: "inline-block" }} />
          {hookActive ? t("keyboard.hookActive") : t("keyboard.hookInactive")}
        </div>
      )}

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
      <IotModulePanel hw={hw} />
    </>
  );
}

const IOT_REGIONS: IotRegionName[] = ["ERAM", "SMA2", "IOT_STATUS", "IOT_SENSORS"];

function formatHexPreview(hex: string, bytes = 16): string {
  if (!hex) return "—";
  const trimmed = hex.slice(0, bytes * 2);
  const pairs = trimmed.match(/.{1,2}/g) ?? [];
  return `${pairs.join(" ")}${hex.length > trimmed.length ? " ..." : ""}`;
}

function formatPerfProfile(value: number): string {
  switch (value) {
    case 0: return "Balance";
    case 1: return "Performance";
    case 2: return "Silence";
    default: return `0x${value.toString(16).toUpperCase().padStart(2, "0")}`;
  }
}

// Known writable ECRAM registers (ERAM base 0xFE0B0300)
const ERAM_BASE = 0xFE0B0300;
const ERAM_KNOWN_WRITES: { label: string; offset: number; values: { label: string; byte: number }[] }[] = [
  {
    label: "Performance Profile",
    offset: 0x40,
    values: [
      { label: "Balanced", byte: 0x00 },
      { label: "Performance", byte: 0x01 },
      { label: "Silent", byte: 0x02 },
    ],
  },
  {
    label: "AI Limit (AILM)",
    offset: 0x1B,
    values: [
      { label: "Enable AI Limit", byte: 0x04 },
      { label: "Disable AI Limit", byte: 0x00 },
    ],
  },
  {
    label: "Long Battery Limit (LBLM)",
    offset: 0x1B,
    values: [
      { label: "Enable Long Battery Limit", byte: 0x08 },
      { label: "Disable Long Battery Limit", byte: 0x00 },
    ],
  },
];

function IotModulePanel({ hw }: { hw: Hardware }) {
  const [elevated, setElevated] = useState<boolean | null>(null);
  const [ecramMap, setEcramMap] = useState<EramMap | null>(null);
  const [regions, setRegions] = useState<Partial<Record<IotRegionName, string>>>({});
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [relaunching, setRelaunching] = useState(false);

  // Raw read state
  const [rawReadAddr, setRawReadAddr] = useState("0xFE0B0300");
  const [rawReadCount, setRawReadCount] = useState("16");
  const [rawReadResult, setRawReadResult] = useState<string | null>(null);
  const [rawReadLoading, setRawReadLoading] = useState(false);

  // Raw write state
  const [writeAddress, setWriteAddress] = useState("0xFE0B0300");
  const [writeHex, setWriteHex] = useState("");
  const [writeStatus, setWriteStatus] = useState<string | null>(null);

  const checkElevation = useCallback(async () => {
    try {
      const elev = await hw.isElevated();
      setElevated(elev);
    } catch {
      setElevated(false);
    }
  }, [hw]);

  const refreshIot = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [map, ...hexes] = await Promise.all([
        hw.getEcramMap(),
        ...IOT_REGIONS.map((region) => hw.getIotRegionHex(region)),
      ]);
      setEcramMap(map);
      setRegions({
        ERAM: hexes[0],
        SMA2: hexes[1],
        IOT_STATUS: hexes[2],
        IOT_SENSORS: hexes[3],
      });
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [hw]);

  useEffect(() => {
    void checkElevation().then(() => {
      if (elevated !== false) void refreshIot();
    });
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (elevated === true) void refreshIot();
  }, [elevated, refreshIot]);

  const handleRelaunch = useCallback(async () => {
    setRelaunching(true);
    try {
      await hw.relaunchAsAdmin();
    } catch (e) {
      setRelaunching(false);
      setError(`Re-launch failed: ${String(e)}`);
    }
  }, [hw]);

  const handleKnownWrite = useCallback(async (offset: number, byte: number) => {
    const addr = `0x${(ERAM_BASE + offset).toString(16).toUpperCase()}`;
    try {
      await hw.writeIotHex(addr, byte.toString(16).padStart(2, "0"));
      setTimeout(() => void refreshIot(), 300);
    } catch (e) {
      setError(`Write failed: ${String(e)}`);
    }
  }, [hw, refreshIot]);

  const handleRawRead = useCallback(async () => {
    setRawReadLoading(true);
    setRawReadResult(null);
    try {
      const count = parseInt(rawReadCount, 10);
      const hex = await hw.readEcramRaw(rawReadAddr, count);
      // Format as hex dump (16 bytes per line)
      const bytes = (hex.match(/.{1,2}/g) ?? []);
      const addrBase = parseInt(rawReadAddr.replace(/^0x/i, ""), 16) || 0;
      const lines: string[] = [];
      for (let i = 0; i < bytes.length; i += 16) {
        const lineAddr = `0x${(addrBase + i).toString(16).toUpperCase().padStart(8, "0")}`;
        const lineHex = bytes.slice(i, i + 16).join(" ");
        const lineAscii = bytes.slice(i, i + 16)
          .map((b) => { const c = parseInt(b, 16); return c >= 0x20 && c < 0x7F ? String.fromCharCode(c) : "."; })
          .join("");
        lines.push(`${lineAddr}: ${lineHex.padEnd(47)}  ${lineAscii}`);
      }
      setRawReadResult(lines.join("\n"));
    } catch (e) {
      setRawReadResult(`Error: ${String(e)}`);
    } finally {
      setRawReadLoading(false);
    }
  }, [hw, rawReadAddr, rawReadCount]);

  // ── Not-elevated gate ─────────────────────────────────────────────────────
  if (elevated === false) {
    return (
      <div className="card" style={{ marginTop: 14 }}>
        <div className="card-title" style={{ marginBottom: 4 }}>IoT Module</div>
        <div style={{ display: "flex", gap: 20, alignItems: "flex-start", padding: "16px 0" }}>
          <span style={{ fontSize: 40, lineHeight: 1 }}>🔒</span>
          <div>
            <div style={{ fontWeight: 600, fontSize: 15, marginBottom: 6 }}>Administrator required</div>
            <div style={{ fontSize: 13, color: "var(--text-muted)", marginBottom: 16, maxWidth: 420 }}>
              IoTDriver requires the process to have an elevated token. Being logged in as an
              administrator is not enough — the app must be launched with UAC elevation.
              Click below to restart MiControl as administrator.
            </div>
            {error && (
              <div style={{ fontSize: 12, color: "var(--error)", marginBottom: 10 }}>{error}</div>
            )}
            <button
              className="btn-primary"
              onClick={() => void handleRelaunch()}
              disabled={relaunching}
              style={{ minWidth: 240 }}
            >
              {relaunching ? "Waiting for UAC…" : "Re-launch as Administrator"}
            </button>
          </div>
        </div>
      </div>
    );
  }

  // ── Elevated: full panel ──────────────────────────────────────────────────
  return (
    <div className="card" style={{ marginTop: 14 }}>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12, marginBottom: 14 }}>
        <div>
          <div className="card-title" style={{ marginBottom: 2 }}>
            IoT Module
            {elevated && <span style={{ marginLeft: 8, fontSize: 11, color: "var(--success, #4caf50)", fontWeight: 500 }}>● Elevated</span>}
          </div>
          <div style={{ fontSize: 12, color: "var(--text-muted)" }}>
            Full ECRAM access via IoTDriver. Read/write decoded registers and raw physical memory.
          </div>
        </div>
        <button className="btn-secondary" onClick={() => void refreshIot()} disabled={loading}>
          {loading ? "Reading…" : "Refresh"}
        </button>
      </div>

      {error && (
        <div style={{
          marginBottom: 14,
          padding: "10px 14px",
          borderRadius: 10,
          background: "color-mix(in srgb, var(--error) 10%, transparent)",
          border: "1px solid color-mix(in srgb, var(--error) 30%, transparent)",
          fontSize: 13,
          color: "var(--error)",
        }}>
          {error}
        </div>
      )}

      {ecramMap && (
        <>
          {/* Decoded registers */}
          <div className="grid-2" style={{ marginBottom: 14 }}>
            <div className="card" style={{ marginBottom: 0 }}>
              <div className="card-title" style={{ fontSize: 13 }}>System State</div>
              <div className="stat-row"><span className="stat-label">AC connected</span><span className="stat-value">{ecramMap.ac_connected ? "Yes" : "No"}</span></div>
              <div className="stat-row"><span className="stat-label">Adapter power</span><span className="stat-value">{ecramMap.ac_adapter_w} W</span></div>
              <div className="stat-row"><span className="stat-label">Battery current</span><span className="stat-value">{ecramMap.battery_current_ma} mA</span></div>
              <div className="stat-row"><span className="stat-label">Battery voltage</span><span className="stat-value">{ecramMap.battery_voltage_mv} mV</span></div>
              <div className="stat-row"><span className="stat-label">Battery capacity</span><span className="stat-value">{ecramMap.battery_capacity_mah} mAh</span></div>
              <div className="stat-row"><span className="stat-label">Charge limit</span><span className="stat-value">{ecramMap.charge_threshold_pct} %</span></div>
              <div className="stat-row"><span className="stat-label">Battery temp</span><span className="stat-value">{ecramMap.battery_temp_c} °C</span></div>
              <div className="stat-row"><span className="stat-label">CPU temp</span><span className="stat-value">{ecramMap.cpu_temp_c} °C</span></div>
              <div className="stat-row"><span className="stat-label">CPU power</span><span className="stat-value">{ecramMap.cpu_power_w} W</span></div>
              <div className="stat-row"><span className="stat-label">Fan 1 RPM</span><span className="stat-value">{ecramMap.fan_rpm}</span></div>
              {ecramMap.fan2_rpm > 0 && <div className="stat-row"><span className="stat-label">Fan 2 RPM</span><span className="stat-value">{ecramMap.fan2_rpm}</span></div>}
            </div>
            <div className="card" style={{ marginBottom: 0 }}>
              <div className="card-title" style={{ fontSize: 13 }}>Mode / Limits (decoded)</div>
              <div className="stat-row"><span className="stat-label">Performance profile</span><span className="stat-value">{formatPerfProfile(ecramMap.perf_profile)} (0x{ecramMap.perf_profile.toString(16).toUpperCase()})</span></div>
              <div className="stat-row"><span className="stat-label">TDP override</span><span className="stat-value">{ecramMap.tdp_w} W</span></div>
              <div className="stat-row"><span className="stat-label">Smart profile</span><span className="stat-value">{ecramMap.smart_mode_profile ?? "—"}</span></div>
              <div className="stat-row"><span className="stat-label">SMMT</span><span className="stat-value">0x{ecramMap.smart_mode_type.toString(16).toUpperCase().padStart(2, "0")}</span></div>
              <div className="stat-row"><span className="stat-label">SMMD</span><span className="stat-value">0x{ecramMap.smart_mode_data.toString(16).toUpperCase().padStart(2, "0")}</span></div>
              <div className="stat-row"><span className="stat-label">QFAN</span><span className="stat-value">0x{ecramMap.qfan_mode.toString(16).toUpperCase().padStart(2, "0")}</span></div>
              <div className="stat-row"><span className="stat-label">AI limit (AILM)</span><span className="stat-value">{ecramMap.ai_limit_enabled ? "Enabled" : "Disabled"}</span></div>
              <div className="stat-row"><span className="stat-label">Long battery limit</span><span className="stat-value">{ecramMap.long_battery_limit_enabled ? "Enabled" : "Disabled"}</span></div>
              <div className="stat-row"><span className="stat-label">Display brightness</span><span className="stat-value">{ecramMap.display_brightness_level}</span></div>
              <div className="stat-row"><span className="stat-label">KB backlight</span><span className="stat-value">{ecramMap.keyboard_backlight_level}</span></div>
              <div className="stat-row"><span className="stat-label">control_flags[0x1B]</span><span className="stat-value">0x{ecramMap.control_flags_1b.toString(16).toUpperCase().padStart(2, "0")}</span></div>
            </div>
          </div>

          {/* Known-safe write controls */}
          <div className="card" style={{ marginBottom: 14 }}>
            <div className="card-title" style={{ fontSize: 13 }}>Write Controls (known-safe registers)</div>
            <div style={{ fontSize: 12, color: "var(--text-muted)", marginBottom: 10 }}>
              Direct ECRAM writes via IoTDriver. Changes take effect immediately; some may require a fan/profile cycle to apply.
            </div>
            <div style={{ display: "grid", gap: 12 }}>
              {ERAM_KNOWN_WRITES.map((reg) => (
                <div key={reg.label} style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
                  <span style={{ fontSize: 12, fontWeight: 600, minWidth: 180 }}>
                    {reg.label}
                    <span style={{ fontFamily: "var(--font-mono)", fontSize: 10, color: "var(--text-muted)", marginLeft: 6 }}>
                      +0x{reg.offset.toString(16).toUpperCase().padStart(2, "0")}
                    </span>
                  </span>
                  {reg.values.map((v) => (
                    <button
                      key={v.label}
                      className="btn-secondary"
                      style={{ fontSize: 11, padding: "4px 10px" }}
                      onClick={() => void handleKnownWrite(reg.offset, v.byte)}
                    >
                      {v.label}
                    </button>
                  ))}
                </div>
              ))}
            </div>
          </div>
        </>
      )}

      {/* Raw region dumps */}
      <div style={{ display: "grid", gap: 8, marginBottom: 14 }}>
        {IOT_REGIONS.map((region) => (
          <details key={region} style={{ border: "1px solid var(--border)", borderRadius: 10, padding: "8px 12px", background: "var(--surface-2)" }}>
            <summary style={{ cursor: "pointer", fontWeight: 500, fontSize: 13, display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <span>{region}</span>
              <span style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--text-muted)" }}>
                {formatHexPreview(regions[region] ?? "")}
              </span>
            </summary>
            <pre style={{ marginTop: 8, fontSize: 11, whiteSpace: "pre-wrap", wordBreak: "break-all", color: "var(--text-muted)" }}>
              {regions[region] ?? "No data"}
            </pre>
          </details>
        ))}
      </div>

      {/* Raw read tool */}
      <details style={{ marginBottom: 10 }}>
        <summary style={{ cursor: "pointer", fontWeight: 600, fontSize: 13, padding: "8px 0" }}>
          Raw Read — arbitrary address
        </summary>
        <div className="card" style={{ marginTop: 8, marginBottom: 0 }}>
          <div style={{ display: "grid", gridTemplateColumns: "1fr auto auto", gap: 8, alignItems: "end", marginBottom: 8 }}>
            <div>
              <div style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 4 }}>Physical address (hex)</div>
              <input className="text-input" value={rawReadAddr} onChange={(e) => setRawReadAddr(e.target.value)} placeholder="0xFE0B0300" />
            </div>
            <div>
              <div style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 4 }}>Bytes (1–256)</div>
              <input className="text-input" value={rawReadCount} onChange={(e) => setRawReadCount(e.target.value)} placeholder="16" style={{ width: 72 }} />
            </div>
            <button className="btn-secondary" disabled={rawReadLoading} onClick={() => void handleRawRead()}>
              {rawReadLoading ? "Reading…" : "Read"}
            </button>
          </div>
          {rawReadResult && (
            <pre style={{ fontSize: 11, whiteSpace: "pre-wrap", wordBreak: "break-all", color: "var(--text-muted)", marginTop: 4, fontFamily: "var(--font-mono)" }}>
              {rawReadResult}
            </pre>
          )}
        </div>
      </details>

      {/* Raw write tool */}
      <details>
        <summary style={{ cursor: "pointer", fontWeight: 600, fontSize: 13, padding: "8px 0" }}>
          Raw Write — arbitrary address
          <span style={{ fontSize: 11, color: "var(--warning, #ff9800)", marginLeft: 8, fontWeight: 400 }}>⚠ danger zone</span>
        </summary>
        <div className="card" style={{ marginTop: 8, marginBottom: 0 }}>
          <div style={{ fontSize: 12, color: "var(--text-muted)", marginBottom: 10 }}>
            Direct write to any physical address via IOCTL 0x22E004. Use only with verified-safe offsets.
          </div>
          <div style={{ display: "grid", gap: 8 }}>
            <input className="text-input" value={writeAddress} onChange={(e) => setWriteAddress(e.target.value)} placeholder="0xFE0B0300" />
            <textarea
              className="text-input"
              value={writeHex}
              onChange={(e) => setWriteHex(e.target.value)}
              placeholder="hex bytes e.g. 01 02 03 04"
              rows={3}
              style={{ resize: "vertical", fontFamily: "var(--font-mono)", fontSize: 12 }}
            />
            <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
              <button
                className="btn-secondary"
                disabled={loading || !writeHex.trim()}
                onClick={async () => {
                  setWriteStatus(null);
                  try {
                    await hw.writeIotHex(writeAddress, writeHex);
                    setWriteStatus("Write OK");
                    setTimeout(() => void refreshIot(), 200);
                  } catch (e) {
                    setWriteStatus(`Write failed: ${String(e)}`);
                  }
                }}
              >
                Write
              </button>
              {writeStatus && (
                <span style={{ fontSize: 12, color: writeStatus.startsWith("Write failed") ? "var(--error)" : "var(--success, #4caf50)" }}>
                  {writeStatus}
                </span>
              )}
            </div>
          </div>
        </div>
      </details>
    </div>
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

function AiAnalysisTab({ hw, ai, onOpenSettings }: { hw: Hardware; ai: AiSettings; onOpenSettings: () => void }) {
  return (
    <>
      <PageHeader title={t("aiAnalysis.title")} subtitle={t("aiAnalysis.subtitle")} />
      <AiAnalysis hw={hw} ai={ai} onOpenSettings={onOpenSettings} />
    </>
  );
}

export default function MainWindow({ hardware, activeTab, onTabChange, themeMode, toggleTheme }: Props) {
  const aiSettings = useSettings();
  const [showTrayPreview, setShowTrayPreview] = useState(false);
  const refreshErrorLabels: Record<string, string> = {
    system_info: "System",
    battery: "Battery",
    display: "Display",
    fan: "Fan",
    touchpad: "Touchpad",
    performance_mode: "Performance",
    charging_threshold: "Charging",
  };
  const refreshErrorItems = Object.entries(hardware.refreshErrors).filter(
    ([, message]) => Boolean(message)
  );

  // Background logger — runs regardless of active tab
  useAnalysisLogger(hardware, aiSettings);

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
      case "ai_analysis": return <AiAnalysisTab hw={hardware} ai={aiSettings} onOpenSettings={() => onTabChange("settings")} />;
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
          {refreshErrorItems.length > 0 && (
            <div
              style={{
                padding: "4px 8px",
                fontSize: 10,
                color: "var(--text-dim)",
                display: "flex",
                flexDirection: "column",
                gap: 3,
              }}
            >
              {refreshErrorItems.map(([key, message]) => (
                <div key={key} title={message ?? undefined}>
                  {refreshErrorLabels[key] ?? key}: {message}
                </div>
              ))}
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
          position: "fixed", bottom: 10, right: 14,
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
