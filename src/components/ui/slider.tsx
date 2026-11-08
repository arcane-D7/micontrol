import { useId } from 'react';
import type { ReactNode } from 'react';
import { cx } from './cx';

export interface SliderProps {
  label?: ReactNode;
  value: number;
  min?: number;
  max?: number;
  step?: number;
  onChange: (value: number) => void;
  /** `17,3` → `17,3 °C`. O valor vive ao lado da label, tabular. */
  format?: (value: number) => string;
  unit?: string;
  disabled?: boolean;
  ariaLabel?: string;
  id?: string;
  className?: string;
}

/**
 * Slider — `accent-color` pinta o fill do track e o thumb nativos.
 * Zero `style={{}}`, zero JS de posicionamento, zero sombra.
 */
export function Slider({
  label,
  value,
  min = 0,
  max = 100,
  step = 1,
  onChange,
  format,
  unit,
  disabled,
  ariaLabel,
  id,
  className,
}: SliderProps) {
  const autoId = useId();
  const inputId = id ?? autoId;
  const shown = format ? format(value) : String(value);

  return (
    <div className={cx('mc-slider', className)}>
      {label ? (
        <div className="mc-slider__head">
          <label className="mc-slider__label" htmlFor={inputId}>
            {label}
          </label>
          <span className="mc-slider__value">
            {shown}
            {unit ? <span className="mc-stat-row__unit"> {unit}</span> : null}
          </span>
        </div>
      ) : null}
      <input
        id={inputId}
        className="mc-slider__input"
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        aria-label={ariaLabel ?? (typeof label === 'string' ? label : undefined)}
        aria-valuetext={unit ? `${shown} ${unit}` : shown}
        onChange={(e) => onChange(Number(e.target.value))}
      />
    </div>
  );
}

export default Slider;
