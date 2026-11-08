import type { ReactNode } from 'react';
import { Icon, type IconName } from './icons';
import { cx } from './cx';

export interface StatRowProps {
  label: ReactNode;
  value: ReactNode;
  /** Unidade em `--ink-dim` a 0,85em (nunca colada ao número sem nbsp). */
  unit?: string;
  /** Nota secundária à direita do valor ("pausa", "est. RAPL"). */
  hint?: ReactNode;
  icon?: IconName;
  /** Fonte mono para valores técnicos (voltagem, hex). */
  mono?: boolean;
  /** Estado "sem leitura": valor em `--ink-dim`, peso normal. */
  empty?: boolean;
  className?: string;
}

/**
 * StatRow — a linha label/valor que hoje está copiada 87×.
 * Anatomia: label 13 `--ink-muted` / valor 13 tabular 600 à direita.
 */
export function StatRow({ label, value, unit, hint, icon, mono, empty, className }: StatRowProps) {
  return (
    <div className={cx('mc-stat-row', empty && 'mc-stat-row--empty', className)}>
      <span className="mc-stat-row__label">
        {icon ? <Icon name={icon} size={16} /> : null}
        {label}
      </span>
      <span className="mc-stat-row__value">
        <span className={cx(mono && 'mc-stat-row__value--mono')}>{value}</span>
        {unit ? <span className="mc-stat-row__unit">{unit}</span> : null}
        {hint ? <span className="mc-stat-row__hint">{hint}</span> : null}
      </span>
    </div>
  );
}

export interface StatGroupProps {
  children: ReactNode;
  className?: string;
}

export function StatGroup({ children, className }: StatGroupProps) {
  return <div className={cx('mc-stat-group', className)}>{children}</div>;
}

export default StatRow;
