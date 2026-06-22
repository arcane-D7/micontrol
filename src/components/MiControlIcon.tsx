import type { SVGProps } from 'react';

interface MiControlIconProps extends SVGProps<SVGSVGElement> {
  size?: number;
}

/**
 * Inline SVG icon for MiControl.
 *
 * Renders a geometric "M" letterform in `currentColor` (inherits from parent)
 * with a periwinkle accent underbar using `var(--accent)`.
 * Designed to be sharp at 16–32 px sidebar sizes.
 */
export function MiControlIcon({ size = 22, style, ...props }: MiControlIconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      style={{ flexShrink: 0, ...style }}
      aria-hidden="true"
      {...props}
    >
      {/* M letterform — left stem, left diagonal, right diagonal, right stem */}
      <polyline
        points="4,19 4,5 12,12 20,5 20,19"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      {/* Accent nodes at the stem bases */}
      <circle cx="4" cy="19" r="1.8" fill="var(--accent)" />
      <circle cx="20" cy="19" r="1.8" fill="var(--accent)" />
      {/* Accent underbar — represents the "Control" dimension */}
      <line
        x1="4"
        y1="22.5"
        x2="20"
        y2="22.5"
        stroke="var(--accent)"
        strokeWidth="2"
        strokeLinecap="round"
      />
    </svg>
  );
}
