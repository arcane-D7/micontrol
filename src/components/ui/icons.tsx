import type { ReactNode, SVGProps } from 'react';

/**
 * Sistema de ícones — um só vocabulário de forma.
 *
 * Regras (theme-migration.md §5.8):
 *  - 24×24, stroke 1.6, `linecap`/`linejoin` round, `fill: none`
 *  - `currentColor` sempre (emoji não herda a cor do tema; SVG herda)
 *  - tamanhos: 16 em botões inline, 20 em linha de dados, 24 só no herói
 *  - `aria-hidden` por omissão; o nome vive no `aria-label` do botão
 *
 * Substitui progressivamente os 283 glifos emoji. Os ecrãs que ainda não foram
 * tocados continuam com emoji até ao varrimento do tabs-cognitive.
 */

export type IconName =
  | 'overview'
  | 'tasks'
  | 'performance'
  | 'optimize'
  | 'battery'
  | 'display'
  | 'fan'
  | 'audio'
  | 'cast'
  | 'touchpad'
  | 'plug'
  | 'person'
  | 'wifi'
  | 'power'
  | 'cpu'
  | 'keyboard'
  | 'shield'
  | 'devices'
  | 'palette'
  | 'sparkles'
  | 'bot'
  | 'settings'
  | 'info'
  | 'check'
  | 'x'
  | 'chevron-down'
  | 'chevron-right'
  | 'chevron-left'
  | 'search'
  | 'refresh'
  | 'history'
  | 'sun'
  | 'moon'
  | 'monitor'
  | 'bolt'
  | 'thermometer'
  | 'clock'
  | 'download'
  | 'trash'
  | 'alert'
  | 'plus'
  | 'lock'
  | 'file'
  | 'folder'
  | 'arrow-right'
  | 'eye'
  | 'target'
  | 'sliders';

