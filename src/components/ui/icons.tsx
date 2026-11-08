/**
 * Ícones SVG da tab Bateria (spec §5.8).
 *
 * Regras da casa:
 *  - 24×24 viewBox, stroke 1,5, `currentColor` — herdam o tema, ao contrário
 *    dos 283 glifos emoji que a app ainda tem;
 *  - `aria-hidden` sempre: o nome acessível vive no `aria-label` do botão;
 *  - tamanhos 20 (linha de dados) / 16 (botão inline) / 24 (herói). Nunca
 *    escalar 16 → 64.
 *
 * `refresh` não está na lista de 6 da spec — é o 7º, exigido pelo estado
 * "sem leitura desde HH:MM" (§5.5, estado 3), que precisa de um botão de retry.
 */

import type { ReactNode } from 'react';

export type IconName = 'battery' | 'plug' | 'shield' | 'thermometer' | 'bolt' | 'info' | 'refresh';

const PATHS: Record<IconName, ReactNode> = {
  battery: (
    <>
      <rect x="2" y="7" width="17" height="10" rx="2.5" />
      <path d="M22 11v2" />
    </>
  ),
  plug: (
    <>
      <path d="M9 3v5M15 3v5" />
      <path d="M6 8h12v3a6 6 0 0 1-12 0z" />
      <path d="M12 17v4" />
    </>
  ),
  shield: <path d="M12 3l7 3v6c0 4.4-3 7.4-7 9-4-1.6-7-4.6-7-9V6z" />,
  thermometer: <path d="M14 14.8V5a2 2 0 1 0-4 0v9.8a4 4 0 1 0 4 0z" />,
  bolt: <path d="M13 2L4.5 13.5H11l-1 8.5 8.5-11.5H12z" />,
  info: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 11v5M12 8h.01" />
    </>
  ),
  refresh: (
    <>
      <path d="M3 12a9 9 0 1 0 3-6.7L3 8" />
      <path d="M3 3v5h5" />
    </>
  ),
};

export interface IconProps {
  name: IconName;
  /** 20 por omissão (linha de dados). 16 em botões inline, 24 no herói. */
  size?: number;
  className?: string;
  /** Só para `battery`: preenche o interior até `level` % (0–100). */
  level?: number;
}

export function Icon({ name, size = 20, className, level }: IconProps) {
  return (
    <svg
      className={className ? `mc-icon ${className}` : 'mc-icon'}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {PATHS[name]}
      {name === 'battery' && level != null && level > 0 && (
        <rect
          x="4"
          y="9"
          width={Math.max(1, (13 * Math.min(100, Math.max(0, level))) / 100)}
          height="6"
          rx="1"
          fill="currentColor"
          stroke="none"
        />
      )}
    </svg>
  );
}
