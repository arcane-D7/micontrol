import { useState } from 'react';

interface FunctionExplainProps {
  /** Short inline description shown next to the control. */
  summary: string;
  /** Detailed explanation shown when expanded. */
  details: string;
  /** Optional extra bullet points (each rendered as a list item). */
  bullets?: string[];
  /** Optional "learn more" / behavior note. */
  note?: string;
}

/**
 * Collapsible "how this works" explanation used on hardware controls that are
 * otherwise a single toggle/button. Provides a clear, always-visible one-line
 * summary plus an expandable detailed explanation (what the feature does,
 * what it affects, and when to use it) — improving UX without cluttering the UI.
 */
export default function FunctionExplain({ summary, details, bullets, note }: FunctionExplainProps) {
  const [open, setOpen] = useState(false);

  return (
    <div
      style={{
        marginTop: 8,
        border: '1px solid var(--border, rgba(0,0,0,0.1))',
        borderRadius: 8,
        padding: '8px 12px',
        background: 'var(--surface-2, rgba(0,0,0,0.02))',
      }}
    >
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        style={{
          background: 'none',
          border: 'none',
          padding: 0,
          cursor: 'pointer',
          fontSize: 12,
          color: 'var(--text-muted)',
          textAlign: 'left',
          width: '100%',
          display: 'flex',
          alignItems: 'center',
          gap: 6,
        }}
      >
        <span aria-hidden="true" style={{ display: 'inline-block', transition: 'transform 150ms' }}>
          ▸
        </span>
        <span>{summary}</span>
        <span aria-hidden="true" style={{ marginLeft: 'auto', opacity: 0.6 }}>
          {open ? '−' : '+'}
        </span>
      </button>

      {open && (
        <div style={{ marginTop: 8, fontSize: 12, lineHeight: 1.6, color: 'var(--text)' }}>
          <p style={{ margin: '0 0 6px' }}>{details}</p>
          {bullets && bullets.length > 0 && (
            <ul style={{ margin: '4px 0', paddingLeft: 18 }}>
              {bullets.map((b, i) => (
                <li key={i} style={{ marginBottom: 2 }}>
                  {b}
                </li>
              ))}
            </ul>
          )}
          {note && (
            <p style={{ margin: '6px 0 0', color: 'var(--text-dim)', fontSize: 11 }}>{note}</p>
          )}
        </div>
      )}
    </div>
  );
}
