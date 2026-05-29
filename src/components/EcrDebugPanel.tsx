import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useToast } from "../contexts/ToastContext";

export default function EcrDebugPanel() {
  const [hexData, setHexData] = useState("");
  const [address, setAddress] = useState("");
  const [count, setCount] = useState("32");
  const [result, setResult] = useState("");
  const [loading, setLoading] = useState(false);
  const { addToast } = useToast();

  const handleRead = async () => {
    setLoading(true);
    try {
      const data = await invoke<string>("read_ecram_raw", {
        address: address || "0x0",
        count: parseInt(count) || 32,
      });
      setResult(data);
    } catch (e) {
      addToast(`Read error: ${String(e)}`, "error");
    } finally {
      setLoading(false);
    }
  };

  const handleWrite = async () => {
    if (!address || !hexData) return;
    setLoading(true);
    try {
      await invoke("write_iot_hex", {
        address,
        hexData,
      });
      addToast("Write successful", "success");
    } catch (e) {
      addToast(`Write error: ${String(e)}`, "error");
    } finally {
      setLoading(false);
    }
  };

  const handleReadMap = async () => {
    setLoading(true);
    try {
      const map = await invoke<string>("get_ecram_map");
      setResult(JSON.stringify(map, null, 2));
    } catch (e) {
      addToast(`Map error: ${String(e)}`, "error");
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="card">
      <div className="card-title">🔧 EC Debug Panel</div>
      <p className="page-subtitle">Direct EC RAM access (advanced)</p>

      {/* Read */}
      <div style={{ marginTop: 12 }}>
        <div style={{ fontWeight: 600, marginBottom: 8, fontSize: 13, color: "var(--text-dim)" }}>
          Read ECRAM
        </div>
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          <input
            type="text"
            placeholder="Address (hex)"
            value={address}
            onChange={(e) => setAddress(e.target.value)}
            style={{ flex: 1, minWidth: 120, padding: "6px 8px", borderRadius: "var(--r-xs)", border: "1px solid var(--border)", background: "var(--bg)", color: "var(--text)" }}
          />
          <input
            type="number"
            placeholder="Count"
            value={count}
            onChange={(e) => setCount(e.target.value)}
            min={1}
            max={256}
            style={{ width: 80, padding: "6px 8px", borderRadius: "var(--r-xs)", border: "1px solid var(--border)", background: "var(--bg)", color: "var(--text)" }}
          />
          <button className="btn btn-primary" onClick={handleRead} disabled={loading}>
            Read
          </button>
        </div>
      </div>

      {/* Write */}
      <div style={{ marginTop: 12 }}>
        <div style={{ fontWeight: 600, marginBottom: 8, fontSize: 13, color: "var(--text-dim)" }}>
          Write ECRAM
        </div>
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          <input
            type="text"
            placeholder="Address (hex)"
            value={address}
            onChange={(e) => setAddress(e.target.value)}
            style={{ flex: 1, minWidth: 120, padding: "6px 8px", borderRadius: "var(--r-xs)", border: "1px solid var(--border)", background: "var(--bg)", color: "var(--text)" }}
          />
          <input
            type="text"
            placeholder="Hex data"
            value={hexData}
            onChange={(e) => setHexData(e.target.value)}
            style={{ flex: 2, minWidth: 160, padding: "6px 8px", borderRadius: "var(--r-xs)", border: "1px solid var(--border)", background: "var(--bg)", color: "var(--text)" }}
          />
          <button className="btn btn-primary" onClick={handleWrite} disabled={loading}>
            Write
          </button>
        </div>
      </div>

      {/* Read Map */}
      <button
        className="btn btn-secondary"
        onClick={handleReadMap}
        disabled={loading}
        style={{ marginTop: 12, width: "100%" }}
      >
        📋 Read ECRAM Map
      </button>

      {/* Result */}
      {result && (
        <div style={{ marginTop: 12 }}>
          <div style={{ fontWeight: 600, marginBottom: 8, fontSize: 13, color: "var(--text-dim)" }}>
            Result
          </div>
          <pre style={{
            padding: 12,
            background: "var(--bg-hover)",
            borderRadius: "var(--r-xs)",
            fontSize: 12,
            maxHeight: 300,
            overflow: "auto",
            whiteSpace: "pre-wrap",
            wordBreak: "break-all",
          }}>
            {result}
          </pre>
        </div>
      )}
    </div>
  );
}