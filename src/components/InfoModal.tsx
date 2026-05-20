import { createPortal } from "react-dom";

interface Props {
  open: boolean;
  onClose: () => void;
  title: string;
  children: React.ReactNode;
}

/** Generic info modal rendered via portal. Click backdrop or × to close. */
export default function InfoModal({ open, onClose, title, children }: Props) {
  if (!open) return null;

  return createPortal(
    <div
      style={{
        position: "fixed", inset: 0, zIndex: 9999,
        background: "oklch(0% 0 0 / 0.5)", backdropFilter: "blur(6px)",
        display: "flex", alignItems: "center", justifyContent: "center",
        padding: "0 20px",
      }}
      onClick={onClose}
    >
      <div
        style={{
          background: "var(--surface)",
          border: "1px solid var(--border)",
          borderRadius: "var(--r-md)",
          padding: "22px 24px",
          maxWidth: 520,
          width: "100%",
          maxHeight: "80vh",
          overflow: "auto",
          boxShadow: "var(--shadow-glass)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 18 }}>
          <div style={{ fontWeight: 600, fontSize: 14, color: "var(--text)" }}>{title}</div>
          <button
            onClick={onClose}
            style={{
              background: "none", border: "none", cursor: "pointer",
              color: "var(--text-muted)", fontSize: 20, lineHeight: 1,
              padding: "0 4px", display: "flex", alignItems: "center",
            }}
          >
            ×
          </button>
        </div>

        {/* Content */}
        {children}
      </div>
    </div>,
    document.body,
  );
}

/** A single glossary row inside InfoModal. */
export function InfoRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div style={{ marginBottom: 16 }}>
      <div style={{ fontSize: 12, fontWeight: 600, color: "var(--text)", marginBottom: 4 }}>{label}</div>
      <div style={{ fontSize: 12, color: "var(--text-muted)", lineHeight: 1.6 }}>{children}</div>
    </div>
  );
}

/** A section divider with optional title used inside InfoModal. */
export function InfoSection({ title, children }: { title?: string; children: React.ReactNode }) {
  return (
    <div style={{ borderTop: "1px solid var(--border)", paddingTop: 14, marginTop: 4, marginBottom: 4 }}>
      {title && (
        <div style={{
          fontSize: 10.5, fontWeight: 600, textTransform: "uppercase",
          letterSpacing: "0.08em", color: "var(--text-dim)", marginBottom: 10,
        }}>
          {title}
        </div>
      )}
      {children}
    </div>
  );
}
