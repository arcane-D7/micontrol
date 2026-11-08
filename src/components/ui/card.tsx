import type { ReactNode } from 'react';
import { cx } from './cx';

export interface CardProps {
  /** Título em sentence case, 13 px 600. Sem uppercase, sem tracking. */
  title?: ReactNode;
  icon?: ReactNode;
  /** Acção à direita do título (normalmente um IconButton). */
  action?: ReactNode;
  /** Metadados à direita (badge, "atualizado há 12 s"). */
  meta?: ReactNode;
  children?: ReactNode;
  footer?: ReactNode;
  /** Torna o cartão um botão (card deck / modo). */
  selectable?: boolean;
  selected?: boolean;
  loading?: boolean;
  /** Última leitura falhou: a hairline superior passa a --warning. */
  stale?: boolean;
  flush?: boolean;
  className?: string;
  id?: string;
  onClick?: () => void;
  'aria-label'?: string;
}

/**
 * Card — a moldura hairline: 1 px, raio 12, ZERO sombra.
 * Sem `Card.Header/Body/Footer` inventados: o corpo é `children`, o resto é
 * `title`/`action`/`footer`.
 */
export function Card({
  title,
  icon,
  action,
  meta,
  children,
  footer,
  selectable,
  selected,
  loading,
  stale,
  flush,
  className,
  id,
  onClick,
  'aria-label': ariaLabel,
}: CardProps) {
  const cls = cx(
    'mc-card',
    selectable && 'mc-card--selectable',
    selected && 'mc-card--selected',
    loading && 'mc-card--loading',
    stale && 'mc-card--stale',
    flush && 'mc-card--flush',
    className,
  );

  const content = (
    <>
      {title || action || meta ? (
        <div className="mc-card__head">
          {icon}
          {title ? <div className="mc-card__title">{title}</div> : null}
          {meta ? <div className="mc-card__meta">{meta}</div> : null}
          {action ? <div className="mc-card__meta">{action}</div> : null}
        </div>
      ) : null}
      {children}
      {footer ? <div className="mc-card__foot">{footer}</div> : null}
    </>
  );

  if (onClick) {
    return (
      <button
        type="button"
        className={cls}
        id={id}
        onClick={onClick}
        aria-label={ariaLabel}
        aria-pressed={selectable ? Boolean(selected) : undefined}
        aria-busy={loading || undefined}
      >
        {content}
      </button>
    );
  }

  return (
    <section className={cls} id={id} aria-label={ariaLabel} aria-busy={loading || undefined}>
      {content}
    </section>
  );
}

export default Card;
