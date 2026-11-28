import Switch from './ui/switch';

interface ToggleSwitchProps {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
  ariaLabel?: string;
  id?: string;
}

/**
 * ToggleSwitch — o mesmo `Switch`, sem a linha label+descrição.
 * Existe para não partir as chamadas que já usam este nome; os dois donos do
 * mesmo controlo passam a ser um só (`ui/switch`).
 */
export default function ToggleSwitch({
  checked,
  onChange,
  disabled,
  ariaLabel,
  id,
}: ToggleSwitchProps) {
  return (
    <Switch
      checked={checked}
      onChange={onChange}
      disabled={disabled}
      ariaLabel={ariaLabel}
      id={id}
      bare
    />
  );
}
