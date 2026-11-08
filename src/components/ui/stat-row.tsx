/**
 * StatRow — a linha label/valor que hoje está copiada 87× em 23 tabs.
 *
 * Anatomia (spec §1.3): label 12–13 px `--ink-muted` à esquerda, valor 15 px
 * tabular alinhado à direita, unidade em `--ink-dim` a 0,85em. `hint` é a
 * legenda secundária (ex.: "95% da capacidade de fábrica").
 *
 * `unit`/`empty`/`className` vêm da API do theme-core: o merge mantém os dois
 * conjuntos (o `dim` do POC e o `empty` do tema dizem a mesma coisa).
 */

import type { ReactNode } from 'react';

export interface StatRowProps {
  label: ReactNode;
  value: ReactNode;
  /** Unidade em `--ink-dim` a 0,85em (nunca colada ao número sem nbsp). */
  unit?: string;
  /** Legenda secundária, à direita do valor. */
  hint?: ReactNode;
  /** Ícone à esquerda do label (20 px). */
  icon?: ReactNode;
  /** Valor em `--font-mono` (tensões, watts). */
  mono?: boolean;
  /** Valor em `--ink-dim` — para "não exposto pelo hardware". */
  dim?: boolean;
  /** Estado "sem leitura": valor em `--ink-dim`, peso normal. */
  empty?: boolean;
  className?: string;
}

export function StatRow({
  label,
  value,
  unit,
  hint,
  icon,
  mono,
  dim,
  empty,
  className,
}: StatRowProps) {
  const valueClasses = ['mc-stat__v'];
  if (mono) valueClasses.push('mc-stat__v--mono');
  if (dim || empty) valueClasses.push('mc-stat__v--dim');

  const classes = ['mc-stat'];
  if (empty) classes.push('mc-stat--empty');
  if (className) classes.push(className);

  return (
    <div className={classes.join(' ')}>
      <span className="mc-stat__k">
        {icon}
        {label}
      </span>
      <span className="mc-stat__right">
        <span className={valueClasses.join(' ')}>{value}</span>
        {unit && <span className="mc-stat__unit">{unit}</span>}
        {hint && <span className="mc-stat__hint">{hint}</span>}
      </span>
    </div>
  );
}

/** StatGroup — agrupa linhas com uma hairline entre elas. */
export interface StatGroupProps {
  children: ReactNode;
  className?: string;
}

export function StatGroup({ children, className }: StatGroupProps) {
  return (
    <div className={className ? `mc-stat-group ${className}` : 'mc-stat-group'}>{children}</div>
  );
}

export default StatRow;
