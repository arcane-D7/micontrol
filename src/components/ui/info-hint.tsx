import { useEffect, useId, useRef, useState } from 'react';
import { Icon } from './icons';
import { cx } from './cx';

export interface InfoHintProps {
  /** O que o controlo faz que o rótulo não diz. Frase curta, sem repetir o rótulo. */
  label: string;
  /** Nome acessível do gatilho quando o rótulo visível não está associado. */
  buttonLabel?: string;
  /**
   * `right` por omissão: o gatilho vive no fim de um rótulo, à esquerda há
   * texto e o painel abre no espaço vazio da linha em vez de tapar o controlo
   * que está a ser explicado. `bottom` para linhas com botões à direita.
   */
  side?: 'top' | 'bottom' | 'right' | 'left';
  className?: string;
}

/**
 * InfoHint — o "i" de um controlo cuja semântica não cabe no rótulo.
 *
 * Não é o `Tooltip`: o `Tooltip` é hover-first e serve para *um* número ou um
 * nome. Este abre por **clique e por foco de teclado** (nunca só hover), fecha
 * com Escape ou clique fora, e liga-se ao gatilho por `aria-describedby`
 * enquanto está aberto — por isso um leitor de ecrã lê a explicação, e um
 * utilizador de teclado que faz Tab para o "i" recebe-a sem clicar.
 *
 * Regra da casa: um hint por controlo, no máximo. Se a explicação precisa de
 * passos ou de um diagrama, não é um hint — é um `Accordion`.
 */
export function InfoHint({ label, buttonLabel, side = 'right', className }: InfoHintProps) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLSpanElement>(null);
  /**
   * O foco que abre o painel é o mesmo gesto que o clique vai consumir: sem
   * isto, o primeiro clique num botão por focar abria e fechava no mesmo
   * instante (focus → abre, click → alterna → fecha). Marcamos o foco e
   * deixamos o clique que o segue ser um no-op.
   */
  const openedByFocus = useRef(false);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: PointerEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) {
        setOpen(false);
        openedByFocus.current = false;
      }
    };
    document.addEventListener('pointerdown', onPointerDown);
    return () => document.removeEventListener('pointerdown', onPointerDown);
  }, [open]);

  return (
    <span
      ref={rootRef}
      className={cx('mc-infohint', className)}
      onKeyDown={(e) => {
        if (e.key === 'Escape' && open) {
          e.stopPropagation();
          setOpen(false);
          openedByFocus.current = false;
        }
      }}
    >
      <button
        type="button"
        className="mc-infohint__btn"
        aria-label={buttonLabel ?? label}
        aria-expanded={open}
        aria-describedby={open ? id : undefined}
        onClick={() => {
          if (openedByFocus.current) {
            openedByFocus.current = false;
            return;
          }
          setOpen((v) => !v);
        }}
        onFocus={() => {
          if (!open) {
            openedByFocus.current = true;
            setOpen(true);
          }
        }}
        onBlur={() => {
          openedByFocus.current = false;
          setOpen(false);
        }}
      >
        <Icon name="info" size={14} />
      </button>
      <span
        id={id}
        role="tooltip"
        className="mc-infohint__panel"
        data-open={open ? 'true' : 'false'}
        data-side={side}
        aria-hidden={open ? undefined : true}
      >
        {label}
      </span>
    </span>
  );
}

export default InfoHint;
