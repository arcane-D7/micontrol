/**
 * StatRow — a linha label/valor que hoje está copiada 87× em 23 tabs.
 *
 * Anatomia (spec §1.3): label 12–13 px `--ink-muted` à esquerda, valor 15 px
 * tabular alinhado à direita, unidade em `--ink-dim` a 0,85em. `hint` é a
 * legenda secundária (ex.: "95% da capacidade de fábrica").
 */

import type { ReactNode } from 'react';

export interface StatRowProps {
  label: string;
  value: ReactNode;
  /** Legenda secundária, à direita do valor. */
  hint?: ReactNode;
  /** Ícone à esquerda do label (20 px). */
  icon?: ReactNode;
  /** Valor em `--font-mono` (tensões, watts). */
  mono?: boolean;
  /** Valor em `--ink-dim` — para "não exposto pelo hardware". */
  dim?: boolean;
}

export function StatRow({ label, value, hint, icon, mono, dim }: StatRowProps) {
  const valueClasses = ['mc-stat__v'];
  if (mono) valueClasses.push('mc-stat__v--mono');
  if (dim) valueClasses.push('mc-stat__v--dim');

  return (
    <div className="mc-stat">
      <span className="mc-stat__k">
        {icon}
        {label}
      </span>
      <span className="mc-stat__right">
        <span className={valueClasses.join(' ')}>{value}</span>
        {hint && <span className="mc-stat__hint">{hint}</span>}
      </span>
    </div>
  );
}
