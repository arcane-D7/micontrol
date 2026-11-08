import { useRef } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from 'react';
import { cx } from './cx';

export interface SegmentedOption<T extends string | number = number> {
  value: T;
  label: ReactNode;
  /** Selo dentro do item ("recomendado"). */
  badge?: ReactNode;
  disabled?: boolean;
}

export interface SegmentedProps<T extends string | number = number> {
  label?: ReactNode;
  options: ReadonlyArray<SegmentedOption<T>>;
  value: T;
  onChange: (value: T) => void;
  disabled?: boolean;
  /** Porque está desligado — nunca um clique morto (§4.3 #1). */
  disabledReason?: string;
  ariaLabel?: string;
  className?: string;
}

/**
 * Segmented — limite de carga, 40/50/60/70/80 + "recomendado".
 *
 * Seleccionado = `--accent-soft` + `--accent-ink` (5,81:1), NUNCA `--accent`
 * como fundo com texto branco (dava 3,59:1). Teclado: ←/→/Home/End.
 * `aria-pressed` é o contrato; a classe só pinta.
 */
export function Segmented<T extends string | number = number>({
  label,
  options,
  value,
  onChange,
  disabled,
  disabledReason,
  ariaLabel,
  className,
}: SegmentedProps<T>) {
  const refs = useRef<Array<HTMLButtonElement | null>>([]);

  const selectable = options
    .map((option, index) => (!option.disabled && !disabled ? index : -1))
    .filter((index) => index >= 0);

  const handleKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (!['ArrowRight', 'ArrowLeft', 'Home', 'End'].includes(e.key)) return;
    if (selectable.length === 0) return;
    e.preventDefault();

    const current = options.findIndex((option) => option.value === value);
    let next: number | undefined;

    if (e.key === 'Home') {
      next = selectable[0];
    } else if (e.key === 'End') {
      next = selectable[selectable.length - 1];
    } else {
      const position = selectable.indexOf(current);
      const step = e.key === 'ArrowRight' ? 1 : -1;
      const base = position === -1 ? 0 : position;
      next = selectable[(base + step + selectable.length) % selectable.length];
    }

    if (next === undefined) return;
    onChange(options[next].value);
    refs.current[next]?.focus();
  };

  return (
    <div className={cx('mc-segmented-group', className)}>
      {label ? <div className="mc-switch__label">{label}</div> : null}
      <div className="mc-segmented" role="group" aria-label={ariaLabel} onKeyDown={handleKeyDown}>
        {options.map((option, index) => {
          const pressed = option.value === value;
          return (
            <button
              key={String(option.value)}
              ref={(node) => {
                refs.current[index] = node;
              }}
              type="button"
              className="mc-segmented__item"
              aria-pressed={pressed}
              disabled={disabled || option.disabled}
              tabIndex={pressed ? 0 : -1}
              onClick={() => onChange(option.value)}
            >
              {option.label}
              {option.badge ? <span className="mc-segmented__badge">{option.badge}</span> : null}
            </button>
          );
        })}
      </div>
      {disabled && disabledReason ? (
        <div className="mc-segmented__note">{disabledReason}</div>
      ) : null}
    </div>
  );
}

export default Segmented;
