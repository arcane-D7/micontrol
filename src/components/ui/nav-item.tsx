import type { ReactNode } from 'react';
import { Icon, type IconName } from './icons';
import { cx } from './cx';

export interface NavItemProps {
  icon?: IconName;
  active?: boolean;
  onClick?: () => void;
  children: ReactNode;
  /** Contador ou badge à direita. */
  trail?: ReactNode;
  className?: string;
  title?: string;
}

/**
 * NavItem — o destino do sidebar. Activo = `--surface-2` + hairline + `--ink`
 * + 600; o ícone é o único accent. `aria-current="page"` substitui `.active`
 * como contrato para os leitores de ecrã.
 */
export function NavItem({
  icon,
  active,
  onClick,
  children,
  trail,
  className,
  title,
}: NavItemProps) {
  return (
    <button
      type="button"
      className={cx('mc-nav-item', className)}
      onClick={onClick}
      aria-current={active ? 'page' : undefined}
      title={title}
    >
      {icon ? <Icon name={icon} size={18} /> : null}
      <span className="mc-nav-item__label">{children}</span>
      {trail ? <span className="mc-nav-item__trail">{trail}</span> : null}
    </button>
  );
}

export interface NavGroupProps {
  label: string;
  children: ReactNode;
  className?: string;
}

/**
 * NavGroup — o agrupamento dos 23 destinos em 5 grupos (§4.3 #3).
 * Só estrutura: quem decide os grupos é o tabs-cognitive.
 */
export function NavGroup({ label, children, className }: NavGroupProps) {
  return (
    <div className={cx('mc-nav-group', className)} role="group" aria-label={label}>
      <div className="mc-nav-group__label" aria-hidden="true">
        {label}
      </div>
      {children}
    </div>
  );
}

export default NavItem;
