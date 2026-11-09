/**
 * Switch — `button role="switch"`, 40×22, thumb que estica no press (spec §5.4).
 *
 * Substitui `ToggleRow` + `ToggleSwitch` (dois donos do mesmo controlo) e
 * resolve o estado que faltava: `pending` (enquanto o EC grava) e
 * `disabledReason` (desligado **com razão visível**, em vez do
 * `opacity:.4 + pointer-events:none` mudo de hoje).
 *
 * `bare` + `ariaLabel` + `id` existem para os invólucros antigos
 * (`ToggleSwitch`) continuarem a compilar sobre este único dono do controlo.
 */

import { useId } from 'react';
import { InfoHint } from './info-hint';

export interface SwitchProps {
  label?: string;
  desc?: string;
  /** Semântica que o rótulo não diz — vira um "i" ao lado do nome. */
  hint?: string;
  /** Texto do `aria-label` do "i" quando o rótulo visível não chega. */
  hintLabel?: string;
  checked: boolean;
  onChange: (next: boolean) => void;
  /** A gravar no hardware — o controlo aceita o clique mas não o repete. */
  pending?: boolean;
  disabled?: boolean;
  /** Mostrado quando `disabled`: diz o que fazer, não só que não dá. */
  disabledReason?: string;
  /** Só o controlo, sem a linha label+descrição (ToggleSwitch). */
  bare?: boolean;
  /** Nome acessível quando não há `label` visível (modo `bare`). */
  ariaLabel?: string;
  /** Id do controlo, para associação externa. */
  id?: string;
}

export function Switch({
  label,
  desc,
  hint,
  hintLabel,
  checked,
  onChange,
  pending,
  disabled,
  disabledReason,
  bare = false,
  ariaLabel,
  id,
}: SwitchProps) {
  const generatedId = useId();
  const controlId = id ?? generatedId;
  const isDisabled = disabled || pending;

  const control = (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-labelledby={!bare && label ? controlId : undefined}
      aria-label={bare ? ariaLabel : undefined}
      aria-busy={pending || undefined}
      className={`mc-switch${checked ? ' mc-switch--on' : ''}${pending ? ' mc-switch--pending' : ''}`}
      disabled={isDisabled}
      onClick={() => onChange(!checked)}
    >
      <span className="mc-switch__thumb" />
    </button>
  );

  if (bare) return control;

  return (
    <div className="mc-switchrow">
      <div className="mc-switchrow__text">
        <div className="mc-switchrow__labelrow">
          <span className="mc-switchrow__label" id={controlId}>
            {label}
          </span>
          {hint && <InfoHint label={hint} buttonLabel={hintLabel} />}
        </div>
        {desc && <div className="mc-switchrow__desc">{desc}</div>}
        {isDisabled && disabledReason && (
          <div className="mc-switchrow__reason">{disabledReason}</div>
        )}
      </div>
      {control}
    </div>
  );
}

export default Switch;
