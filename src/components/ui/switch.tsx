import { useId } from 'react';
import type { ReactNode } from 'react';
import { cx } from './cx';

export interface SwitchProps {
  checked: boolean;
  onChange: (value: boolean) => void;
  label?: ReactNode;
  desc?: ReactNode;
  disabled?: boolean;
  /** Escrita em curso: o track pulsa, o input fica bloqueado. */
  pending?: boolean;
  /** Porque está desligado. Nunca um controlo morto sem explicação. */
  disabledReason?: string;
  ariaLabel?: string;
  id?: string;
  /** Só o controlo, sem a linha label+descrição (ToggleSwitch). */
  bare?: boolean;
  className?: string;
}

/**
 * Switch — o dono único do controlo (substitui ToggleRow + ToggleSwitch).
 * Track 40×22, thumb 16, thumb estica 4 px no press e volta com EASE_THROW.
 * O input é um checkbox real: teclado, `Space`, e leitor de ecrã saem de graça.
 */
export function Switch({
  checked,
  onChange,
  label,
  desc,
  disabled,
  pending,
  disabledReason,
  ariaLabel,
  id,
  bare,
  className,
}: SwitchProps) {
  const autoId = useId();
  const inputId = id ?? autoId;
  const descId = `${inputId}-desc`;
  const locked = disabled || pending;
  /* O nome acessível é o label, não "label + descrição" colados. */
  const name = bare ? ariaLabel : typeof label === 'string' ? label : ariaLabel;

  return (
    <div className={cx('mc-switch-wrap', className)}>
      <label
        className={cx(
          'mc-switch',
          bare && 'mc-switch--bare',
          locked && 'mc-switch--disabled',
          pending && 'mc-switch--pending',
        )}
        htmlFor={inputId}
      >
        {!bare && (label || desc) ? (
          <span className="mc-switch__text">
            {label ? <span className="mc-switch__label">{label}</span> : null}
            {desc ? (
              <span className="mc-switch__desc" id={descId}>
                {desc}
              </span>
            ) : null}
          </span>
        ) : null}
        <span className="mc-switch__control">
          <input
            id={inputId}
            className="mc-switch__input"
            type="checkbox"
            role="switch"
            checked={checked}
            disabled={locked}
            aria-label={name}
            aria-describedby={desc ? descId : undefined}
            onChange={(e) => onChange(e.target.checked)}
          />
          <span className="mc-switch__track" aria-hidden="true" />
          <span className="mc-switch__knob" aria-hidden="true" />
        </span>
      </label>
      {disabled && disabledReason ? (
        <div className="mc-switch__reason">{disabledReason}</div>
      ) : null}
    </div>
  );
}

export default Switch;
