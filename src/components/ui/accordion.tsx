import type { ReactNode } from 'react';
import { Icon } from './icons';
import { cx } from './cx';

export interface AccordionProps {
  /** A linha sempre visível — o que a pessoa lê antes de decidir abrir. */
  summary: ReactNode;
  children: ReactNode;
  /** Abre de início (usa-se quando o conteúdo é a resposta, não a ajuda). */
  defaultOpen?: boolean;
  /** Identificador para `aria-controls` externo ou para testes. */
  id?: string;
  className?: string;
}

/**
 * Accordion — `details`/`summary` com a gramática hairline.
 *
 * Porque o nativo e não um `<div role="button">`: o `<summary>` já é focável,
 * já responde a Enter/Espaço, já é anunciado como "revelar" e já funciona com
 * o "encontrar na página" do browser. Um accordion em JS reconstrói tudo isso
 * pior. O que o nativo não faz — o marcador `▸` e o espaçamento da casa — é o
 * que o CSS resolve, não o componente.
 *
 * Sem sombra, sem plano novo: 1 px de `--hairline` entre o resumo e o corpo.
 */
export function Accordion({
  summary,
  children,
  defaultOpen = false,
  id,
  className,
}: AccordionProps) {
  return (
    <details id={id} className={cx('mc-accordion', className)} open={defaultOpen || undefined}>
      <summary className="mc-accordion__summary">
        <span className="mc-accordion__chev" aria-hidden="true">
          <Icon name="chevron-right" size={12} />
        </span>
        <span>{summary}</span>
      </summary>
      <div className="mc-accordion__body">{children}</div>
    </details>
  );
}

export default Accordion;
