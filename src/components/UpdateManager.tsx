import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t } from "../hooks/useI18n";
import type { UpdateStatus } from "../hooks/useHardware";

interface Props {
  updateStatus: UpdateStatus | null;
  loadingUpdate: boolean;
  onRefreshUpdate: () => void;
}

export default function UpdateManager({
  updateStatus,
  loadingUpdate,
  onRefreshUpdate,
}: Props) {
  const [scanning, setScanning] = useState(false);
  const [scanMsg, setScanMsg] = useState<string | null>(null);
  const [scanError, setScanError] = useState(false);

  const handleScan = async () => {
    setScanning(true);
    setScanMsg(null);
    setScanError(false);
    try {
      const msg = await invoke<string>("trigger_driver_scan");
      setScanMsg(msg);
      setScanError(false);
      // Refresh status after scan so the LastScanTime updates
      setTimeout(() => {
        onRefreshUpdate();
      }, 1500);
    } catch (e) {
      setScanMsg(t("updates.scanError").replace("{error}", String(e)));
      setScanError(true);
    } finally {
      setScanning(false);
    }
  };

  if (loadingUpdate && !updateStatus) {
    return (
      <div className="card">
        <div className="card-title">{t("updates.title")}</div>
        <div className="loading-spinner">{t("common.loading")}</div>
      </div>
    );
  }

  const bios = updateStatus?.bios;
  const drivers = updateStatus?.xiaomi_drivers ?? [];
  const lastScan = updateStatus?.last_xpm_scan;
  const xpmInstalled = updateStatus?.xpm_installed ?? false;
  const xpmVersion = updateStatus?.xpm_version;

  return (
    <>
      {/* BIOS Information */}
      <div className="card">
        <div className="card-title">{t("updates.biosSection")}</div>
        <div className="stat-grid">
          <div className="stat-item">
            <span className="stat-label">{t("updates.biosVersion")}</span>
            <span className="stat-value">{bios?.version || t("common.unknown")}</span>
          </div>
          <div className="stat-item">
            <span className="stat-label">{t("updates.biosDate")}</span>
            <span className="stat-value">{bios?.release_date || t("common.unknown")}</span>
          </div>
          <div className="stat-item">
            <span className="stat-label">{t("updates.biosMfg")}</span>
            <span className="stat-value">{bios?.manufacturer || t("common.unknown")}</span>
          </div>
        </div>
      </div>

      {/* XPM Nucleus status + scan trigger */}
      <div className="card">
        <div className="card-title">{t("updates.xpmStatus")}</div>

        <div className="stat-grid" style={{ marginBottom: "1rem" }}>
          <div className="stat-item" style={{ gridColumn: "1 / -1" }}>
            <span className="stat-label">{t("updates.xpmStatus")}</span>
            <span
              className="stat-value"
              style={{ color: xpmInstalled ? "var(--accent)" : "var(--text-muted)" }}
            >
              {xpmInstalled
                ? t("updates.xpmInstalled").replace("{version}", xpmVersion ?? "?")
                : t("updates.xpmNotInstalled")}
            </span>
          </div>
          <div className="stat-item">
            <span className="stat-label">{t("updates.lastScan")}</span>
            <span className="stat-value">{lastScan ?? t("updates.never")}</span>
          </div>
        </div>

        <button
          className="btn-primary btn-full"
          onClick={() => void handleScan()}
          disabled={scanning}
        >
          {scanning ? t("updates.scanning") : t("updates.triggerScan")}
        </button>

        {scanMsg && (
          <div
            style={{
              marginTop: 12,
              padding: "8px 12px",
              borderRadius: "var(--r-sm)",
              background: scanError ? "oklch(from var(--error) l c h / 0.10)" : "oklch(from var(--success) l c h / 0.10)",
              color: scanError ? "var(--error)" : "var(--success)",
              fontSize: 12,
              border: `1px solid ${scanError ? "oklch(from var(--error) l c h / 0.20)" : "oklch(from var(--success) l c h / 0.20)"}`,
            }}
          >
            {scanMsg}
          </div>
        )}

        {/* Nucleus explanation note */}
        <p
          style={{
            marginTop: "1rem",
            fontSize: "0.78rem",
            color: "var(--text-muted)",
            lineHeight: 1.5,
          }}
        >
          {t("updates.nucleusNote")}
        </p>
      </div>

      {/* Installed Xiaomi drivers */}
      <div className="card">
        <div className="card-title">{t("updates.driversSection")}</div>

        {drivers.length === 0 ? (
          <p style={{ color: "var(--text-muted)", fontSize: "0.85rem" }}>
            {t("updates.noXiaomiDrivers")}
          </p>
        ) : (
          <div style={{ overflowX: "auto" }}>
            <table
              style={{
                width: "100%",
                borderCollapse: "collapse",
                fontSize: "0.82rem",
              }}
            >
              <thead>
                <tr
                  style={{
                    borderBottom: "1px solid var(--border)",
                    color: "var(--text-muted)",
                  }}
                >
                  <th style={{ textAlign: "left", padding: "0.4rem 0.5rem" }}>
                    {t("updates.driverPublished")}
                  </th>
                  <th style={{ textAlign: "left", padding: "0.4rem 0.5rem" }}>
                    {t("updates.driverOriginal")}
                  </th>
                  <th style={{ textAlign: "left", padding: "0.4rem 0.5rem" }}>
                    {t("updates.driverProvider")}
                  </th>
                  <th style={{ textAlign: "left", padding: "0.4rem 0.5rem" }}>
                    {t("updates.driverVersion")}
                  </th>
                </tr>
              </thead>
              <tbody>
                {drivers.map((d) => (
                  <tr
                    key={d.published_name}
                    style={{ borderBottom: "1px solid rgba(255,255,255,0.05)" }}
                  >
                    <td
                      style={{
                        padding: "0.45rem 0.5rem",
                        color: "var(--accent)",
                        fontFamily: "monospace",
                      }}
                    >
                      {d.published_name}
                    </td>
                    <td
                      style={{
                        padding: "0.45rem 0.5rem",
                        fontFamily: "monospace",
                      }}
                    >
                      {d.original_name}
                    </td>
                    <td style={{ padding: "0.45rem 0.5rem" }}>{d.provider}</td>
                    <td
                      style={{
                        padding: "0.45rem 0.5rem",
                        color: "var(--text-muted)",
                      }}
                    >
                      {d.version_string}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </>
  );
}
