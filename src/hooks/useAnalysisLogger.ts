/**
 * useAnalysisLogger
 *
 * Background hook called from MainWindow so it runs regardless of active tab.
 * - Polls hardware data at `ai_poll_interval_sec` and stores snapshots in
 *   localStorage under `micontrol_analysis_logs_v1` (circular, max 500).
 * - Triggers AI analysis `ai_daily_analyses` times per day and stores the
 *   last result in `micontrol_last_analysis_v1`.
 * - All activity stops when `ai_analysis_enabled` is false.
 */

import { useEffect, useRef, useCallback } from "react";
import type { useHardware } from "./useHardware";
import type { useSettings as UseSettings } from "./useSettings";
import type { AnalysisLogEntry } from "./useSettings";

type Hardware = ReturnType<typeof useHardware>;
type Settings = ReturnType<typeof UseSettings>;

// ── localStorage keys ─────────────────────────────────────────────────────────
export const LOGS_KEY      = "micontrol_analysis_logs_v1";
export const LAST_KEY      = "micontrol_last_analysis_v1";
export const SCHEDULE_KEY  = "micontrol_analysis_schedule_v1";
const MAX_LOGS = 500;

export interface LastAnalysis {
  ts: string;       // ISO-8601
  text: string;
  log_count: number;
}

interface ScheduleState {
  /** ISO timestamps of the last N analyses (kept for de-dup / rate-limiting). */
  recent: string[];
}

// ── Helpers ───────────────────────────────────────────────────────────────────

export function loadLogs(): AnalysisLogEntry[] {
  try {
    return JSON.parse(localStorage.getItem(LOGS_KEY) ?? "[]") as AnalysisLogEntry[];
  } catch { return []; }
}

export function saveLogs(logs: AnalysisLogEntry[]) {
  const trimmed = logs.slice(-MAX_LOGS);
  localStorage.setItem(LOGS_KEY, JSON.stringify(trimmed));
}

export function loadLastAnalysis(): LastAnalysis | null {
  try {
    const raw = localStorage.getItem(LAST_KEY);
    return raw ? (JSON.parse(raw) as LastAnalysis) : null;
  } catch { return null; }
}

function saveLastAnalysis(a: LastAnalysis) {
  localStorage.setItem(LAST_KEY, JSON.stringify(a));
}

function loadSchedule(): ScheduleState {
  try {
    return JSON.parse(localStorage.getItem(SCHEDULE_KEY) ?? '{"recent":[]}') as ScheduleState;
  } catch { return { recent: [] }; }
}

function saveSchedule(s: ScheduleState) {
  localStorage.setItem(SCHEDULE_KEY, JSON.stringify(s));
}

/** Seconds elapsed since the last recorded analysis. Returns Infinity if none. */
function secondsSinceLastAnalysis(): number {
  const s = loadSchedule();
  if (!s.recent.length) return Infinity;
  const lastTs = new Date(s.recent[s.recent.length - 1]).getTime();
  return (Date.now() - lastTs) / 1000;
}

/** Interval (seconds) between analyses given a daily_analyses count. */
function analysisIntervalSec(daily: number): number {
  return Math.floor((24 * 3600) / Math.max(1, daily));
}

// ── Hook ──────────────────────────────────────────────────────────────────────

export function useAnalysisLogger(hw: Hardware, ai: Settings) {
  // Keep current hw + settings in refs so effects never need re-registering
  const hwRef  = useRef(hw);
  const aiRef  = useRef(ai);
  hwRef.current = hw;
  aiRef.current = ai;

  // ── Collect snapshot callback ─────────────────────────────────────────────
  const collectSnapshot = useCallback(async () => {
    const h = hwRef.current;
    const a = aiRef.current;
    if (!a.settings.ai_analysis_enabled) return;

    const fan = h.fan;
    const sys = h.systemInfo;
    const bat = h.battery;

    let procs: Array<{ name: string; cpu_pct: number; memory_mb: number }> = [];
    try {
      const list = await h.getProcessList();
      procs = list
        .sort((x, y) => y.cpu_percent - x.cpu_percent)
        .slice(0, 10)
        .map((p) => ({ name: p.name, cpu_pct: p.cpu_percent, memory_mb: p.memory_mb }));
    } catch { /* non-fatal */ }

    const entry: AnalysisLogEntry = {
      ts: new Date().toISOString(),
      mode: h.performanceMode ?? "unknown",
      cpu_temp: fan?.cpu_temp_celsius ?? 0,
      gpu_temp: fan?.gpu_temp_celsius ?? 0,
      tdp_watts: fan?.tdp_watts ?? null,
      cpu_pct: sys?.cpu_usage ?? 0,
      gpu_pct: sys?.gpu_usage ?? 0,
      battery_level: bat?.level ?? null,
      is_charging: bat?.is_charging ?? false,
      top_processes: procs,
    };

    const existing = loadLogs();
    saveLogs([...existing, entry]);
  }, []);

  // ── Maybe trigger AI analysis ─────────────────────────────────────────────
  const maybeAnalyze = useCallback(async () => {
    const a = aiRef.current;
    const h = hwRef.current;
    if (!a.settings.ai_analysis_enabled || !a.isConfigured) return;

    const intervalSec = analysisIntervalSec(a.settings.ai_daily_analyses);
    if (secondsSinceLastAnalysis() < intervalSec) return;

    const logs = loadLogs();
    if (logs.length < 2) return; // not enough data yet

    // Detect current UI language
    let language = "en";
    try {
      language = (localStorage.getItem("micontrol_lang") ?? "en");
    } catch { /* ignore */ }

    try {
      const text = await a.analyzeWithLogs(logs, {
        deviceModel: h.hardwareProfile?.device_model ?? null,
        systemInfo: h.systemInfo,
        battery: h.battery,
        performanceMode: h.performanceMode,
        fan: h.fan,
        display: h.display,
        capabilities: h.hardwareProfile?.capabilities ?? null,
      }, language);

      const result: LastAnalysis = { ts: new Date().toISOString(), text, log_count: logs.length };
      saveLastAnalysis(result);

      const sched = loadSchedule();
      sched.recent = [...sched.recent.slice(-23), result.ts]; // keep last 24
      saveSchedule(sched);
    } catch (e) {
      console.warn("[useAnalysisLogger] AI analysis failed:", e);
    }
  }, []);

  // ── Polling effect ────────────────────────────────────────────────────────
  useEffect(() => {
    if (!aiRef.current.settings.ai_analysis_enabled) return;

    const intervalMs = (aiRef.current.settings.ai_poll_interval_sec ?? 60) * 1000;

    // Collect immediately then on interval
    void collectSnapshot();
    void maybeAnalyze();

    const id = setInterval(() => {
      void collectSnapshot();
      void maybeAnalyze();
    }, intervalMs);

    return () => clearInterval(id);
    // Re-register when enabled/interval setting changes:
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    aiRef.current.settings.ai_analysis_enabled,
    aiRef.current.settings.ai_poll_interval_sec,
    collectSnapshot,
    maybeAnalyze,
  ]);
}
