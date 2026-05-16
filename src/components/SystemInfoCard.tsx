import { t } from "../hooks/useI18n";
import type { SystemInfo } from "../hooks/useHardware";

interface Props {
  info: SystemInfo | null;
}

export default function SystemInfoCard({ info }: Props) {
  if (!info) {
    return (
      <div className="card">
        <div className="card-title">{t("overview.system")}</div>
        <div className="skeleton" style={{ height: 120 }} />
      </div>
    );
  }

  const cpuUsageColor = info.cpu_usage > 80 ? "temp" : info.cpu_usage > 50 ? "cpu" : "cpu";
  const ramUsedPct = (info.ram_used_gb / info.ram_total_gb) * 100;

  return (
    <div className="card">
      <div className="card-title">{t("overview.system")}</div>

      <div className="stat-row">
        <span className="stat-label">{t("overview.cpu")}</span>
        <span className="stat-value">{info.cpu_name}</span>
      </div>
      <div style={{ marginBottom: 12 }}>
        <div className="stat-row" style={{ paddingBottom: 4 }}>
          <span className="stat-label">{t("overview.cpuUsage")}</span>
          <span className="stat-value">{info.cpu_usage.toFixed(1)}%</span>
        </div>
        <div className="progress-bar">
          <div className={`progress-fill ${cpuUsageColor}`} style={{ width: `${info.cpu_usage}%` }} />
        </div>
      </div>

      <div className="stat-row">
        <span className="stat-label">{t("overview.gpu")}</span>
        <span className="stat-value">{info.gpu_name}</span>
      </div>

      <div style={{ marginBottom: 4 }}>
        <div className="stat-row" style={{ paddingBottom: 4 }}>
          <span className="stat-label">{t("overview.ram")}</span>
          <span className="stat-value">
            {info.ram_used_gb.toFixed(1)} / {info.ram_total_gb.toFixed(0)} GB
          </span>
        </div>
        <div className="progress-bar">
          <div className="progress-fill ram" style={{ width: `${ramUsedPct}%` }} />
        </div>
      </div>

      <div className="stat-row">
        <span className="stat-label">{t("overview.cores")}</span>
        <span className="stat-value">{info.cpu_cores}C / {info.cpu_threads}T</span>
      </div>
    </div>
  );
}
