/**
 * Ícones SVG da app (spec §5.8).
 *
 * Regras da casa:
 *  - 24×24 viewBox, stroke 1,5, `currentColor` — herdam o tema, ao contrário
 *    dos glifos emoji que a app ainda tinha;
 *  - `aria-hidden` por omissão: o nome acessível vive no `aria-label` do botão;
 *    com `title` o SVG passa a `role="img"` nomeado;
 *  - tamanhos 20 (linha de dados) / 16 (botão inline) / 24 (herói). Nunca
 *    escalar 16 → 64.
 *
 * `refresh` não está na lista de 6 da spec — é o 7º, exigido pelo estado
 * "sem leitura desde HH:MM" (§5.5, estado 3), que precisa de um botão de retry.
 * `x` e `fan` vêm do sistema de ícones do theme-core (diálogos e ventoinha) e
 * ficam aqui para o merge não deixar consumidores sem glifo.
 *
 * O catálogo cobre a totalidade dos glifos que a UI usava em emoji (nav,
 * cabeçalhos, estados, botões, modos de desempenho). Os nomes são semânticos,
 * não o glifo de origem: dois símbolos que diziam a mesma coisa (câmara,
 * altifalante baixo, folha, brilho) partilham um único glifo.
 */

import type { ReactNode } from 'react';

export type IconName =
  // theme-core (ordem original preservada)
  | 'battery'
  | 'plug'
  | 'shield'
  | 'thermometer'
  | 'bolt'
  | 'info'
  | 'refresh'
  | 'fan'
  | 'x'
  // estado / feedback
  | 'check'
  | 'check-circle'
  | 'alert-triangle'
  | 'ban'
  | 'hourglass'
  | 'dot'
  | 'star'
  | 'sparkles'
  | 'lightbulb'
  // navegação / setas
  | 'chevron-up'
  | 'chevron-down'
  | 'chevron-right'
  | 'arrow-up'
  | 'arrow-down'
  | 'play'
  | 'pause'
  | 'stop'
  | 'rotate-ccw'
  | 'search'
  | 'pencil'
  // sistema / hardware
  | 'monitor'
  | 'smartphone'
  | 'tv'
  | 'keyboard'
  | 'mouse'
  | 'wifi'
  | 'radio'
  | 'satellite'
  | 'camera'
  | 'microphone'
  | 'music'
  | 'save'
  | 'gear'
  | 'wrench'
  | 'broom'
  | 'rocket'
  | 'wind'
  | 'chart-bar'
  | 'chart-line'
  | 'target'
  | 'scale'
  | 'brain'
  | 'flame'
  | 'mountain'
  | 'skull'
  | 'leaf'
  | 'sun'
  | 'virus'
  | 'building'
  // ficheiros / dados
  | 'folder'
  | 'folder-open'
  | 'file-text'
  | 'clipboard'
  | 'package'
  | 'archive'
  | 'image'
  | 'trash'
  | 'download'
  | 'upload'
  | 'globe'
  | 'link'
  | 'key'
  | 'lock'
  | 'eye'
  | 'eye-off'
  // pessoas / conteúdo
  | 'bot'
  | 'smile'
  | 'coffee'
  | 'book-open'
  | 'message'
  | 'phone'
  | 'palette'
  | 'flask'
  | 'shopping-cart'
  | 'tag'
  | 'thumbs-up'
  | 'thumbs-down'
  | 'volume-x'
  | 'volume-low'
  | 'volume-high';

