/**
 * Card — superfície hairline: 1 px, raio 12, **zero sombra** (spec §2.4/§3.1).
 *
 * Sem `style={{}}`: a regra da vaga 1 é que nada em `components/ui/` escreve
 * estilos inline. O título é `h2` real (a app hoje tem `h1=h2=h3=0`).
 */

import type { ReactNode } from 'react';

export interface CardProps {
  title?: string;
  /** Acção à direita do título (ex.: `IconButton` de ajuda). */
  action?: ReactNode;
  children: ReactNode;
  className?: string;
  /** Marca o card como "sem leitura" — hairline superior passa a `--warning`. */
  stale?: boolean;
}

export function Card({ title, action, children, className, stale }: CardProps) {
  const classes = ['mc-card'];
  if (stale) classes.push('mc-card--stale');
  if (className) classes.push(className);

  return (
    <section className={classes.join(' ')}>
      {(title || action) && (
        <div className="mc-card__head">
          {title && <h2 className="mc-card__title">{title}</h2>}
          {action && <div className="mc-card__action">{action}</div>}
        </div>
      )}
      {children}
    </section>
  );
}
