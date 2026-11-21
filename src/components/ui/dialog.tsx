import { createPortal } from 'react-dom';
import { useCallback, useEffect, useId, useRef } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from 'react';
import { IconButton } from './button';
import { cx } from './cx';

export interface DialogProps {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  children?: ReactNode;
  footer?: ReactNode;
  size?: 'sm' | 'md' | 'lg';
  closeLabel?: string;
  className?: string;
}

const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * Dialog — véu da própria cor do fundo (sem blur), painel sobe de 6 px,
 * 1 px hairline, raio 12, ZERO sombra.
 *
 * O comportamento (focus trap, Escape, restore de foco, `aria-modal`) é escrito
 * à mão — o hairline também não usa Radix. Substitui os 4 diálogos divergentes
 * (InfoModal, ProcessModal, ConsentDialog, .overpower-modal).
 */
export function Dialog({
  open,
  onClose,
  title,
  children,
  footer,
  size = 'md',
  closeLabel = 'Fechar',
  className,
}: DialogProps) {
  const panelRef = useRef<HTMLDivElement>(null);
  const restoreRef = useRef<HTMLElement | null>(null);
  const titleId = useId();

  useEffect(() => {
    if (!open) return;
    restoreRef.current = document.activeElement as HTMLElement | null;
    const raf = requestAnimationFrame(() => panelRef.current?.focus());
    return () => {
      cancelAnimationFrame(raf);
      restoreRef.current?.focus?.();
    };
  }, [open]);

  const handleKeyDown = useCallback(
    (e: ReactKeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key !== 'Tab') return;

      const nodes = panelRef.current?.querySelectorAll<HTMLElement>(FOCUSABLE);
      if (!nodes || nodes.length === 0) {
        e.preventDefault();
        panelRef.current?.focus();
        return;
      }

      const first = nodes[0];
      const last = nodes[nodes.length - 1];

      if (e.shiftKey) {
        if (document.activeElement === first || document.activeElement === panelRef.current) {
          e.preventDefault();
          last.focus();
        }
      } else if (document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    },
    [onClose],
  );

  if (!open) return null;

  return createPortal(
    <div
      className="mc-dialog-backdrop"
      data-testid="mc-dialog-backdrop"
      onClick={onClose}
      onKeyDown={handleKeyDown}
    >
      <div
        ref={panelRef}
        className={cx('mc-dialog', size !== 'md' && `mc-dialog--${size}`, className)}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mc-dialog__head">
          <div className="mc-dialog__title" id={titleId}>
            {title}
          </div>
          <IconButton icon="x" label={closeLabel} onClick={onClose} size={16} />
        </div>
        <div className="mc-dialog__body">{children}</div>
        {footer ? <div className="mc-dialog__foot">{footer}</div> : null}
      </div>
    </div>,
    document.body,
  );
}

export default Dialog;
