import type { ReactNode } from 'react';
import { Icon, type IconName } from './icons';
import { cx } from './cx';

export type BadgeTone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger' | 'info';

export interface BadgeProps {
  tone?: BadgeTone;
  /** Ponto que herda a cor do texto. O estado nunca é só cor: há palavra. */
  dot?: boolean;
  icon?: IconName;
  count?: number;
  children: ReactNode;
  className?: string;
}

/**
 * Badge — pastel é superfície, tinta é texto (§3.6).
 * `--x-soft` pinta o fundo, `--x` pinta a palavra. Nunca o contrário.
 */
export function Badge({ tone = 'neutral', dot, icon, count, children, className }: BadgeProps) {
  return (
    <span className={cx('mc-badge', `mc-badge--${tone}`, className)}>
      {dot ? <span className="mc-badge__dot" aria-hidden="true" /> : null}
      {icon ? <Icon name={icon} size={13} /> : null}
      {children}
      {typeof count === 'number' ? <span className="mc-badge__count">{count}</span> : null}
    </span>
  );
}

export default Badge;
