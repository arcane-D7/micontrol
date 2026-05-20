import { useState } from "react";
import { t } from "../hooks/useI18n";
import type { BatteryInfo as BatteryData } from "../hooks/useHardware";
import InfoModal, { InfoRow, InfoSection } from "./InfoModal";

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

/** mWh → Wh string */
function mwhToWh(mwh: number): string {
  return (mwh / 1000).toFixed(1);
}

/** mWh + voltage_mv → estimated mAh */
function mwhToMah(mwh: number, voltage_mv: number): string | null {
  if (voltage_mv <= 0) return null;
  return Math.round((mwh * 1000) / voltage_mv).toLocaleString();
}

export default function BatteryInfo({ battery }: Props) {
  const [showInfo, setShowInfo] = useState(false);

  if (!battery) {
    return (
      <div className="card">
        <div className="card-title">{t("battery.title")}</div>
        <div className="skeleton" style={{ height: 120 }} />
      </div>
    );
  }

  const statusKey = battery.is_charging ? "charging" : battery.is_plugged ? "full" : "discharging";
  const designedMah = mwhToMah(battery.designed_capacity_mwh, battery.voltage_mv);
  const fullMah = mwhToMah(battery.full_capacity_mwh, battery.voltage_mv);

  return (
    <div className="card">
      {/* Card header with info button */}
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 16 }}>
        <div className="card-title" style={{ margin: 0 }}>{t("battery.title")}</div>
        <button
          onClick={() => setShowInfo(true)}
          title={t("battery.infoModal.title")}
          style={{
            background: "none", border: "none", cursor: "pointer",
            color: "var(--text-dim)", fontSize: 16, lineHeight: 1, padding: "2px 4px",
            borderRadius: "var(--r-xs)", transition: "color var(--t-fast)",
          }}
          onMouseEnter={(e) => (e.currentTarget.style.color = "var(--text)")}
          onMouseLeave={(e) => (e.currentTarget.style.color = "var(--text-dim)")}
        >
          ⓘ
        </button>
      </div>

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
        <span className="stat-value">
          {mwhToWh(battery.designed_capacity_mwh)} {t("battery.wh")}
          {designedMah && (
            <span style={{ color: "var(--text-muted)", fontSize: 11, marginLeft: 5 }}>
              (≈{designedMah} {t("battery.mah")})
            </span>
          )}
        </span>
      </div>
      <div className="stat-row">
        <span className="stat-label">{t("battery.fullCapacity")}</span>
        <span className="stat-value">
          {mwhToWh(battery.full_capacity_mwh)} {t("battery.wh")}
          {fullMah && (
            <span style={{ color: "var(--text-muted)", fontSize: 11, marginLeft: 5 }}>
              (≈{fullMah} {t("battery.mah")})
            </span>
          )}
        </span>
      </div>
      <div className="stat-row">
        <span className="stat-label">{t("battery.cycleCount")}</span>
        <span className="stat-value">{battery.cycle_count}</span>
      </div>
      {battery.voltage_mv > 0 && (
        <div className="stat-row">
          <span className="stat-label">{t("battery.voltage")}</span>
          <span className="stat-value" style={{ fontFamily: "var(--font-mono)" }}>
            {(battery.voltage_mv / 1000).toFixed(2)} {t("battery.voltageUnit")}
          </span>
        </div>
      )}
      {battery.temperature_celsius != null && (
        <div className="stat-row">
          <span className="stat-label">{t("battery.temperature")}</span>
          <span className="stat-value">{battery.temperature_celsius.toFixed(1)} {t("battery.celsius")}</span>
        </div>
      )}
      {battery.is_charging && battery.charge_rate_mw > 0 && (
        <div className="stat-row">
          <span className="stat-label">{t("battery.chargeRate")}</span>
          <span className="stat-value" style={{ color: "var(--success, #4caf50)", fontWeight: 600 }}>
            ⚡ {(battery.charge_rate_mw / 1000).toFixed(1)} W
          </span>
        </div>
      )}
      {!battery.is_charging && !battery.is_plugged && battery.charge_rate_mw < 0 && (
        <div className="stat-row">
          <span className="stat-label">{t("battery.dischargeRate")}</span>
          <span className="stat-value" style={{ fontFamily: "var(--font-mono)" }}>
            {(Math.abs(battery.charge_rate_mw) / 1000).toFixed(1)} W
          </span>
        </div>
      )}
      {!battery.is_charging && battery.time_remaining_minutes != null && (
        <div className="stat-row">
          <span className="stat-label">{t("battery.timeRemaining")}</span>
          <span className="stat-value">{formatTime(battery.time_remaining_minutes)}</span>
        </div>
      )}

      {/* Info modal */}
      <InfoModal open={showInfo} onClose={() => setShowInfo(false)} title={t("battery.infoModal.title")}>
        <InfoRow label={t("battery.infoModal.levelLabel")}>
          {t("battery.infoModal.levelDesc")}
        </InfoRow>
        <InfoRow label={t("battery.infoModal.healthLabel")}>
          {t("battery.infoModal.healthDesc")}
        </InfoRow>
        <InfoSection title={t("battery.infoModal.capacitySection")}>
          <InfoRow label={t("battery.infoModal.capacityLabel")}>
            {t("battery.infoModal.capacityDesc")}
          </InfoRow>
          <InfoRow label={t("battery.infoModal.cyclesLabel")}>
            {t("battery.infoModal.cyclesDesc")}
          </InfoRow>
        </InfoSection>
        <InfoSection title={t("battery.infoModal.powerSection")}>
          <InfoRow label={t("battery.infoModal.chargeRateLabel")}>
            {t("battery.infoModal.chargeRateDesc")}
          </InfoRow>
          <InfoRow label={t("battery.infoModal.timeLabel")}>
            {t("battery.infoModal.timeDesc")}
          </InfoRow>
        </InfoSection>
      </InfoModal>
    </div>
  );
}

