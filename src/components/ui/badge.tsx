/**
 * Badge — pastel de superfície + tinta legível (spec §3.6, regra 1).
 *
 * `--x-soft` pinta o fundo, `--x` pinta a palavra. O pastel nunca é texto:
 * nenhum pastel atinge 4,5:1 sobre fundo claro — é aritmética, não gosto.
 *
 * `dot` acrescenta o ponto de estado; a palavra continua obrigatória
 * (regra 6: cor nunca sozinha).
 */

import type { ReactNode } from 'react';

export type BadgeTone = 'neutral' | 'success' | 'warning' | 'danger' | 'info' | 'accent';

export interface BadgeProps {
  tone?: BadgeTone;
  dot?: boolean;
  children: ReactNode;
}

export function Badge({ tone = 'neutral', dot, children }: BadgeProps) {
  return (
    <span className={`mc-badge mc-badge--${tone}`}>
      {dot && <span className="mc-badge__dot" aria-hidden="true" />}
      {children}
    </span>
  );
}
