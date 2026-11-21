import { cx } from './cx';

export interface HairlineProps {
  /** 1 px em `--hairline-2` (separação mais forte, ainda sem sombra). */
  strong?: boolean;
  tight?: boolean;
  className?: string;
}

/**
 * Hairline — a única forma de separar planos que a casa aceita.
 * Ordem de recurso (theme-migration.md §2.6): espaço → `--surface-2` → hairline.
 */
export function Hairline({ strong, tight, className }: HairlineProps) {
  return (
    <hr
      className={cx(
        'mc-hairline',
        strong && 'mc-hairline--strong',
        tight && 'mc-hairline--tight',
        className,
      )}
    />
  );
}

export default Hairline;
