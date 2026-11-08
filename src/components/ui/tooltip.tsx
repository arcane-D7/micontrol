import { cloneElement, useEffect, useId, useRef, useState } from 'react';
import type { ReactElement } from 'react';
import { cx } from './cx';

export interface TooltipProps {
  /** O texto que substitui o `title=""` nativo. */
  label: string;
  children: ReactElement<{ 'aria-describedby'?: string }>;
  /** Atraso antes de abrir (ms). 0 em foco por teclado. */
  delay?: number;
  side?: 'top' | 'bottom';
  className?: string;
}

/**
 * Tooltip — alvo de 24 px, atraso, `aria-describedby`.
 * Sem sombra: o plano separa-se por cor invertida (`--ink` sobre `--surface`).
 * Isto é o que os 411 `title=""` do Gestor de Tarefas deviam ser.
 */
export function Tooltip({ label, children, delay = 400, side = 'top', className }: TooltipProps) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clear = () => {
    if (timer.current) {
      clearTimeout(timer.current);
      timer.current = null;
    }
  };

  useEffect(() => clear, []);

  const show = (immediate: boolean) => {
    clear();
    if (immediate) {
      setOpen(true);
      return;
    }
    timer.current = setTimeout(() => setOpen(true), delay);
  };

  const hide = () => {
    clear();
    setOpen(false);
  };

  return (
    <span
      className={cx('mc-tooltip', className)}
      onMouseEnter={() => show(false)}
      onMouseLeave={hide}
      onFocus={() => show(true)}
      onBlur={hide}
      onKeyDown={(e) => {
        if (e.key === 'Escape') hide();
      }}
    >
      {cloneElement(children, { 'aria-describedby': open ? id : undefined })}
      <span
        id={id}
        role="tooltip"
        className="mc-tooltip__bubble"
        data-open={open ? 'true' : 'false'}
        data-side={side}
        aria-hidden={open ? undefined : true}
      >
        {label}
      </span>
    </span>
  );
}

export default Tooltip;