const ICONS: Record<IconName, ReactNode> = {
  overview: (
    <>
      <rect x="3" y="3" width="7" height="7" rx="1.5" />
      <rect x="14" y="3" width="7" height="7" rx="1.5" />
      <rect x="3" y="14" width="7" height="7" rx="1.5" />
      <rect x="14" y="14" width="7" height="7" rx="1.5" />
    </>
  ),
  tasks: <path d="M4 6h16M4 12h10M4 18h13" />,
  performance: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v5l3 3" />
    </>
  ),
  optimize: (
    <>
      <path d="M12 14l4-4" />
      <path d="M20.5 18a9 9 0 1 0-17 0" />
    </>
  ),
  battery: (
    <>
      <rect x="2" y="7" width="17" height="10" rx="2.5" />
      <path d="M22 10v4" />
    </>
  ),
  display: (
    <>
      <rect x="2" y="5" width="20" height="14" rx="2" />
      <path d="M8 21h8M12 19v2" />
    </>
  ),
  fan: (
    <>
      <path d="M12 3v18" />
      <path d="M5 8c0 4 3 7 7 7s7-3 7-7" />
      <path d="M4 21h16" />
    </>
  ),
  audio: (
    <>
      <path d="M5 12a7 7 0 0 1 14 0c0 4-3 5-3 8H8c0-3-3-4-3-8Z" />
      <path d="M10 20v2M14 20v2" />
    </>
  ),
  cast: (
    <>
      <path d="M2 7a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2h-6" />
      <path d="M2 12a9 9 0 0 1 9 9" />
      <path d="M2 17a4 4 0 0 1 4 4" />
      <path d="M2 21h.01" />
    </>
  ),
  touchpad: (
    <>
      <rect x="5" y="2" width="14" height="20" rx="7" />
      <path d="M12 6v4" />
    </>
  ),
  plug: (
    <>
      <path d="M9 3v5M15 3v5" />
      <path d="M6 8h12v3a6 6 0 0 1-12 0z" />
      <path d="M12 17v4" />
    </>
  ),
  person: (
    <>
      <circle cx="12" cy="9" r="4" />
      <path d="M4 21c0-4 3.6-6 8-6s8 2 8 6" />
    </>
  ),
  wifi: (
    <>
      <path d="M5 12.5a10 10 0 0 1 14 0" />
      <path d="M8.5 16a5.5 5.5 0 0 1 7 0" />
      <path d="M12 19.5h.01" />
    </>
  ),
  power: (
    <>
      <path d="M12 3v9" />
      <path d="M6.3 6.3a8 8 0 1 0 11.4 0" />
    </>
  ),
  cpu: (
    <>
      <rect x="6" y="6" width="12" height="12" rx="2" />
      <path d="M9 2v4M15 2v4M9 18v4M15 18v4M2 9h4M2 15h4M18 9h4M18 15h4" />
    </>
  ),
  keyboard: (
    <>
      <rect x="2" y="6" width="20" height="12" rx="2" />
      <path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M6 14h.01M18 14h.01M9 14h6" />
    </>
  ),
  shield: <path d="M12 3l7 3v6c0 4.4-3 7.4-7 9-4-1.6-7-4.6-7-9V6z" />,
  devices: (
    <>
      <rect x="2" y="5" width="13" height="10" rx="2" />
      <path d="M6 19h6" />
      <rect x="16" y="9" width="5" height="11" rx="1.2" />
      <path d="M18 17h.01" />
    </>
  ),
  palette: (
    <>
      <path d="M12 21a9 9 0 1 1 9-9c0 2.2-1.8 3-3 3h-2a2 2 0 0 0-1.6 3.2A2 2 0 0 1 12 21Z" />
      <path d="M7.5 10h.01M11 7.5h.01M15 8h.01" />
    </>
  ),
  sparkles: (
    <>
      <path d="M12 3l1.7 4.3L18 9l-4.3 1.7L12 15l-1.7-4.3L6 9l4.3-1.7z" />
      <path d="M18.5 15l.8 2 2 .8-2 .8-.8 2-.8-2-2-.8 2-.8z" />
    </>
  ),
  bot: (
    <>
      <rect x="4" y="8" width="16" height="12" rx="3" />
      <path d="M12 3v5" />
      <path d="M9 14h.01M15 14h.01" />
      <path d="M2 13h2M20 13h2" />
    </>
  ),
  settings: (
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.9.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.9 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.9l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.9.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.9-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.9V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" />
    </>
  ),
  info: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 11v5M12 8h.01" />
    </>
  ),
  check: <path d="M20 6 9 17l-5-5" />,
  x: <path d="M18 6 6 18M6 6l12 12" />,
  'chevron-down': <path d="M6 9l6 6 6-6" />,
  'chevron-right': <path d="M9 6l6 6-6 6" />,
  'chevron-left': <path d="M15 6l-6 6 6 6" />,
  search: (
    <>
      <circle cx="11" cy="11" r="7" />
      <path d="M20 20l-4.3-4.3" />
    </>
  ),
  refresh: (
    <>
      <path d="M3 12a9 9 0 1 0 3-6.7L3 8" />
      <path d="M3 3v5h5" />
    </>
  ),
  history: (
    <>
      <path d="M3 12a9 9 0 1 0 3-6.7L3 8" />
      <path d="M3 3v5h5" />
      <path d="M12 8v4l3 2" />
    </>
  ),
  sun: (
    <>
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M19.1 4.9l-1.4 1.4M6.3 17.7l-1.4 1.4" />
    </>
  ),
  moon: <path d="M20 14.5A8.5 8.5 0 0 1 9.5 4a8.5 8.5 0 1 0 10.5 10.5Z" />,
  monitor: (
    <>
      <rect x="2" y="4" width="20" height="12" rx="2" />
      <path d="M8 20h8M12 16v4" />
    </>
  ),
  bolt: <path d="M13 2 4.5 13.5H11l-1 8.5 8.5-11.5H12z" />,
  thermometer: <path d="M14 14.8V5a2 2 0 1 0-4 0v9.8a4 4 0 1 0 4 0z" />,
  clock: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v5l3 3" />
    </>
  ),
  download: (
    <>
      <path d="M12 3v12M7 11l5 5 5-5" />
      <path d="M4 20h16" />
    </>
  ),
  trash: (
    <>
      <path d="M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2" />
      <path d="M6 7l1 13h10l1-13" />
      <path d="M10 11v6M14 11v6" />
    </>
  ),
  alert: (
    <>
      <path d="M12 9v4M12 17h.01" />
      <path d="M10.3 3.9 2.4 18a2 2 0 0 0 1.7 3h15.8a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z" />
    </>
  ),
  plus: <path d="M12 5v14M5 12h14" />,
  lock: (
    <>
      <rect x="4" y="10" width="16" height="11" rx="2" />
      <path d="M8 10V7a4 4 0 0 1 8 0v3" />
    </>
  ),
  file: (
    <>
      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
      <path d="M14 2v6h6" />
    </>
  ),
  folder: <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />,
  'arrow-right': <path d="M5 12h14M13 6l6 6-6 6" />,
  eye: (
    <>
      <path d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6-10-6-10-6Z" />
      <circle cx="12" cy="12" r="2.5" />
    </>
  ),
  target: (
    <>
      <circle cx="12" cy="12" r="9" />
      <circle cx="12" cy="12" r="4" />
    </>
  ),
  sliders: (
    <>
      <path d="M4 8h9M18 8h2M4 16h3M12 16h8" />
      <circle cx="15.5" cy="8" r="2" />
      <circle cx="9.5" cy="16" r="2" />
    </>
  ),
};

export interface IconProps extends Omit<SVGProps<SVGSVGElement>, 'name' | 'children'> {
  name: IconName;
  /** 16 em botões inline, 20 em linha de dados, 24 só no herói. */
  size?: number;
  /** Nome acessível. Sem ele o SVG é decorativo (`aria-hidden`). */
  title?: string;
}

export function Icon({ name, size = 20, title, className, ...rest }: IconProps) {
  return (
    <svg
      className={className ? `mc-icon ${className}` : 'mc-icon'}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.6}
      strokeLinecap="round"
      strokeLinejoin="round"
      focusable="false"
      aria-hidden={title ? undefined : true}
      role={title ? 'img' : undefined}
      {...rest}
    >
      {title ? <title>{title}</title> : null}
      {ICONS[name]}
    </svg>
  );
}

export default Icon;
