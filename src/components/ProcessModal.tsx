import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { ProcessInfo } from "../hooks/useHardware";

interface Props {
  open: boolean;
  onClose: () => void;
  resource: "cpu" | "gpu" | "ram";
  getProcessList: () => Promise<ProcessInfo[]>;
}

const RESOURCE_LABELS: Record<Props["resource"], string> = {
  cpu: "CPU Usage",
  gpu: "GPU Usage",
  ram: "Memory",
};

export default function ProcessModal({ open, onClose, resource, getProcessList }: Props) {
  const [processes, setProcesses] = useState<ProcessInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const fetchProcesses = async () => {
    try {
      const procs = await getProcessList();
      // Sort by the relevant metric
      const sorted = [...procs].sort((a, b) => {
        if (resource === "ram") return b.memory_mb - a.memory_mb;
        return b.cpu_percent - a.cpu_percent;
      });
      setProcesses(sorted.slice(0, 20));
    } catch (e) {
      console.warn("getProcessList error:", e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (!open) return;
    setLoading(true);
    void fetchProcesses();
    timerRef.current = setInterval(() => void fetchProcesses(), 3000);
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, resource]);

  if (!open) return null;

  return createPortal(
    <div
      style={{
        position: "fixed", inset: 0, zIndex: 9999,
        background: "rgba(0,0,0,0.55)", backdropFilter: "blur(4px)",
        display: "flex", alignItems: "center", justifyContent: "center",
      }}
      onClick={onClose}
    >
      <div
        style={{
          background: "var(--color-surface, #1e1e2e)",
          border: "1px solid var(--color-border, rgba(255,255,255,0.08))",
          borderRadius: 12,
          padding: 20,
          minWidth: 480,
          maxWidth: 600,
          maxHeight: "70vh",
          overflow: "auto",
          boxShadow: "0 24px 64px rgba(0,0,0,0.6)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 16 }}>
          <div style={{ fontWeight: 600, fontSize: 15 }}>
            Top Processes — {RESOURCE_LABELS[resource]}
          </div>
          <button
            onClick={onClose}
            style={{
              background: "none", border: "none", cursor: "pointer",
              color: "var(--color-text-dim)", fontSize: 18, lineHeight: 1,
              padding: "0 4px",
            }}
          >
            ✕
          </button>
        </div>

        {loading ? (
          <div style={{ textAlign: "center", padding: 32, color: "var(--color-text-dim)" }}>
            Loading…
          </div>
        ) : (
          <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 13 }}>
            <thead>
              <tr style={{ borderBottom: "1px solid var(--color-border, rgba(255,255,255,0.08))" }}>
                <th style={{ textAlign: "left", padding: "4px 8px", color: "var(--color-text-dim)", fontWeight: 500 }}>Process</th>
                <th style={{ textAlign: "right", padding: "4px 8px", color: "var(--color-text-dim)", fontWeight: 500 }}>PID</th>
                <th style={{ textAlign: "right", padding: "4px 8px", color: "var(--color-text-dim)", fontWeight: 500 }}>CPU %</th>
                <th style={{ textAlign: "right", padding: "4px 8px", color: "var(--color-text-dim)", fontWeight: 500 }}>RAM MB</th>
              </tr>
            </thead>
            <tbody>
              {processes.map((p) => (
                <tr
                  key={p.pid}
                  style={{
                    borderBottom: "1px solid var(--color-border, rgba(255,255,255,0.04))",
                  }}
                >
                  <td style={{ padding: "5px 8px", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", maxWidth: 220 }}>
                    {p.name}
                  </td>
                  <td style={{ padding: "5px 8px", textAlign: "right", color: "var(--color-text-dim)" }}>{p.pid}</td>
                  <td style={{ padding: "5px 8px", textAlign: "right", fontVariantNumeric: "tabular-nums" }}>
                    {p.cpu_percent > 0 ? `${p.cpu_percent.toFixed(1)}%` : "—"}
                  </td>
                  <td style={{ padding: "5px 8px", textAlign: "right", fontVariantNumeric: "tabular-nums" }}>
                    {p.memory_mb.toFixed(0)}
                  </td>
                </tr>
              ))}
              {processes.length === 0 && (
                <tr>
                  <td colSpan={4} style={{ textAlign: "center", padding: 24, color: "var(--color-text-dim)" }}>
                    No processes found
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        )}

        <div style={{ marginTop: 10, fontSize: 11, color: "var(--color-text-dim)" }}>
          Refreshes every 3 s · Top 20 by {resource === "ram" ? "memory" : "CPU usage"}
        </div>
      </div>
    </div>,
    document.body
  );
}
