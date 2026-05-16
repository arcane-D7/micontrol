import { t } from "../hooks/useI18n";
import type { BatteryInfo as BatteryData } from "../hooks/useHardware";

interface Props {
  battery: BatteryData | null;
}

function formatTime(minutes: number | null): string {
  if (minutes == null || minutes < 0) return t("common.unknown");
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return h > 0 ? `${h}${t("battery.hours")} ${m}${t("battery.minutes")}` : `${m}${t("battery.minutes")}`;
}

function healthColor(pct: number): string {
  if (pct >= 80) return "success";
  if (pct >= 60) return "warning";
  return "error";
}

function batteryColor(level: number): string {
  if (level > 30) return "battery";
  if (level > 15) return "battery warning";
  return "battery critical";
}

export default function BatteryInfo({ battery }: Props) {
  if (!battery) {
    return (
      <div className="card">
        <div className="card-title">{t("battery.title")}</div>
        <div className="skeleton" style={{ height: 120 }} />
      </div>
    );
  }

  const statusKey = battery.is_charging ? "charging" : battery.is_plugged ? "full" : "discharging";

  return (
    <div className="card">
      <div className="card-title">{t("battery.title")}</div>

      <div className="grid-2" style={{ marginBottom: 16 }}>
        <div>
          <div className="card-value">{battery.level}%</div>
          <div className="card-subtitle">
            <span className={`badge ${battery.is_charging ? "success" : battery.is_plugged ? "info" : "warning"}`}>
              {t(`battery.${statusKey}` as Parameters<typeof t>[0])}
            </span>
          </div>
          <div className="progress-bar" style={{ marginTop: 12 }}>
            <div
              className={`progress-fill ${batteryColor(battery.level)}`}
              style={{ width: `${battery.level}%` }}
            />
          </div>
        </div>
        <div>
          <div className="card-value">
            {battery.health_percent.toFixed(1)}%
          </div>
          <div className="card-subtitle">
            <span className={`badge ${healthColor(battery.health_percent)}`}>
              {t("battery.health")}
            </span>
          </div>
        </div>
      </div>

      <div className="stat-row">
        <span className="stat-label">{t("battery.manufacturer")}</span>
        <span className="stat-value">{battery.manufacturer}</span>
      </div>
      <div className="stat-row">
        <span className="stat-label">{t("battery.designedCapacity")}</span>
        <span className="stat-value">{battery.designed_capacity_mah.toLocaleString()} {t("battery.mah")}</span>
      </div>
      <div className="stat-row">
        <span className="stat-label">{t("battery.fullCapacity")}</span>
        <span className="stat-value">{battery.full_capacity_mah.toLocaleString()} {t("battery.mah")}</span>
      </div>
      <div className="stat-row">
        <span className="stat-label">{t("battery.cycleCount")}</span>
        <span className="stat-value">{battery.cycle_count}</span>
      </div>
      {battery.temperature_celsius != null && (
        <div className="stat-row">
          <span className="stat-label">{t("battery.temperature")}</span>
          <span className="stat-value">{battery.temperature_celsius.toFixed(1)} {t("battery.celsius")}</span>
        </div>
      )}
      {!battery.is_charging && battery.time_remaining_minutes != null && (
        <div className="stat-row">
          <span className="stat-label">{t("battery.timeRemaining")}</span>
          <span className="stat-value">{formatTime(battery.time_remaining_minutes)}</span>
        </div>
      )}
    </div>
  );
}
