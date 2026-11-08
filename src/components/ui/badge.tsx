/**
 * Badge — pastel de superfície + tinta legível (spec §3.6, regra 1).
 *
 * `--x-soft` pinta o fundo, `--x` pinta a palavra. O pastel nunca é texto:
 * nenhum pastel atinge 4,5:1 sobre fundo claro — é aritmética, não gosto.
 *
 * `dot` acrescenta o ponto de estado; a palavra continua obrigatória
 * (regra 6: cor nunca sozinha). `icon`/`count` vêm da API do theme-core.
 */

import type { ReactNode } from 'react';
import { Icon, type IconName } from './icons';

export type BadgeTone = 'neutral' | 'success' | 'warning' | 'danger' | 'info' | 'accent';

export interface BadgeProps {
  tone?: BadgeTone;
  dot?: boolean;
  icon?: IconName;
  count?: number;
  className?: string;
  children: ReactNode;
}

export function Badge({ tone = 'neutral', dot, icon, count, className, children }: BadgeProps) {
  return (
    <span className={`mc-badge mc-badge--${tone}${className ? ` ${className}` : ''}`}>
      {dot && <span className="mc-badge__dot" aria-hidden="true" />}
      {icon && <Icon name={icon} size={13} />}
      {children}
      {typeof count === 'number' && <span className="mc-badge__count">{count}</span>}
    </span>
  );
}

export default Badge;