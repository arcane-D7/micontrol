/**
 * IconButton, alvo de 32×32 com glifo de 20 px e círculo de 28 px em hover
 * (`--accent-soft`, 120 ms). Spec §5.8.
 *
 * O ⓘ que existe hoje tem 16 px e 1,76:1 de contraste, é o acesso à única
 * explicação do limite de carga. 32 px é o mínimo da casa (WCAG 2.5.8 pede 24).
 */

import { Icon, type IconName } from './icons';

export interface IconButtonProps {
  icon: IconName;
  /** Nome acessível, obrigatório: o SVG é sempre `aria-hidden`. */
  label: string;
  onClick: () => void;
  disabled?: boolean;
  /** Botão de alternância (ex.: já aberto). */
  pressed?: boolean;
  /** Variante compacta de 28 px (o glifo desce para 16 px) para linhas
      apertadas, ex.: o toggle de tema no fundo da sidebar. */
  compact?: boolean;
  /** Tooltip; por omissão é o próprio `label`. */
  title?: string;
}

export function IconButton({
  icon,
  label,
  onClick,
  disabled,
  pressed,
  compact,
  title,
}: IconButtonProps) {
  return (
    <button
      type="button"
      className={compact ? 'mc-iconbtn mc-iconbtn--sm' : 'mc-iconbtn'}
      aria-label={label}
      title={title ?? label}
      aria-pressed={pressed}
      disabled={disabled}
      onClick={onClick}
    >
      <Icon name={icon} size={compact ? 16 : 20} />
    </button>
  );
}
