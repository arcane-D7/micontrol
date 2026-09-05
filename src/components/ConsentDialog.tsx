import { useCallback, useRef, useEffect, useState } from 'react';
import { t } from '../hooks/useI18n';

interface ConsentDialogProps {
  onAllow: () => void;
  onDeny: () => void;
  onOpenPrivacy: () => void;
}

/**
 * S49 — Crash-report consent modal (full refactor).
 *
 * Goals vs the old dialog:
 * - Clear binary choice with EQUAL visual weight (the old version had
 *   "Allow" as the only primary button — a subtle dark pattern).
 * - Benefits stated explicitly (better debugging across many machines,
 *   faster fixes) instead of legal boilerplate.
 * - Scope of data stated in plain words + link to the full policy.
 * - Neutral choice persisted: "Not now" behaves exactly like "No" — the
 *   user is never nagged; the choice lives in Settings.
 */
export function ConsentDialog({ onAllow, onDeny, onOpenPrivacy }: ConsentDialogProps) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const [leaving, setLeaving] = useState(false);

  // Focus the dialog container on mount (neutral element — never auto-focus
  // either choice button; that would bias the decision).
  useEffect(() => {
    dialogRef.current?.focus();
  }, []);

  const finish = useCallback((choose: () => void) => {
    // Small fade-out so the choice feels acknowledged, then commit.
    setLeaving(true);
    window.setTimeout(choose, 140);
  }, []);

  const handleAllow = useCallback(() => finish(onAllow), [finish, onAllow]);
  const handleDeny = useCallback(() => finish(onDeny), [finish, onDeny]);

  // Focus trap: cycle Tab within the dialog only; Escape = "Not now" (deny).
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        handleDeny();
        return;
      }
      if (e.key !== 'Tab') return;

      const focusable = dialogRef.current?.querySelectorAll<HTMLElement>(
        'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
      );
      if (!focusable || focusable.length === 0) return;

      const first = focusable[0];
      const last = focusable[focusable.length - 1];

      if (e.shiftKey) {
        if (document.activeElement === first) {
          e.preventDefault();
          last.focus();
        }
      } else {
        if (document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      }
    },
    [handleDeny],
  );

  const handlePrivacyClick = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      onOpenPrivacy();
    },
    [onOpenPrivacy],
  );

  const benefits = [
    'consent.dialog.benefit1',
    'consent.dialog.benefit2',
    'consent.dialog.benefit3',
  ] as const;

  return (
    <div
      className="consent-overlay"
      role="dialog"
      aria-modal="true"
      aria-labelledby="consent-title"
      aria-describedby="consent-desc"
      onKeyDown={handleKeyDown}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 9999,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        background: 'rgba(0,0,0,0.6)',
        backdropFilter: 'blur(6px)',
        opacity: leaving ? 0 : 1,
        transition: 'opacity 140ms ease',
      }}
    >
      <div
        className="consent-dialog"
        ref={dialogRef}
        tabIndex={-1}
        role="document"
        style={{
          background: 'var(--surface-solid)',
          border: '1px solid var(--border-strong)',
          borderRadius: 'var(--r-lg, 22px)',
          padding: '30px 34px 26px',
          maxWidth: 560,
          width: '92%',
          maxHeight: '88vh',
          overflowY: 'auto',
          boxShadow: '0 24px 64px rgba(0,0,0,0.45)',
          transform: leaving ? 'scale(0.97)' : 'scale(1)',
          transition: 'transform 140ms ease',
        }}
      >
        {/* Icon + title */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 14 }}>
          <span
            aria-hidden="true"
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              justifyContent: 'center',
              width: 42,
              height: 42,
              borderRadius: 12,
              background: 'var(--accent-soft)',
              fontSize: 20,
              flexShrink: 0,
            }}
          >
            🛡️
          </span>
          <h2
            id="consent-title"
            style={{
              fontSize: 19,
              fontWeight: 700,
              margin: 0,
              color: 'var(--color-text)',
              letterSpacing: '-0.3px',
            }}
          >
            {t('consent.dialog.title')}
          </h2>
        </div>

        <p
          id="consent-desc"
          style={{
            fontSize: 13.5,
            lineHeight: 1.65,
            color: 'var(--color-text-muted)',
            marginTop: 0,
            marginBottom: 18,
          }}
        >
          {t('consent.dialog.intro')}
        </p>

        {/* Benefits — the "why should I?" answered up front */}
        <div
          role="list"
          aria-label={t('consent.dialog.benefitsLabel')}
          style={{ display: 'flex', flexDirection: 'column', gap: 9, marginBottom: 18 }}
        >
          {benefits.map((key) => (
            <div
              key={key}
              role="listitem"
              style={{ display: 'flex', gap: 10, alignItems: 'flex-start' }}
            >
              <span aria-hidden="true" style={{ fontSize: 14, lineHeight: 1.5, flexShrink: 0 }}>
                ✓
              </span>
              <span style={{ fontSize: 13, lineHeight: 1.55, color: 'var(--color-text)' }}>
                {t(key)}
              </span>
            </div>
          ))}
        </div>

        {/* What is sent — plain scope box */}
        <div
          style={{
            fontSize: 12.5,
            lineHeight: 1.6,
            color: 'var(--color-text-muted)',
            marginBottom: 14,
            padding: '12px 14px',
            background: 'var(--color-surface-alt)',
            borderRadius: 10,
            border: '1px solid var(--border)',
          }}
        >
          <strong style={{ color: 'var(--color-text)' }}>{t('consent.dialog.scopeTitle')}</strong>{' '}
          {t('consent.dialog.scopeBody')}
        </div>

        {/* Reassurance + policy link */}
        <p
          style={{
            fontSize: 12,
            lineHeight: 1.55,
            color: 'var(--color-text-muted)',
            marginBottom: 6,
          }}
        >
          🔒 {t('consent.dialog.reassurance')}
        </p>
        <p style={{ fontSize: 12, marginBottom: 22, color: 'var(--color-text-muted)' }}>
          <a
            href="#"
            onClick={handlePrivacyClick}
            style={{
              color: 'var(--color-accent)',
              textDecoration: 'underline',
              textUnderlineOffset: 2,
            }}
          >
            {t('consent.dialog.privacyLink')}
          </a>
          {' · '}
          {t('consent.dialog.changeLater')}
        </p>

        {/* Choices — equal weight, clearly labeled */}
        <div style={{ display: 'flex', gap: 12, flexDirection: 'column' }}>
          <button
            type="button"
            onClick={handleAllow}
            className="btn-primary"
            style={{
              padding: '11px 20px',
              borderRadius: 10,
              fontSize: 13.5,
              fontWeight: 600,
              cursor: 'pointer',
              width: '100%',
            }}
          >
            {t('consent.dialog.allow')}
          </button>
          <button
            type="button"
            onClick={handleDeny}
            className="btn-secondary"
            style={{
              padding: '11px 20px',
              borderRadius: 10,
              fontSize: 13.5,
              fontWeight: 600,
              cursor: 'pointer',
              width: '100%',
            }}
          >
            {t('consent.dialog.deny')}
          </button>
        </div>
      </div>
    </div>
  );
}
