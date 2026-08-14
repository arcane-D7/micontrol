/**
 * Settings-related type definitions.
 *
 * Extracted from `src/hooks/useSettings.ts` (S28-003).
 */

import type {
  BatteryInfo,
  DisplayInfo,
  FanInfo,
  HardwareCapabilities,
  PerformanceMode,
  SystemInfo,
} from './hardware';

export interface AppSettings {
  /** Whether the first-run onboarding wizard has been completed. */
  onboardingCompleted: boolean;
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
  /** MCP integration (DOM control for AI agents / debug tools) — opt-in. */
  mcp_integration_enabled: boolean;
}

/** Context for AI analysis — snapshot of current hardware state. */
export interface SystemContext {
  deviceModel: string | null;
  systemInfo: SystemInfo | null;
  battery: BatteryInfo | null;
  performanceMode: PerformanceMode | null;
  fan: FanInfo | null;
  display: DisplayInfo | null;
  capabilities: HardwareCapabilities | null;
}

/** Shared log entry type for AI Analysis module. */
export interface AnalysisLogEntry {
  id: string;
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
