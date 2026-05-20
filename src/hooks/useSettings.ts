import { useState } from "react";
import type {
  SystemInfo,
  BatteryInfo,
  FanInfo,
  DisplayInfo,
  PerformanceMode,
  HardwareCapabilities,
} from "./useHardware";

// ── Persisted settings ────────────────────────────────────────────────────────

const STORAGE_KEY = "micontrol_settings_v2";
const STORAGE_KEY_V1 = "micontrol_settings_v1";

export interface AppSettings {
  /** OpenAI (or compatible) API key */
  openai_api_key: string;
  /** Base URL — change to use Ollama, Azure, or any OpenAI-compatible endpoint */
  openai_base_url: string;
  /** Model name */
  openai_model: string;
  /** Performance mode to auto-apply when plugged in. null = manual only. */
  perf_mode_ac: PerformanceMode | null;
  /** Performance mode to auto-apply when on battery. null = manual only. */
  perf_mode_dc: PerformanceMode | null;
  /** Whether to automatically switch performance mode on AC/DC state change. */
  auto_switch_perf: boolean;
  /** Tray popup window opacity (0.3 – 1.0). */
  tray_opacity: number;
  /** Whether the AI Analysis background logger is active. */
  ai_analysis_enabled: boolean;
  /** How often (in seconds) to collect a performance snapshot. */
  ai_poll_interval_sec: number;
  /** How many times per day to automatically send logs to AI for analysis. */
  ai_daily_analyses: number;
}

export const DEFAULT_SETTINGS: AppSettings = {
  openai_api_key: "",
  openai_base_url: "https://api.openai.com/v1",
  openai_model: "gpt-4o-mini",
  perf_mode_ac: null,
  perf_mode_dc: null,
  auto_switch_perf: false,
  tray_opacity: 1.0,
  ai_analysis_enabled: false,
  ai_poll_interval_sec: 60,
  ai_daily_analyses: 2,
};

function loadSettings(): AppSettings {
  try {
    // Try v2 first, fall back to v1 (migrating AI keys across)
    const raw = localStorage.getItem(STORAGE_KEY) ?? localStorage.getItem(STORAGE_KEY_V1);
    return raw ? { ...DEFAULT_SETTINGS, ...JSON.parse(raw) } : DEFAULT_SETTINGS;
  } catch {
    return DEFAULT_SETTINGS;
  }
}

// ── Context for AI analysis ───────────────────────────────────────────────────

export interface SystemContext {
  deviceModel: string | null;
  systemInfo: SystemInfo | null;
  battery: BatteryInfo | null;
  performanceMode: PerformanceMode | null;
  fan: FanInfo | null;
  display: DisplayInfo | null;
  capabilities: HardwareCapabilities | null;
}

function buildPrompt(ctx: SystemContext): string {
  const sys = ctx.systemInfo;
  const bat = ctx.battery;
  const fan = ctx.fan;
  const disp = ctx.display;
  const cap = ctx.capabilities;

  return `You are analyzing a Xiaomi laptop. Provide concise, specific recommendations.

== HARDWARE ==
Device: ${ctx.deviceModel ?? "Unknown"}
CPU: ${sys?.cpu_name ?? "Unknown"} — usage: ${sys?.cpu_usage?.toFixed(0) ?? "?"}%
RAM: ${sys?.ram_used_gb?.toFixed(1) ?? "?"} / ${sys?.ram_total_gb ?? "?"} GB used
OS: ${sys?.os_version ?? "Unknown"}

== BATTERY ==
Level: ${bat?.level ?? "?"}%  |  Charging: ${bat?.is_charging ? "yes" : "no"}
Health: ${bat?.health_percent ?? "?"}%  |  Cycles: ${bat?.cycle_count ?? "?"}
Temperature: ${bat?.temperature_celsius != null ? bat.temperature_celsius + "°C" : "unavailable"}
Capacity: ${bat?.full_capacity_mwh ?? "?"} mWh (designed: ${bat?.designed_capacity_mwh ?? "?"} mWh)

== PERFORMANCE ==
Current mode: ${ctx.performanceMode ?? "unknown"}
Fan: ${fan?.mode ?? "?"} — ${fan?.speed_rpm ?? "?"}rpm  |  GPU temp: ${fan?.gpu_temp_celsius ?? "?"}°C
Display: brightness ${disp?.brightness ?? "?"}%  |  refresh ${disp?.refresh_rate_hz ?? "?"}Hz  |  HDR: ${disp?.hdr_enabled ?? false}

== HARDWARE CAPABILITIES ==
VHF performance control: ${cap?.has_vhf_performance ? "✓" : "✗ (registry fallback)"}
IoT charging service: ${cap?.has_iot_charging ? "✓" : "✗ (registry fallback)"}
Intel IGCL display: ${cap?.has_igcl ? "✓" : "✗"}
Touchpad HID channel: ${cap?.has_touchpad_hid ? "✓" : "✗"}
Touchscreen: ${cap?.has_touchscreen ? "✓" : "✗"}
Stylus: ${cap?.has_stylus ? "✓" : "✗"}

== REQUESTED ANALYSIS ==
1. Battery health assessment — is the health/cycle count concerning? Recommend optimal charging threshold (values: 60, 70, 80, 100).
2. Performance mode recommendation — is the current mode suitable for the measured CPU/GPU load?
3. Thermal assessment — is the GPU temperature healthy?
4. Any issues or warnings detected from the capability flags.
5. Two specific optimisation tips for this device profile.

Be concise. Use bullet points.`.trim();
}

