/**
 * Segmented — selector de limite 40/50/60/70/80/100 (spec §5.4, §5.6.3).
 *
 * Contraste: seleccionado = `--accent-soft` + `--accent-ink` + 1 px
 * `--accent` (5,81:1); não seleccionado = `--surface-2` + `--ink-muted`;
 * hover = `--hairline-2`. **Nunca** `--accent` como fundo com texto branco
 * (daria 3,59:1).
 *
 * Teclado: setas ←/→ movem e aplicam (roving tabindex), Home/End nas pontas.
 * `disabledReason` é a parte que hoje falta: o grupo não fica inerte sem
 * explicação — mostra a razão e um caminho para agir.
 */

import { useId, useRef } from 'react';

export interface SegmentedOption {
  value: number;
  label: string;
  /** Selo dentro do item (ex.: "recomendado"). */
  badge?: string;
}

export interface SegmentedProps {
  label: string;
  options: SegmentedOption[];
  value: number;
  onChange: (value: number) => void;
  /** A escrever no EC: o item escolhido mostra-se mas não aceita repetição. */
  pending?: boolean;
  /** Presente = o grupo está desactivado **com razão** (não há cliques mortos). */
  disabledReason?: string;
  /** Clique num item desactivado: aplica-o ligando o master switch (1 gesto). */
  onDisabledSelect?: (value: number) => void;
  /** Clique na razão mostrada por baixo do grupo. */
  onDisabledClick?: () => void;
}

export function Segmented({
  label,
  options,
  value,
  onChange,
  pending,
  disabledReason,
  onDisabledSelect,
  onDisabledClick,
}: SegmentedProps) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const labelId = useId();
  const disabled = Boolean(disabledReason);
  const currentIndex = Math.max(
    0,
    options.findIndex((o) => o.value === value),
  );

  const move = (delta: number) => {
    if (disabled || pending) return;
    const next = (currentIndex + delta + options.length) % options.length;
    onChange(options[next].value);
    refs.current[next]?.focus();
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown') {
      e.preventDefault();
      move(1);
    } else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') {
      e.preventDefault();
      move(-1);
    } else if (e.key === 'Home') {
      e.preventDefault();
      move(-currentIndex);
    } else if (e.key === 'End') {
      e.preventDefault();
      move(options.length - 1 - currentIndex);
    }
  };

  return (
    <div className={`mc-seg${disabled ? ' mc-seg--disabled' : ''}`}>
      <div className="mc-seg__label" id={labelId}>
        {label}
      </div>
      <div
        className="mc-seg__group"
        role="radiogroup"
        aria-labelledby={labelId}
        aria-busy={pending || undefined}
        onKeyDown={onKeyDown}
      >
        {options.map((opt, i) => {
          const selected = opt.value === value;
          return (
            <button
              key={opt.value}
              ref={(el) => {
                refs.current[i] = el;
              }}
              type="button"
              role="radio"
              aria-checked={selected}
              tabIndex={selected ? 0 : -1}
              className={`mc-seg__item${selected ? ' mc-seg__item--on' : ''}`}
              disabled={pending}
              onClick={() => {
                if (disabled) {
                  onDisabledSelect?.(opt.value);
                  return;
                }
                onChange(opt.value);
              }}
            >
              <span className="mc-seg__value">{opt.label}</span>
              {opt.badge && <span className="mc-seg__badge">{opt.badge}</span>}
            </button>
          );
        })}
      </div>
      {disabledReason && (
        <button type="button" className="mc-seg__reason" onClick={onDisabledClick}>
          {disabledReason}
        </button>
      )}
    </div>
  );
}
