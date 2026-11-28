import { Accordion } from './ui/accordion';

interface FunctionExplainProps {
  /** Short inline description shown next to the control. */
  summary: string;
  /** Detailed explanation shown when expanded. */
  details: string;
  /** Optional extra bullet points (each rendered as a list item). */
  bullets?: string[];
  /** Optional "learn more" / behavior note. */
  note?: string;
}

/**
 * Collapsible "how this works" explanation used on hardware controls that are
 * otherwise a single toggle/button.
 *
 * O corpo passou a ser o `Accordion` da casa (details/summary + hairline) em
 * vez de um `<div>` com estilos inline: isto era o único consumidor do padrão
 * e trazia bordas/raios próprios a fugir aos tokens. A API não mudou.
 */
export default function FunctionExplain({ summary, details, bullets, note }: FunctionExplainProps) {
  return (
    <Accordion summary={summary} className="mc-function-explain">
      <p>{details}</p>
      {bullets && bullets.length > 0 && (
        <ul>
          {bullets.map((b, i) => (
            <li key={i}>{b}</li>
          ))}
        </ul>
      )}
      {note && <p className="mc-accordion__note">{note}</p>}
    </Accordion>
  );
}