// ── Hook ──────────────────────────────────────────────────────────────────────

export function useSettings() {
  const [settings, setSettingsState] = useState<AppSettings>(loadSettings);

  function saveSettings(updated: AppSettings) {
    setSettingsState(updated);
    localStorage.setItem(STORAGE_KEY, JSON.stringify(updated));
  }

  function updateKey<K extends keyof AppSettings>(key: K, value: AppSettings[K]) {
    saveSettings({ ...settings, [key]: value });
  }

  /** Sends system context to the configured AI model and returns the analysis text. */
  async function analyzeSystem(ctx: SystemContext): Promise<string> {
    if (!settings.openai_api_key.trim()) {
      throw new Error("api_key_missing");
    }

    const baseUrl = settings.openai_base_url.replace(/\/+$/, "");
    const res = await fetch(`${baseUrl}/chat/completions`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "Authorization": `Bearer ${settings.openai_api_key.trim()}`,
      },
      body: JSON.stringify({
        model: settings.openai_model || "gpt-4o-mini",
        messages: [
          {
            role: "system",
            content:
              "You are a hardware optimization assistant specialising in Xiaomi laptops running Windows. Give clear, actionable advice in 200 words or less.",
          },
          { role: "user", content: buildPrompt(ctx) },
        ],
        max_tokens: 700,
        temperature: 0.3,
      }),
    });

    if (!res.ok) {
      let detail = "";
      try {
        const err = await res.json();
        detail = err?.error?.message ?? JSON.stringify(err);
      } catch {
        detail = await res.text();
      }
      throw new Error(`API ${res.status}: ${detail}`);
    }

    const json = await res.json() as {
      choices?: Array<{ message?: { content?: string } }>;
    };
    return json.choices?.[0]?.message?.content?.trim() ?? "No response from model.";
  }

  /** Quick connectivity + auth test — sends a minimal prompt. */
  async function testConnection(): Promise<void> {
    if (!settings.openai_api_key.trim()) {
      throw new Error("api_key_missing");
    }
    const baseUrl = settings.openai_base_url.replace(/\/+$/, "");
    const res = await fetch(`${baseUrl}/chat/completions`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "Authorization": `Bearer ${settings.openai_api_key.trim()}`,
      },
      body: JSON.stringify({
        model: settings.openai_model || "gpt-4o-mini",
        messages: [{ role: "user", content: "Reply with the single word OK." }],
        max_tokens: 5,
      }),
    });

    if (!res.ok) {
      const err = await res.json().catch(() => ({})) as { error?: { message?: string } };
      throw new Error(`API ${res.status}: ${err?.error?.message ?? res.statusText}`);
    }
  }

  /**
   * Sends a structured log summary to the AI and returns the analysis text.
   * The AI is instructed to respond in the given language code (en/pt/es/fr).
   */
  async function analyzeWithLogs(
    logs: AnalysisLogEntry[],
    hwCtx: SystemContext,
    language: string,
  ): Promise<string> {
    if (!settings.openai_api_key.trim()) throw new Error("api_key_missing");
    if (logs.length === 0) throw new Error("no_logs");

    const langNames: Record<string, string> = {
      en: "English", pt: "Portuguese", es: "Spanish", fr: "French",
    };
    const langName = langNames[language] ?? "English";

    // Compute statistics from logs
    const n = logs.length;
    const avg = (arr: number[]) => arr.reduce((a, b) => a + b, 0) / arr.length;
    const max = (arr: number[]) => Math.max(...arr);

    const cpuTemps = logs.map((l) => l.cpu_temp);
    const gpuTemps = logs.map((l) => l.gpu_temp);
    const tdps = logs.filter((l) => l.tdp_watts != null).map((l) => l.tdp_watts as number);
    const cpuPcts = logs.map((l) => l.cpu_pct);
    const gpuPcts = logs.map((l) => l.gpu_pct);
    const batLevels = logs.filter((l) => l.battery_level != null).map((l) => l.battery_level as number);

    const first = logs[0];
    const last = logs[n - 1];
    const spanMin = Math.round(
      (new Date(last.ts).getTime() - new Date(first.ts).getTime()) / 60000,
    );

    // Top processes from last snapshot
    const topProcs = (last.top_processes ?? [])
      .sort((a, b) => b.cpu_pct - a.cpu_pct)
      .slice(0, 6)
      .map((p) => `  - ${p.name}: ${p.cpu_pct.toFixed(1)}% CPU, ${p.memory_mb.toFixed(0)} MB RAM`)
      .join("\n");

    const batterySection =
      batLevels.length > 1
        ? `**Battery:** ${batLevels[0].toFixed(0)}% → ${batLevels[batLevels.length - 1].toFixed(0)}% (${last.is_charging ? "charging" : "discharging"})`
        : "";

    const prompt = `Respond in ${langName}.

You are a hardware optimization assistant for a Xiaomi laptop. Analyze the following performance data.

## Performance Log Summary (${n} snapshots over ${spanMin} min)

**CPU Temperature:** avg ${avg(cpuTemps).toFixed(1)}°C, peak ${max(cpuTemps).toFixed(1)}°C
**GPU Temperature:** avg ${avg(gpuTemps).toFixed(1)}°C, peak ${max(gpuTemps).toFixed(1)}°C
**TDP (Package Power):** ${tdps.length ? `avg ${avg(tdps).toFixed(1)} W, peak ${max(tdps).toFixed(1)} W` : "unavailable"}
**CPU Usage:** avg ${avg(cpuPcts).toFixed(1)}%, peak ${max(cpuPcts).toFixed(1)}%
**GPU Usage:** avg ${avg(gpuPcts).toFixed(1)}%, peak ${max(gpuPcts).toFixed(1)}%
${batterySection}
**Performance Mode:** ${last.mode}

**Top Processes (latest snapshot):**
${topProcs || "  - No process data available"}

**Current System:**
- Device: ${hwCtx.deviceModel ?? "Xiaomi Laptop"}
- CPU: ${hwCtx.systemInfo?.cpu_name ?? "Unknown"} (${hwCtx.systemInfo?.cpu_cores ?? "?"} cores)
- RAM: ${hwCtx.systemInfo?.ram_used_gb?.toFixed(1) ?? "?"} / ${hwCtx.systemInfo?.ram_total_gb ?? "?"} GB used

## Analysis Tasks
1. **Thermal:** Are temperatures healthy? Any throttling risk?
2. **Performance:** Is the current mode optimal for the observed workload?
3. **Battery:** Is consumption normal? Any drain concerns?
4. **Top Processes:** Any resource-heavy process worth investigating?
5. **Recommendation:** Best performance mode for this usage pattern?

Be concise. Use short paragraphs with emoji section headers. Max 300 words.`;

    const baseUrl = settings.openai_base_url.replace(/\/+$/, "");
    const res = await fetch(`${baseUrl}/chat/completions`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "Authorization": `Bearer ${settings.openai_api_key.trim()}`,
      },
      body: JSON.stringify({
        model: settings.openai_model || "gpt-4o-mini",
        messages: [
          {
            role: "system",
            content: `You are a hardware optimization assistant for Xiaomi laptops. Always respond in ${langName}.`,
          },
          { role: "user", content: prompt },
        ],
        max_tokens: 800,
        temperature: 0.4,
      }),
    });

    if (!res.ok) {
      let detail = "";
      try { const err = await res.json(); detail = err?.error?.message ?? JSON.stringify(err); }
      catch { detail = await res.text(); }
      throw new Error(`API ${res.status}: ${detail}`);
    }

    const json = await res.json() as { choices?: Array<{ message?: { content?: string } }> };
    return json.choices?.[0]?.message?.content?.trim() ?? "No response from model.";
  }

  return {
    settings,
    saveSettings,
    updateKey,
    analyzeSystem,
    analyzeWithLogs,
    testConnection,
    isConfigured: Boolean(settings.openai_api_key.trim()),
  };
}

// ── Shared log entry type for AI Analysis module ──────────────────────────────

export interface AnalysisLogEntry {
  ts: string;
  mode: string;
  cpu_temp: number;
  gpu_temp: number;
  tdp_watts: number | null;
  cpu_pct: number;
  gpu_pct: number;
  battery_level: number | null;
  is_charging: boolean;
  top_processes: Array<{ name: string; cpu_pct: number; memory_mb: number }>;
}
