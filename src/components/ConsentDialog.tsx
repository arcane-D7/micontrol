import { useCallback, useRef, useEffect } from 'react';
import { t } from '../hooks/useI18n';

interface ConsentDialogProps {
  onAllow: () => void;
  onDeny: () => void;
  onOpenPrivacy: () => void;
}

export function ConsentDialog({ onAllow, onDeny, onOpenPrivacy }: ConsentDialogProps) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const allowRef = useRef<HTMLButtonElement>(null);
  const denyRef = useRef<HTMLButtonElement>(null);

  // Focus the dialog container on mount (neutral element, not the Allow button)
  // This avoids the dark pattern of auto-focusing the "Allow" button.
  useEffect(() => {
    dialogRef.current?.focus();
  }, []);

  // Focus trap: cycle Tab within the dialog only
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onDeny();
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
    [onDeny],
  );

  const handlePrivacyClick = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      onOpenPrivacy();
    },
    [onOpenPrivacy],
  );

  return (
    <div
      className="consent-overlay"
      role="dialog"
      aria-modal="true"
      aria-labelledby="consent-title"
      onKeyDown={handleKeyDown}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 9999,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        background: 'rgba(0,0,0,0.55)',
        backdropFilter: 'blur(4px)',
      }}
    >
      <div
        className="consent-dialog"
        ref={dialogRef}
        tabIndex={-1}
        style={{
          background: 'var(--color-surface, #1e1e2e)',
          border: '1px solid var(--color-border, #3a3a4e)',
          borderRadius: 14,
          padding: '28px 32px',
          maxWidth: 520,
          width: '90%',
          boxShadow: '0 16px 48px rgba(0,0,0,0.4)',
        }}
      >
        <h2
          id="consent-title"
          style={{
            fontSize: 18,
            fontWeight: 700,
            marginTop: 0,
            marginBottom: 16,
            color: 'var(--color-text)',
          }}
        >
          {t('consent.dialog.title')}
        </h2>

        <p
          style={{
            fontSize: 13,
            lineHeight: 1.6,
            color: 'var(--color-text-muted)',
            marginTop: 0,
            marginBottom: 18,
          }}
        >
          {t('consent.dialog.intro')}
        </p>

        <ul
          style={{
            fontSize: 12.5,
            lineHeight: 1.7,
            paddingLeft: 20,
            marginBottom: 18,
            color: 'var(--color-text)',
          }}
        >
          <li>
            <strong>{t('consent.dialog.what').split(':')[0]}:</strong>{' '}
            {t('consent.dialog.what').split(':').slice(1).join(':')}
          </li>
          <li>
            <strong>{t('consent.dialog.where').split(':')[0]}:</strong>{' '}
            {t('consent.dialog.where').split(':').slice(1).join(':')}
          </li>
          <li>
            <strong>{t('consent.dialog.why').split(':')[0]}:</strong>{' '}
            {t('consent.dialog.why').split(':').slice(1).join(':')}
          </li>
          <li>
            <strong>{t('consent.dialog.control').split(':')[0]}:</strong>{' '}
            {t('consent.dialog.control').split(':').slice(1).join(':')}
          </li>
        </ul>

        <p
          style={{
            fontSize: 12,
            lineHeight: 1.5,
            color: 'var(--color-text-muted)',
            marginBottom: 18,
            padding: '8px 10px',
            background: 'var(--color-surface-alt, rgba(255,255,255,0.03))',
            borderRadius: 8,
            border: '1px solid var(--color-border, #3a3a4e)',
          }}
        >
          {t('consent.dialog.sentryDisclosure')}
        </p>

        <p style={{ fontSize: 12, marginBottom: 20, color: 'var(--color-text-muted)' }}>
          <a
            href="#"
            onClick={handlePrivacyClick}
            style={{
              color: 'var(--color-accent, #6c8cff)',
              textDecoration: 'underline',
            }}
          >
            {t('consent.dialog.privacyLink')}
          </a>
        </p>

        <div style={{ display: 'flex', gap: 12, justifyContent: 'flex-end' }}>
          <button
            type="button"
            onClick={onDeny}
            className="btn-secondary"
            ref={denyRef}
            style={{
              padding: '8px 20px',
              borderRadius: 8,
              fontSize: 13,
              fontWeight: 600,
              cursor: 'pointer',
            }}
          >
            {t('consent.dialog.deny')}
          </button>
          <button
            ref={allowRef}
            type="button"
            onClick={onAllow}
            className="btn-primary"
            style={{
              padding: '8px 20px',
              borderRadius: 8,
              fontSize: 13,
              fontWeight: 600,
              cursor: 'pointer',
            }}
          >
            {t('consent.dialog.allow')}
          </button>
        </div>
      </div>
    </div>
  );
}
