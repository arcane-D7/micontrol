import { t } from "../hooks/useI18n";
import type { FanInfo, SystemInfo, PerformanceMode, PerformanceResult } from "../hooks/useHardware";

interface Props {
  fan: FanInfo | null;
  systemInfo: SystemInfo | null;
  currentMode: PerformanceMode;
  lastResult: PerformanceResult | null;
}

const METHOD_LABELS: Record<string, string> = {
  "hq_wmi+registry+overlay": "HQ WMI (TDP direto)",
  "vhf+registry+overlay": "VHF IOCTL",
  "registry+overlay": "Registro + Overlay",
};

function tempColor(temp: number): string {
  if (temp >= 90) return "var(--error)";
  if (temp >= 75) return "var(--warning)";
  return "var(--success)";
}

function tdpColor(w: number): string {
  if (w >= 35) return "var(--error)";
  if (w >= 20) return "var(--warning)";
  return "var(--info)";
}

// Temperature bar: 40°C = 0%, 100°C = 100%
const tempPct = (t: number) => Math.min(Math.max(((t - 40) / 60) * 100, 0), 100);

interface MetricProps {
  label: string;
  value: React.ReactNode;
  sub?: React.ReactNode;
  bar?: { pct: number; cls: string };
}

function Metric({ label, value, sub, bar }: MetricProps) {
  return (
    <div>
      <div style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 6 }}>{label}</div>
      <div style={{ fontSize: 24, fontWeight: 700, fontFamily: "var(--font-mono)", lineHeight: 1 }}>
        {value}
      </div>
      {sub && <div style={{ fontSize: 11, color: "var(--text-muted)", marginTop: 4 }}>{sub}</div>}
      {bar && (
        <div className="progress-bar" style={{ marginTop: 8 }}>
          <div
            className={`progress-fill ${bar.cls}`}
            style={{ width: `${bar.pct}%`, transition: "width 600ms ease" }}
          />
        </div>
      )}
    </div>
  );
}

export default function PerformanceMonitor({ fan, systemInfo, lastResult }: Props) {
  if (!fan && !systemInfo) return null;

  const cpuTemp = fan?.cpu_temp_celsius ?? 0;
  const gpuTemp = fan?.gpu_temp_celsius ?? 0;
  const fanRpm = fan?.speed_rpm ?? 0;
  const tdp = fan?.tdp_watts ?? null;
  // Use the accurate Win32_PerfFormattedData value from SystemInfo (matches Task Manager)
  const cpuLoad = systemInfo?.cpu_usage ?? null;

  return (
    <div className="card">
      <div className="card-title">{t("performance.monitor.title")}</div>

      <div style={{ display: "grid", gridTemplateColumns: "repeat(5, 1fr)", gap: "10px 14px", marginBottom: lastResult ? 16 : 0 }}>
        {/* CPU Temp */}
        <Metric
          label={t("performance.monitor.cpuTemp")}
          value={<span style={{ color: tempColor(cpuTemp) }}>{cpuTemp.toFixed(0)}°C</span>}
          bar={{ pct: tempPct(cpuTemp), cls: "temp" }}
        />

        {/* GPU Temp */}
        <Metric
          label={t("performance.monitor.gpuTemp")}
          value={<span style={{ color: tempColor(gpuTemp) }}>{gpuTemp.toFixed(0)}°C</span>}
          bar={{ pct: tempPct(gpuTemp), cls: "gpu" }}
        />

        {/* TDP */}
        <Metric
          label="TDP"
          value={
            tdp !== null
              ? <span style={{ color: tdpColor(tdp) }}>{tdp.toFixed(1)} W</span>
              : <span style={{ color: "var(--text-dim)", fontSize: 16 }}>—</span>
          }
          sub={tdp !== null ? "RAPL" : "aguardando..."}
          bar={tdp !== null ? { pct: Math.min((tdp / 45) * 100, 100), cls: "ram" } : undefined}
        />

        {/* Fan */}
        <Metric
          label={t("performance.monitor.fan")}
          value={
            fanRpm > 0
              ? <span style={{ color: "var(--text)" }}>{fanRpm.toLocaleString()}</span>
              : <span style={{ color: "var(--text-dim)", fontSize: 16 }}>N/A</span>
          }
          sub={fanRpm > 0 ? t("performance.monitor.rpm") : "sem sensor WMI"}
        />

        {/* CPU Load */}
        <Metric
          label={t("performance.monitor.cpuLoad")}
          value={
            cpuLoad !== null
              ? <span style={{ color: "var(--info)" }}>{cpuLoad.toFixed(0)}%</span>
              : <span style={{ color: "var(--text-dim)", fontSize: 16 }}>—</span>
          }
          bar={cpuLoad !== null ? { pct: cpuLoad, cls: "cpu" } : undefined}
        />
      </div>

      {/* Activation method confirmation */}
      {lastResult && (
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 10,
            padding: "10px 14px",
            background: "var(--surface-2)",
            borderRadius: "var(--r-sm)",
            marginTop: 16,
            borderLeft: `3px solid ${lastResult.success ? "var(--success)" : "var(--error)"}`,
          }}
        >
          <span className={`badge ${lastResult.success ? "success" : "error"}`}>
            {lastResult.success
              ? `✓ ${t("performance.monitor.modeActive")}`
              : `✗ ${t("performance.monitor.failed")}`}
          </span>
          <span style={{ fontSize: 12, color: "var(--text-muted)" }}>
            {t("performance.monitor.via")}{" "}
            <span style={{ color: "var(--text)", fontWeight: 500 }}>
              {METHOD_LABELS[lastResult.method] ?? lastResult.method}
            </span>
          </span>
        </div>
      )}
    </div>
  );
}