const PATHS: Record<IconName, ReactNode> = {
  // ── theme-core ────────────────────────────────────────────────────────────
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
  fan: (
    <>
      <path d="M12 3v18" />
      <path d="M5 8c0 4 3 7 7 7s7-3 7-7" />
      <path d="M4 21h16" />
    </>
  ),
  x: <path d="M18 6 6 18M6 6l12 12" />,

  // ── estado / feedback ─────────────────────────────────────────────────────
  check: <path d="M20 6 9 17l-5-5" />,
  'check-circle': (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="m8 12 2.5 2.5L16 9" />
    </>
  ),
  'alert-triangle': (
    <>
      <path d="M10.3 3.9 1.9 18a2 2 0 0 0 1.7 3h16.8a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z" />
      <path d="M12 9v4M12 17h.01" />
    </>
  ),
  ban: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="m5.6 5.6 12.8 12.8" />
    </>
  ),
  hourglass: (
    <>
      <path d="M6 2h12M6 22h12" />
      <path d="M7 2v4.5L12 11l5-4.5V2" />
      <path d="M7 22v-4.5L12 13l5 4.5V22" />
    </>
  ),
  dot: <circle cx="12" cy="12" r="3.2" fill="currentColor" stroke="none" />,
  star: <path d="m12 3 2.8 5.9 6.2.9-4.5 4.5 1.1 6.3L12 17.7 6.4 20.6l1.1-6.3L3 9.8l6.2-.9z" />,
  sparkles: (
    <>
      <path d="m11 3 1.7 4.6L17.3 9.3l-4.6 1.7L11 15.6 9.3 11 4.7 9.3 9.3 7.6z" />
      <path d="m18 14 .9 2.1 2.1.9-2.1.9-.9 2.1-.9-2.1-2.1-.9 2.1-.9z" />
    </>
  ),
  lightbulb: (
    <>
      <path d="M8.5 14.5A6 6 0 1 1 15.5 14.5c-.8.7-1.2 1.4-1.3 2.5h-4.4c-.1-1.1-.5-1.8-1.3-2.5z" />
      <path d="M9 18h6M10 21h4" />
    </>
  ),

  // ── navegação / setas ─────────────────────────────────────────────────────
  'chevron-up': <path d="m6 15 6-6 6 6" />,
  'chevron-down': <path d="m6 9 6 6 6-6" />,
  'chevron-right': <path d="m9 6 6 6-6 6" />,
  'arrow-up': <path d="M12 20V4M6 10l6-6 6 6" />,
  'arrow-down': <path d="M12 4v16M6 14l6 6 6-6" />,
  play: <path d="M7 4.5v15l12.5-7.5z" />,
  pause: (
    <>
      <rect x="6.5" y="4" width="3.5" height="16" rx="1" />
      <rect x="14" y="4" width="3.5" height="16" rx="1" />
    </>
  ),
  stop: <rect x="6" y="6" width="12" height="12" rx="2" />,
  'rotate-ccw': (
    <>
      <path d="M3 12a9 9 0 1 0 2.6-6.4L3 8" />
      <path d="M3 3v5h5" />
    </>
  ),
  search: (
    <>
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-3.5-3.5" />
    </>
  ),
  pencil: (
    <>
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z" />
    </>
  ),

  // ── sistema / hardware ────────────────────────────────────────────────────
  monitor: (
    <>
      <rect x="2" y="4" width="20" height="13" rx="2" />
      <path d="M8 21h8M12 17v4" />
    </>
  ),
  smartphone: (
    <>
      <rect x="6" y="2" width="12" height="20" rx="2.5" />
      <path d="M11 18h2" />
    </>
  ),
  tv: (
    <>
      <rect x="2" y="7" width="20" height="13" rx="2" />
      <path d="m7 3 5 4 5-4" />
    </>
  ),
  keyboard: (
    <>
      <rect x="2" y="6" width="20" height="12" rx="2" />
      <path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M8 14h8" />
    </>
  ),
  mouse: (
    <>
      <rect x="6" y="2" width="12" height="20" rx="6" />
      <path d="M12 6v4" />
    </>
  ),
  wifi: (
    <>
      <path d="M4.5 12.5a11 11 0 0 1 15 0" />
      <path d="M8 15.8a6.5 6.5 0 0 1 8 0" />
      <path d="M11 19a1.8 1.8 0 0 1 2 0" />
      <path d="M12 21h.01" />
    </>
  ),
  radio: (
    <>
      <circle cx="12" cy="12" r="2.2" />
      <path d="M4.9 4.9a10 10 0 0 0 0 14.2M19.1 4.9a10 10 0 0 1 0 14.2" />
      <path d="M7.8 7.8a6 6 0 0 0 0 8.4M16.2 7.8a6 6 0 0 1 0 8.4" />
    </>
  ),
  satellite: (
    <>
      <path d="M10.5 6.5 3 14l7 7 7.5-7.5z" />
      <path d="M15 9 9 15" />
      <path d="M14.5 4a6 6 0 0 1 5.5 5.5" />
      <path d="M17 1.5a9.5 9.5 0 0 1 5.5 5.5" />
    </>
  ),
  camera: (
    <>
      <path d="M4 7h3l1.5-2h7L17 7h3a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V8a1 1 0 0 1 1-1z" />
      <circle cx="12" cy="13" r="3.5" />
    </>
  ),
  microphone: (
    <>
      <rect x="9" y="2" width="6" height="11" rx="3" />
      <path d="M5 11a7 7 0 0 0 14 0M12 18v4M8 22h8" />
    </>
  ),
  music: (
    <>
      <path d="M9 18V5l11-2v13" />
      <circle cx="6" cy="18" r="3" />
      <circle cx="17" cy="16" r="3" />
    </>
  ),
  save: (
    <>
      <path d="M4 3h12l4 4v14H4z" />
      <path d="M8 3v5h7V3M8 21v-6h8v6" />
    </>
  ),
  gear: (
    <>
      <path d="M12.2 2h-.4a2 2 0 0 0-2 2v.2a2 2 0 0 1-1 1.7l-.4.3a2 2 0 0 1-2 0l-.2-.1a2 2 0 0 0-2.7.7l-.2.4a2 2 0 0 0 .7 2.7l.2.1a2 2 0 0 1 1 1.7v.5a2 2 0 0 1-1 1.7l-.2.1a2 2 0 0 0-.7 2.7l.2.4a2 2 0 0 0 2.7.7l.2-.1a2 2 0 0 1 2 0l.4.3a2 2 0 0 1 1 1.7V20a2 2 0 0 0 2 2h.4a2 2 0 0 0 2-2v-.2a2 2 0 0 1 1-1.7l.4-.3a2 2 0 0 1 2 0l.2.1a2 2 0 0 0 2.7-.7l.2-.4a2 2 0 0 0-.7-2.7l-.2-.1a2 2 0 0 1-1-1.7v-.5a2 2 0 0 1 1-1.7l.2-.1a2 2 0 0 0 .7-2.7l-.2-.4a2 2 0 0 0-2.7-.7l-.2.1a2 2 0 0 1-2 0l-.4-.3a2 2 0 0 1-1-1.7V4a2 2 0 0 0-2-2z" />
      <circle cx="12" cy="12" r="3" />
    </>
  ),
  wrench: (
    <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.8-3.8a6 6 0 0 1-7.9 7.9l-6.9 6.9a2.1 2.1 0 0 1-3-3l6.9-6.9a6 6 0 0 1 7.9-7.9z" />
  ),
  broom: (
    <>
      <path d="m19 3-8 8" />
      <path d="m13 7 4 4" />
      <path d="M9 10.5 6.5 13a3.5 3.5 0 0 0 0 5l-.5.5 2 2 .5-.5a3.5 3.5 0 0 0 5 0l2.5-2.5z" />
      <path d="m6 19-2 2M9 16l-2 2" />
    </>
  ),
  rocket: (
    <>
      <path d="M12 2c2.8 2.2 4.5 5.6 4.5 9.4L12 15l-4.5-3.6C7.5 7.6 9.2 4.2 12 2z" />
      <path d="M7.5 11.4 5 14l1.5 3.5 3-1.6M16.5 11.4 19 14l-1.5 3.5-3-1.6" />
      <circle cx="12" cy="8.5" r="1.4" />
      <path d="M12 15v3M10 21h4l-2-3z" />
    </>
  ),
  wind: (
    <>
      <path d="M3 8h11a3 3 0 1 0-3-3" />
      <path d="M3 12h15a3 3 0 1 1-3 3" />
      <path d="M3 16h8a2.5 2.5 0 1 1-2.5 2.5" />
    </>
  ),
  'chart-bar': (
    <>
      <path d="M3 20h18" />
      <path d="M6.5 20v-6M11 20V8M15.5 20v-9M20 20V4" />
    </>
  ),
  'chart-line': (
    <>
      <path d="M3 20h18" />
      <path d="M4 16l5-5 4 3 7-8" />
      <path d="M16 6h4v4" />
    </>
  ),
  target: (
    <>
      <circle cx="12" cy="12" r="9" />
      <circle cx="12" cy="12" r="5" />
      <circle cx="12" cy="12" r="1.4" fill="currentColor" stroke="none" />
    </>
  ),
  scale: (
    <>
      <path d="M12 3v18M7 21h10" />
      <path d="M5.5 7h13" />
      <path d="M5.5 7 2.5 14a3.2 3.2 0 0 0 6 0z" />
      <path d="M18.5 7l-3 7a3.2 3.2 0 0 0 6 0z" />
    </>
  ),
  brain: (
    <>
      <path d="M9.5 3a3 3 0 0 0-3 3 3 3 0 0 0-2 5.2A3 3 0 0 0 6 16.7 3 3 0 0 0 9.5 21 2.5 2.5 0 0 0 12 18.5V5.5A2.5 2.5 0 0 0 9.5 3z" />
      <path d="M14.5 3a3 3 0 0 1 3 3 3 3 0 0 1 2 5.2A3 3 0 0 1 18 16.7 3 3 0 0 1 14.5 21 2.5 2.5 0 0 1 12 18.5V5.5A2.5 2.5 0 0 1 14.5 3z" />
      <path d="M9 8h1.5M13.5 12H15M9 16h1.5" />
    </>
  ),
  flame: (
    <path d="M12 2c1 4 5 5.5 5 10a5 5 0 0 1-10 0c0-2 1-3.5 2-4.5 0 2 1 3 2 3 0-3 1-6 1-8.5z" />
  ),
  mountain: (
    <>
      <path d="m2 20 6.5-11 4.5 7.5 2.5-4L22 20z" />
      <path d="M8.5 9v.01" />
    </>
  ),
  skull: (
    <>
      <path d="M5 11a7 7 0 1 1 14 0v3.5a2 2 0 0 1-.9 1.7l-1.1.8V20a1 1 0 0 1-1 1H8a1 1 0 0 1-1-1v-3l-1.1-.8a2 2 0 0 1-.9-1.7z" />
      <path d="M9 11.5h.01M15 11.5h.01" />
      <path d="M10 18v3M14 18v3" />
    </>
  ),
  leaf: (
    <>
      <path d="M20 4c0 9-5 14-11 14a5 5 0 0 1-5-5C4 7 10 4 20 4z" />
      <path d="M4 20c3-6 7-9 12-11" />
    </>
  ),
  sun: (
    <>
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2v2M12 20v2M4.2 4.2l1.4 1.4M18.4 18.4l1.4 1.4M2 12h2M20 12h2M4.2 19.8l1.4-1.4M18.4 5.6l1.4-1.4" />
    </>
  ),
  virus: (
    <>
      <circle cx="12" cy="12" r="7" />
      <path d="M12 2v3M12 19v3M2 12h3M19 12h3M5 5l2 2M17 17l2 2M19 5l-2 2M7 17l-2 2" />
      <path d="M10 10h.01M14 14h.01M10 14h.01" />
    </>
  ),
  building: (
    <>
      <path d="M4 21V5a2 2 0 0 1 2-2h6a2 2 0 0 1 2 2v16" />
      <path d="M14 9h4a2 2 0 0 1 2 2v10" />
      <path d="M2 21h20" />
      <path d="M8 7h2M8 11h2M8 15h2M17 13h1M17 17h1" />
    </>
  ),

  // ── ficheiros / dados ─────────────────────────────────────────────────────
  folder: <path d="M3 7a2 2 0 0 1 2-2h4l2 2.5h8a2 2 0 0 1 2 2V18a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />,
  'folder-open': (
    <>
      <path d="M3 18V7a2 2 0 0 1 2-2h4l2 2.5h8a2 2 0 0 1 2 2V11" />
      <path d="M3 18l3-5.5A2 2 0 0 1 7.8 11H22l-3.4 8H4a1 1 0 0 1-1-1z" />
    </>
  ),
  'file-text': (
    <>
      <path d="M14 2H7a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V7z" />
      <path d="M14 2v5h5M9 13h6M9 17h6" />
    </>
  ),
  clipboard: (
    <>
      <path d="M9 3h6v3H9z" />
      <path d="M9 4.5H7a2 2 0 0 0-2 2V20a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V6.5a2 2 0 0 0-2-2h-2" />
      <path d="M9 12h6M9 16h4" />
    </>
  ),
  package: (
    <>
      <path d="m12 2 9 4.5v11L12 22 3 17.5v-11z" />
      <path d="M3 6.5 12 11l9-4.5M12 11v11" />
    </>
  ),
  archive: (
    <>
      <rect x="2" y="4" width="20" height="5" rx="1" />
      <path d="M4 9v10a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9" />
      <path d="M10 13h4" />
    </>
  ),
  image: (
    <>
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <circle cx="9" cy="9" r="1.6" />
      <path d="m21 15-4.5-4.5L9 18" />
    </>
  ),
  trash: (
    <>
      <path d="M3 6h18" />
      <path d="M8 6V4h8v2" />
      <path d="M6 6l1 15a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-15" />
      <path d="M10 11v6M14 11v6" />
    </>
  ),
  download: (
    <>
      <path d="M12 3v12" />
      <path d="m7 11 5 5 5-5" />
      <path d="M4 20h16" />
    </>
  ),
  upload: (
    <>
      <path d="M12 16V4" />
      <path d="m7 9 5-5 5 5" />
      <path d="M4 20h16" />
    </>
  ),
  globe: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M3 12h18" />
      <path d="M12 3c2.5 2.5 3.8 5.6 3.8 9s-1.3 6.5-3.8 9c-2.5-2.5-3.8-5.6-3.8-9S9.5 5.5 12 3z" />
    </>
  ),
  link: (
    <>
      <path d="M9.5 14.5a4 4 0 0 0 5.6 0l3-3a4 4 0 0 0-5.6-5.6l-1.7 1.7" />
      <path d="M14.5 9.5a4 4 0 0 0-5.6 0l-3 3a4 4 0 0 0 5.6 5.6l1.7-1.7" />
    </>
  ),
  key: (
    <>
      <circle cx="7.5" cy="15.5" r="4" />
      <path d="m10.5 12.5 9.5-9.5" />
      <path d="M16.5 6.5 19 9M14 9l2.5 2.5" />
    </>
  ),
  lock: (
    <>
      <rect x="4.5" y="10" width="15" height="11" rx="2" />
      <path d="M8 10V7a4 4 0 0 1 8 0v3" />
    </>
  ),
  eye: (
    <>
      <path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z" />
      <circle cx="12" cy="12" r="3" />
    </>
  ),
  'eye-off': (
    <>
      <path d="M2 2l20 20" />
      <path d="M6.7 6.7C4 8.4 2 12 2 12s3.5 7 10 7c1.9 0 3.6-.6 5-1.4" />
      <path d="M10.6 5.2A9.9 9.9 0 0 1 12 5c6.5 0 10 7 10 7a17 17 0 0 1-3 3.9" />
      <path d="M9.9 9.9a3 3 0 0 0 4.2 4.2" />
    </>
  ),

  // ── pessoas / conteúdo ────────────────────────────────────────────────────
  bot: (
    <>
      <rect x="4" y="8" width="16" height="12" rx="2" />
      <path d="M12 8V5" />
      <circle cx="12" cy="3.5" r="1.2" />
      <path d="M9 13h.01M15 13h.01M9.5 17h5" />
    </>
  ),
  smile: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M8 14s1.5 2 4 2 4-2 4-2" />
      <path d="M9 9.5h.01M15 9.5h.01" />
    </>
  ),
  coffee: (
    <>
      <path d="M3 8h14v6a5 5 0 0 1-5 5H8a5 5 0 0 1-5-5z" />
      <path d="M17 9h1.5a3.5 3.5 0 0 1 0 7H17" />
      <path d="M6 2v2M10 2v2M14 2v2" />
    </>
  ),
  'book-open': (
    <>
      <path d="M12 6.5C10.5 5 8 4 4 4v14c4 0 6.5 1 8 2.5 1.5-1.5 4-2.5 8-2.5V4c-4 0-6.5 1-8 2.5z" />
      <path d="M12 6.5v14" />
    </>
  ),
  message: (
    <path d="M21 11.5a8.4 8.4 0 0 1-8.5 8.5 8.6 8.6 0 0 1-3.8-.9L3 21l1.9-5.7a8.6 8.6 0 0 1-.9-3.8A8.4 8.4 0 0 1 12.5 3 8.4 8.4 0 0 1 21 11.5z" />
  ),
  phone: (
    <path d="M5 3h4l2 5-2.5 1.5a12 12 0 0 0 6 6L16 13l5 2v4a2 2 0 0 1-2.2 2A17 17 0 0 1 3 5.2 2 2 0 0 1 5 3z" />
  ),
  palette: (
    <>
      <path d="M12 21a9 9 0 1 1 9-9c0 2-1.5 3-3 3h-1.5a2 2 0 0 0-1.4 3.4c.6.6.4 1.6-.4 2A9 9 0 0 1 12 21z" />
      <path d="M8 9h.01M12 6.5h.01M16 9h.01" />
    </>
  ),
  flask: (
    <>
      <path d="M9 2h6M10 2v6.5L4.5 18a2 2 0 0 0 1.7 3h11.6a2 2 0 0 0 1.7-3L14 8.5V2" />
      <path d="M7.5 14h9" />
    </>
  ),
  'shopping-cart': (
    <>
      <path d="M2 3h2.5l2.6 12h11.3l2.3-8H6" />
      <circle cx="9" cy="20" r="1.5" />
      <circle cx="18" cy="20" r="1.5" />
    </>
  ),
  tag: (
    <>
      <path d="M3 12V4a1 1 0 0 1 1-1h8l9 9-9 9z" />
      <circle cx="8" cy="8" r="1.4" />
    </>
  ),
  'thumbs-up': (
    <path d="M7 21H4a1 1 0 0 1-1-1v-7a1 1 0 0 1 1-1h3M7 21h9.6a2 2 0 0 0 2-1.6l1.3-7A2 2 0 0 0 18 10h-4V5a2 2 0 0 0-2-2l-3 8v10z" />
  ),
  'thumbs-down': (
    <path d="M7 3H4a1 1 0 0 0-1 1v7a1 1 0 0 0 1 1h3M7 3h9.6a2 2 0 0 1 2 1.6l1.3 7A2 2 0 0 1 18 14h-4v5a2 2 0 0 1-2 2l-3-8V3z" />
  ),
  'volume-x': (
    <>
      <path d="M11 5 6 9H3v6h3l5 4z" />
      <path d="m16 9 5 6M21 9l-5 6" />
    </>
  ),
  'volume-low': (
    <>
      <path d="M11 5 6 9H3v6h3l5 4z" />
      <path d="M16 9.5a4 4 0 0 1 0 5" />
    </>
  ),
  'volume-high': (
    <>
      <path d="M11 5 6 9H3v6h3l5 4z" />
      <path d="M16 9a4 4 0 0 1 0 6" />
      <path d="M19 6.5a8 8 0 0 1 0 11" />
    </>
  ),
};

export interface IconProps {
  name: IconName;
  /** 20 por omissão (linha de dados). 16 em botões inline, 24 no herói. */
  size?: number;
  className?: string;
  /** Nome acessível. Sem ele o SVG é decorativo (`aria-hidden`). */
  title?: string;
  /** Só para `battery`: preenche o interior até `level` % (0–100). */
  level?: number;
}

export function Icon({ name, size = 20, className, title, level }: IconProps) {
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
      aria-hidden={title ? undefined : true}
      role={title ? 'img' : undefined}
      focusable="false"
    >
      {title ? <title>{title}</title> : null}
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
