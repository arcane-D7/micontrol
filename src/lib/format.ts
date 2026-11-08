/**
 * Formatação numérica pt-PT da tab Bateria (spec §5.7).
 *
 * Regras:
 *  - vírgula decimal: `17,28 V`, nunca `17.28 V`;
 *  - espaço não separável entre número e unidade (o valor e a unidade nunca
 *    quebram para linhas diferentes);
 *  - `0` / `null` nunca produzem um número falso — devolvem string vazia ou `—`.
 *
 * O `Intl.NumberFormat` é a fonte, mas há um fallback manual: um Node/WebView
 * sem dados de locale cai para `en` e devolveria `17.28`. O teste fixa os dois.
 */

const NBSP = '\u00a0';
const NO_DATA = '—';

function commaSeparatorWorks(): boolean {
  try {
    return new Intl.NumberFormat('pt-PT', { minimumFractionDigits: 1 }).format(1.5) === '1,5';
  } catch {
    return false;
  }
}

const PT_OK = commaSeparatorWorks();

/** `17.28` → `17,28` (com o número de casas pedido). */
export function formatDecimal(value: number, digits = 1): string {
  if (!Number.isFinite(value)) return NO_DATA;
  if (PT_OK) {
    return new Intl.NumberFormat('pt-PT', {
      minimumFractionDigits: digits,
      maximumFractionDigits: digits,
    }).format(value);
  }
  return value.toFixed(digits).replace('.', ',');
}

/** Milivolts → `17,28 V`. Devolve `—` quando não há leitura (<= 0). */
export function formatVoltage(mv: number | null | undefined): string {
  if (mv == null || mv <= 0) return NO_DATA;
  return `${formatDecimal(mv / 1000, 2)}${NBSP}V`;
}

/** Milliwatts → `65 W` (inteiro, como o adaptador reporta). */
export function formatWatts(mw: number | null | undefined): string {
  if (mw == null || mw <= 0) return NO_DATA;
  return `${formatDecimal(mw / 1000, 0)}${NBSP}W`;
}

/** Milliwatt-hora → `68,2 Wh`. */
export function formatWh(mwh: number | null | undefined): string {
  if (mwh == null || mwh <= 0) return NO_DATA;
  return `${formatDecimal(mwh / 1000, 1)}${NBSP}Wh`;
}

/** Celsius → `31,4 °C`. */
export function formatCelsius(c: number | null | undefined): string {
  if (c == null) return NO_DATA;
  return `${formatDecimal(c, 1)}${NBSP}°C`;
}

/** Minutos → `1 h 20 min` / `45 min`; `null` → string vazia. */
export function formatDuration(minutes: number | null | undefined): string {
  if (minutes == null || minutes < 0) return '';
  const h = Math.floor(minutes / 60);
  const m = Math.round(minutes % 60);
  if (h === 0) return `${m} min`;
  return m === 0 ? `${h} h` : `${h} h ${m} min`;
}

/** Percentagem já arredondada pelo backend → `95%`. */
export function formatPercent(pct: number): string {
  return `${Math.round(pct)}%`;
}

export { NO_DATA as NO_DATA_LABEL };
