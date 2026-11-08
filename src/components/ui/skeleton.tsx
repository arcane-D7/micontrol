/**
 * Skeleton — **com a forma do conteúdo** (spec §5.5, estado 1).
 *
 * O `BatteryInfo.tsx:50` de hoje desenha um `.skeleton` de `height:120` — um
 * rectângulo que não se parece com nada e salta quando os dados chegam. Aqui o
 * skeleton desenha o herói (anel/número), três linhas a 60/40/28% e a barra de
 * 8 px, pela mesma ordem e com a mesma altura do conteúdo real (CLS 0).
 *
 * A banda de luz anima a 1,6 s e desaparece com `prefers-reduced-motion`.
 */

import type { ReactNode } from 'react';

export interface SkeletonProps {
  shape?: SkeletonShape;
  /** `ring`: diâmetro. `block`: altura. */
  size?: number;
  /** Largura — número (px) ou percentagem. */
  width?: string | number;
}

export function Skeleton({ shape = 'line', size, width }: SkeletonProps) {
  if (shape === 'ring') {
    const d = size ?? 92;
    return (
      <span
        className="mc-skeleton mc-skeleton--ring"
        style={{ width: d, height: d }}
        aria-hidden="true"
      />
    );
  }

  if (shape === 'block') {
    return (
      <span
        className="mc-skeleton mc-skeleton--block"
        style={{ height: size ?? 40, width: width ?? '100%' }}
        aria-hidden="true"
      />
    );
  }

  return (
    <span
      className="mc-skeleton mc-skeleton--line"
      style={{ width: width ?? '100%' }}
      aria-hidden="true"
    />
  );
}

/** Skeleton do card de bateria: herói + 3 linhas + barra — a forma do real. */
export function BatterySkeleton({ label }: { label: string }) {
  return (
    <div className="mc-batskeleton" role="status" aria-busy="true" aria-live="polite">
      <span className="mc-sr-only">{label}</span>
      <div className="mc-batskeleton__hero">
        <Skeleton shape="ring" size={92} />
        <div className="mc-batskeleton__lines">
          <Skeleton shape="line" width="60%" />
          <Skeleton shape="line" width="40%" />
          <Skeleton shape="line" width="28%" />
        </div>
      </div>
      <div style={{ height: 12 }} />
      <Skeleton shape="block" size={8} />
    </div>
  );
}

/** Alias do tema: as formas que o `Skeleton` desenha. */
export type SkeletonShape = 'ring' | 'line' | 'block';

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
        {label ? <span className="mc-sr-only">{label}</span> : null}
        {fallback}
      </div>
    );
  }
  return <div className="mc-skeleton-swap">{children}</div>;
}

export default Skeleton;
