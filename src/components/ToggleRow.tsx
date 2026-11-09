import Switch from './ui/switch';

interface ToggleRowProps {
  label: string;
  desc?: string;
  /** Semântica escondida do controlo — vira um "i" ao lado do rótulo. */
  hint?: string;
  hintLabel?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}

/**
 * ToggleRow — agora uma linha do primitivo `Switch` (o dono único do controlo).
 * A API não mudou: `label`/`desc`/`checked`/`onChange`/`disabled`.
 */
export default function ToggleRow({
  label,
  desc,
  hint,
  hintLabel,
  checked,
  onChange,
  disabled,
}: ToggleRowProps) {
  return (
    <Switch
      label={label}
      desc={desc}
      hint={hint}
      hintLabel={hintLabel}
      checked={checked}
      onChange={onChange}
      disabled={disabled}
    />
  );
}
