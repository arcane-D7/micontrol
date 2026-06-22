interface ToggleRowProps {
  label: string;
  desc?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}

export default function ToggleRow({ label, desc, checked, onChange, disabled }: ToggleRowProps) {
  return (
    <div
      className="toggle"
      role="switch"
      tabIndex={disabled ? -1 : 0}
      aria-checked={checked}
      aria-label={label}
      onClick={() => {
        if (!disabled) onChange(!checked);
      }}
      onKeyDown={(e) => {
        if (!disabled && (e.key === 'Enter' || e.key === ' ')) {
          e.preventDefault();
          onChange(!checked);
        }
      }}
    >
      <div className="toggle-info">
        <div className="toggle-name">{label}</div>
        {desc && <div className="toggle-desc">{desc}</div>}
      </div>
      <label className="toggle-switch" onClick={(e) => e.stopPropagation()}>
        <input
          type="checkbox"
          checked={checked}
          disabled={disabled}
          onChange={(e) => onChange(e.target.checked)}
        />
        <span className="toggle-track" />
        <span className="toggle-knob" />
      </label>
    </div>
  );
}
