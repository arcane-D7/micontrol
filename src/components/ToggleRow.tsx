import Switch from './ui/switch';

interface ToggleRowProps {
  label: string;
  desc?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}

/**
 * ToggleRow — agora uma linha do primitivo `Switch` (o dono único do controlo).
 * A API não mudou: `label`/`desc`/`checked`/`onChange`/`disabled`.
 */
export default function ToggleRow({ label, desc, checked, onChange, disabled }: ToggleRowProps) {
  return (
    <Switch label={label} desc={desc} checked={checked} onChange={onChange} disabled={disabled} />
  );
}
