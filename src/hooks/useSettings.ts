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

const STORAGE_KEY = "micontrol_settings_v1";

export interface AppSettings {
  /** OpenAI (or compatible) API key */
  openai_api_key: string;
  /** Base URL — change to use Ollama, Azure, or any OpenAI-compatible endpoint */
  openai_base_url: string;
  /** Model name */
  openai_model: string;
}

export const DEFAULT_SETTINGS: AppSettings = {
  openai_api_key: "",
  openai_base_url: "https://api.openai.com/v1",
  openai_model: "gpt-4o-mini",
};

function loadSettings(): AppSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
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
Capacity: ${bat?.full_capacity_mah ?? "?"} mAh (designed: ${bat?.designed_capacity_mah ?? "?"} mAh)

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

  return {
    settings,
    saveSettings,
    updateKey,
    analyzeSystem,
    testConnection,
    isConfigured: Boolean(settings.openai_api_key.trim()),
  };
}
