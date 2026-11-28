import type { ButtonHTMLAttributes, ReactNode } from 'react';
import { Icon, type IconName } from './icons';
import { cx } from './cx';

export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: 'sm' | 'md' | 'lg';
  icon?: IconName;
  iconRight?: IconName;
  /** Spinner + `aria-busy`; bloqueia o segundo clique. */
  pending?: boolean;
  full?: boolean;
  children?: ReactNode;
}

/**
 * Button — 32 px de altura, raio 8, 1 px hairline, press de 3%.
 * Primário é TINTA (`--ink`), não accent: o accent só pinta estado.
 */
export function Button({
  variant = 'secondary',
  size = 'md',
  icon,
  iconRight,
  pending,
  full,
  className,
  children,
  disabled,
  type = 'button',
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      className={cx(
        'mc-btn',
        `mc-btn--${variant}`,
        size !== 'md' && `mc-btn--${size}`,
        full && 'mc-btn--full',
        className,
      )}
      disabled={disabled || pending}
      aria-busy={pending || undefined}
      {...rest}
    >
      {pending ? <span className="mc-btn__spinner" aria-hidden="true" /> : null}
      {!pending && icon ? <Icon name={icon} size={size === 'sm' ? 14 : 16} /> : null}
      {children}
      {!pending && iconRight ? <Icon name={iconRight} size={size === 'sm' ? 14 : 16} /> : null}
    </button>
  );
}

export interface IconButtonProps {
  icon: IconName;
  /** Nome acessível obrigatório: o glifo é decorativo. */
  label: string;
  onClick?: () => void;
  disabled?: boolean;
  size?: number;
  className?: string;
  title?: string;
}

/** Hit area real 32×32; hover = círculo de `--accent-soft` atrás do glifo. */
export function IconButton({
  icon,
  label,
  onClick,
  disabled,
  size = 20,
  className,
  title,
}: IconButtonProps) {
  return (
    <button
      type="button"
      className={cx('mc-icon-btn', className)}
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      title={title}
    >
      <Icon name={icon} size={size} />
    </button>
  );
}

export default Button;
