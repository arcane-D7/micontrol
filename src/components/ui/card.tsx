/**
 * Card — superfície hairline: 1 px, raio 12, **zero sombra** (spec §2.4/§3.1).
 *
 * Sem `style={{}}`: a regra da vaga 1 é que nada em `components/ui/` escreve
 * estilos inline. O título é `h2` real (a app hoje tem `h1=h2=h3=0`).
 *
 * `icon`/`meta`/`footer`/`selectable`/`selected`/`loading` vêm da API do
 * theme-core e ficam para não deixar consumidores sem cartão. Só se desenham
 * quando pedidos — o cartão da bateria continua a usar `title`+`action`.
 */

import type { ReactNode } from 'react';

export interface CardProps {
  /** Título em sentence case, 13 px 600. Sem uppercase, sem tracking. */
  title?: ReactNode;
  icon?: ReactNode;
  /** Acção à direita do título (ex.: `IconButton` de ajuda). */
  action?: ReactNode;
  /** Metadados à direita (badge, "atualizado há 12 s"). */
  meta?: ReactNode;
  children?: ReactNode;
  footer?: ReactNode;
  /** Torna o cartão um botão (card deck / modo). */
  selectable?: boolean;
  selected?: boolean;
  loading?: boolean;
  className?: string;
  /** Marca o card como "sem leitura" — hairline superior passa a `--warning`. */
  stale?: boolean;
  id?: string;
  onClick?: () => void;
  'aria-label'?: string;
}

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
  className,
  stale,
  id,
  onClick,
  'aria-label': ariaLabel,
}: CardProps) {
  const classes = ['mc-card'];
  if (stale) classes.push('mc-card--stale');
  if (selectable) classes.push('mc-card--selectable');
  if (selected) classes.push('mc-card--selected');
  if (loading) classes.push('mc-card--loading');
  if (className) classes.push(className);

  const content = (
    <>
      {(title || action || meta || icon) && (
        <div className="mc-card__head">
          {icon && <span className="mc-card__icon">{icon}</span>}
          {title && <h2 className="mc-card__title">{title}</h2>}
          {meta && <span className="mc-card__meta">{meta}</span>}
          {action && <div className="mc-card__action">{action}</div>}
        </div>
      )}
      {children}
      {footer && <div className="mc-card__footer">{footer}</div>}
    </>
  );

  if (selectable || onClick) {
    return (
      <button
        type="button"
        className={classes.join(' ')}
        id={id}
        aria-label={ariaLabel}
        aria-pressed={selectable ? Boolean(selected) : undefined}
        onClick={onClick}
      >
        {content}
      </button>
    );
  }

  return (
    <section className={classes.join(' ')} id={id} aria-label={ariaLabel}>
      {content}
    </section>
  );
}

export default Card;