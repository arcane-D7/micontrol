/**
 * AI prompt construction logic.
 *
 * Extracted from `src/hooks/useSettings.ts` (S28-004).
 * Pure functions — no React, no side effects.
 */

import type { SystemContext, AnalysisLogEntry } from '../types/settings';

/**
 * Build the hardware-context prompt sent to the AI model for a
 * single-snapshot system analysis.
 */
export function buildPrompt(ctx: SystemContext): string {
  const sys = ctx.systemInfo;
  const bat = ctx.battery;
  const fan = ctx.fan;
  const disp = ctx.display;
  const cap = ctx.capabilities;

  return `You are analyzing a Xiaomi laptop. Provide concise, specific recommendations.

== HARDWARE ==
Device: ${ctx.deviceModel ?? 'Unknown'}
CPU: ${sys?.cpu_name ?? 'Unknown'} — usage: ${sys?.cpu_usage?.toFixed(0) ?? '?'}%
RAM: ${sys?.ram_used_gb?.toFixed(1) ?? '?'} / ${sys?.ram_total_gb ?? '?'} GB used
OS: ${sys?.os_version ?? 'Unknown'}

== BATTERY ==
Level: ${bat?.level ?? '?'}%  |  Charging: ${bat?.is_charging ? 'yes' : 'no'}
Health: ${bat?.health_percent ?? '?'}%
Temperature: ${bat?.temperature_celsius != null ? bat.temperature_celsius + '°C' : 'unavailable'}
Capacity: ${bat?.full_capacity_mwh ?? '?'} mWh (designed: ${bat?.designed_capacity_mwh ?? '?'} mWh)

== PERFORMANCE ==
Current mode: ${ctx.performanceMode ?? 'unknown'}
Fan: ${fan?.mode ?? '?'} — ${fan?.speed_rpm ?? '?'}rpm  |  GPU temp: ${fan?.gpu_temp_celsius ?? '?'}°C
Display: brightness ${disp?.brightness ?? '?'}%  |  refresh ${disp?.refresh_rate_hz ?? '?'}Hz  |  HDR: ${disp?.hdr_enabled ?? false}

== HARDWARE CAPABILITIES ==
VHF performance control: ${cap?.has_vhf_performance ? '✓' : '✗ (registry fallback)'}
IoT charging service: ${cap?.has_iot_charging ? '✓' : '✗ (registry fallback)'}
Intel IGCL display: ${cap?.has_igcl ? '✓' : '✗'}
Touchpad HID channel: ${cap?.has_touchpad_hid ? '✓' : '✗'}
Touchscreen: ${cap?.has_touchscreen ? '✓' : '✗'}
Stylus: ${cap?.has_stylus ? '✓' : '✗'}

== REQUESTED ANALYSIS ==
1. Battery health assessment — is the health/cycle count concerning? Recommend optimal charging threshold (values: 60, 70, 80, 100).
2. Performance mode recommendation — is the current mode suitable for the measured CPU/GPU load?
3. Thermal assessment — is the GPU temperature healthy?
4. Any issues or warnings detected from the capability flags.
5. Two specific optimisation tips for this device profile.

Be concise. Use bullet points.`.trim();
}

/**
 * Build the log-based analysis prompt sent to the AI model.
 *
 * Computes statistics from the log entries and formats them into a
 * structured prompt that asks for thermal, performance, battery, and
 * process analysis.
 */
export function buildLogPrompt(
  logs: AnalysisLogEntry[],
  hwCtx: SystemContext,
  language: string,
): string {
  const langNames: Record<string, string> = {
    en: 'English',
    pt: 'Portuguese',
    es: 'Spanish',
    fr: 'French',
  };
  const langName = langNames[language] ?? 'English';

  // Compute statistics from logs
  const n = logs.length;
  const avg = (arr: number[]) => arr.reduce((a, b) => a + b, 0) / arr.length;
  const max = (arr: number[]) => Math.max(...arr);

  const cpuTemps = logs.map((l) => l.cpu_temp);
  const gpuTemps = logs.map((l) => l.gpu_temp);
  const tdps = logs.filter((l) => l.tdp_watts != null).map((l) => l.tdp_watts as number);
  const cpuPcts = logs.map((l) => l.cpu_pct);
  const gpuPcts = logs.map((l) => l.gpu_pct);
  const batLevels = logs
    .filter((l) => l.battery_level != null)
    .map((l) => l.battery_level as number);

  const first = logs[0];
  const last = logs[n - 1];
  const spanMin = Math.round((new Date(last.ts).getTime() - new Date(first.ts).getTime()) / 60000);

  // Top processes from last snapshot
  const topProcs = (last.top_processes ?? [])
    .sort((a, b) => b.cpu_pct - a.cpu_pct)
    .slice(0, 6)
    .map((p) => `  - ${p.name}: ${p.cpu_pct.toFixed(1)}% CPU, ${p.memory_mb.toFixed(0)} MB RAM`)
    .join('\n');

  const batterySection =
    batLevels.length > 1
      ? `**Battery:** ${batLevels[0].toFixed(0)}% → ${batLevels[batLevels.length - 1].toFixed(0)}% (${last.is_charging ? 'charging' : 'discharging'})`
      : '';

  return `Respond in ${langName}.

You are a hardware optimization assistant for a Xiaomi laptop. Analyze the following performance data.

## Performance Log Summary (${n} snapshots over ${spanMin} min)

**CPU Temperature:** avg ${avg(cpuTemps).toFixed(1)}°C, peak ${max(cpuTemps).toFixed(1)}°C
**GPU Temperature:** avg ${avg(gpuTemps).toFixed(1)}°C, peak ${max(gpuTemps).toFixed(1)}°C
**TDP (Package Power):** ${tdps.length ? `avg ${avg(tdps).toFixed(1)} W, peak ${max(tdps).toFixed(1)} W` : 'unavailable'}
**CPU Usage:** avg ${avg(cpuPcts).toFixed(1)}%, peak ${max(cpuPcts).toFixed(1)}%
**GPU Usage:** avg ${avg(gpuPcts).toFixed(1)}%, peak ${max(gpuPcts).toFixed(1)}%
${batterySection}
**Performance Mode:** ${last.mode}

**Top Processes (latest snapshot):**
${topProcs || '  - No process data available'}

**Current System:**
- Device: ${hwCtx.deviceModel ?? 'Xiaomi Laptop'}
- CPU: ${hwCtx.systemInfo?.cpu_name ?? 'Unknown'} (${hwCtx.systemInfo?.cpu_cores ?? '?'} cores)
- RAM: ${hwCtx.systemInfo?.ram_used_gb?.toFixed(1) ?? '?'} / ${hwCtx.systemInfo?.ram_total_gb ?? '?'} GB used

## Analysis Tasks
1. **Thermal:** Are temperatures healthy? Any throttling risk?
2. **Performance:** Is the current mode optimal for the observed workload?
3. **Battery:** Is consumption normal? Any drain concerns?
4. **Top Processes:** Any resource-heavy process worth investigating?
5. **Recommendation:** Best performance mode for this usage pattern?

Be concise. Use short paragraphs with emoji section headers. Max 300 words.`;
}
