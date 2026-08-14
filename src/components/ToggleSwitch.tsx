interface ToggleSwitchProps {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
  ariaLabel?: string;
  id?: string;
}

/**
 * A standalone themed toggle switch (track + knob), matching the
 * `.toggle-switch` / `.toggle-track` / `.toggle-knob` CSS classes used by
 * `ToggleRow`. Use it anywhere a bare `<input type="checkbox">` would
 * otherwise render with the ugly native checkbox.
 */
export default function ToggleSwitch({
  checked,
  onChange,
  disabled,
  ariaLabel,
  id,
}: ToggleSwitchProps) {
  return (
    <label className="toggle-switch" style={{ opacity: disabled ? 0.55 : 1 }} title={ariaLabel}>
      <input
        id={id}
        type="checkbox"
        checked={checked}
        disabled={disabled}
        aria-label={ariaLabel}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span className="toggle-track" />
      <span className="toggle-knob" />
    </label>
  );
}
