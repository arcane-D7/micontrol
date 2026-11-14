import type { SVGProps } from 'react';

interface MiControlIconProps extends SVGProps<SVGSVGElement> {
  size?: number;
}

/**
 * Small cut of the MiControl symbol (docs/brand — "M de precisão").
 *
 * Mono, single `currentColor` fill: the brand palette is closed, so the mark
 * never carries an accent hue. The small cut (22% stem, no corner chamfers) is
 * the one specified for 16–24 px — at those sizes the master cut's chamfers
 * smear into the stem.
 */
const MARK_PATH =
  'M0 0L17 0L39 38.105L61 0L78 0L78 0L78 72L78 72L61 72L61 33.862L39 71.967L17 33.862L17 72L0 72L0 72L0 0Z';

/** viewBox of `logo-mark-small.svg`: the 78×72 glyph plus 2 units of trim. */
const VIEW_BOX = '-2 -2 82 76';
const RATIO = 76 / 82;

export function MiControlIcon({ size = 22, style, ...props }: MiControlIconProps) {
  return (
    <svg
      width={size}
      height={Math.round(size * RATIO * 100) / 100}
      viewBox={VIEW_BOX}
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      style={{ flexShrink: 0, ...style }}
      aria-hidden="true"
      {...props}
    >
      <path fill="currentColor" d={MARK_PATH} />
    </svg>
  );
}
