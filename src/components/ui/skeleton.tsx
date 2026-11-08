import type { CSSProperties, ReactNode } from 'react';
import { cx } from './cx';

export type SkeletonShape = 'line' | 'text' | 'block' | 'circle' | 'ring';

export interface SkeletonProps {
  shape?: SkeletonShape;
  /**
   * Geometria, não estilo: é a única razão aceitável para um `style` em
   * `components/ui` (a regra ESLint da vaga 4 só proíbe o resto).
   */
  width?: number | string;
  height?: number | string;
  size?: number;
  className?: string;
}

/**
 * Skeleton — a banda de luz do POC (1,4 s) com a FORMA do conteúdo.
 * Nunca um rectângulo de 120 px: anel + 3 linhas + barra, como o card real.
 */
export function Skeleton({ shape = 'line', width, height, size, className }: SkeletonProps) {
  const style: CSSProperties = {};
  if (width !== undefined) style.width = width;
  if (height !== undefined) style.height = height;
  if (size !== undefined) {
    style.width = size;
    style.height = size;
  }

  return (
    <div
      className={cx('mc-skeleton', `mc-skeleton--${shape}`, className)}
      style={Object.keys(style).length > 0 ? style : undefined}
      aria-hidden="true"
    />
  );
}

export interface SkeletonSwapProps {
  loading: boolean;
  /** A forma do conteúdo que vai chegar. */
  fallback: ReactNode;
  children: ReactNode;
  /** "A ler a bateria" — o skeleton é a mensagem, isto é para o leitor. */
  label?: string;
}

/** Troca sem pop: o fallback desaparece quando os dados entram. */
export function SkeletonSwap({ loading, fallback, children, label }: SkeletonSwapProps) {
  if (loading) {
    return (
      <div className="mc-skeleton-swap" role="status" aria-busy="true" aria-live="polite">
        {label ? <span className="sr-only">{label}</span> : null}
        {fallback}
      </div>
    );
  }
  return <div className="mc-skeleton-swap">{children}</div>;
}

export default Skeleton;
