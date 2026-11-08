/**
 * Switch — `button role="switch"`, 40×22, thumb que estica no press (spec §5.4).
 *
 * Substitui `ToggleRow` + `ToggleSwitch` (dois donos do mesmo controlo) e
 * resolve o estado que faltava: `pending` (enquanto o EC grava) e
 * `disabledReason` (desligado **com razão visível**, em vez do
 * `opacity:.4 + pointer-events:none` mudo de hoje).
 */

import { useId } from 'react';

export interface SwitchProps {
  label: string;
  desc?: string;
  checked: boolean;
  onChange: (next: boolean) => void;
  /** A gravar no hardware — o controlo aceita o clique mas não o repete. */
  pending?: boolean;
  disabled?: boolean;
  /** Mostrado quando `disabled`: diz o que fazer, não só que não dá. */
  disabledReason?: string;
}

export function Switch({
  label,
  desc,
  checked,
  onChange,
  pending,
  disabled,
  disabledReason,
}: SwitchProps) {
  const id = useId();
  const isDisabled = disabled || pending;

  return (
    <div className="mc-switchrow">
      <div className="mc-switchrow__text">
        <div className="mc-switchrow__label" id={id}>
          {label}
        </div>
        {desc && <div className="mc-switchrow__desc">{desc}</div>}
        {isDisabled && disabledReason && (
          <div className="mc-switchrow__reason">{disabledReason}</div>
        )}
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        aria-labelledby={id}
        aria-busy={pending || undefined}
        className={`mc-switch${checked ? ' mc-switch--on' : ''}${pending ? ' mc-switch--pending' : ''}`}
        disabled={isDisabled}
        onClick={() => onChange(!checked)}
      >
        <span className="mc-switch__thumb" />
      </button>
    </div>
  );
}
