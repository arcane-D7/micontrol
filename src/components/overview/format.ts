/**
 * Formatação da tab Overview.
 *
 * Regras da casa herdadas da tab Bateria (`src/lib/format.ts`): `null`/`0`
 * nunca produzem um número falso — devolvem `—`. A diferença é o locale: o
 * resumo é a única superfície que mostra CPU, RAM, disco e rede juntos, e um
 * disco "952.9 GB" ao lado de uma bateria "17,37 V" lia-se como dois idiomas.
 * Tudo aqui usa o locale activo; o espaço antes da unidade é não separável para
 * o valor nunca quebrar de linha sozinho.
 */

import { getLanguage } from '../../hooks/useI18n';

const NBSP = '\u00a0';

/** O valor honesto para "o hardware não reporta isto". */
export const EMPTY = '—';

function formatter(digits: number): Intl.NumberFormat {
  try {
    return new Intl.NumberFormat(getLanguage(), {
      minimumFractionDigits: digits,
      maximumFractionDigits: digits,
    });
  } catch {
    return new Intl.NumberFormat('en', {
      minimumFractionDigits: digits,
      maximumFractionDigits: digits,
    });
  }
}

/** `1234.5` → `1 234,5` (locale activo). `null`/`NaN` → `—`. */
export function fmtNumber(value: number | null | undefined, digits = 0): string {
  if (value == null || !Number.isFinite(value)) return EMPTY;
  return formatter(digits).format(value);
}

/** Valor + unidade com espaço não separável: `16${NBSP}GB`. */
export function fmtUnit(value: number | null | undefined, unit: string, digits = 0): string {
  if (value == null || !Number.isFinite(value)) return EMPTY;
  return `${formatter(digits).format(value)}${NBSP}${unit}`;
}

/** Texto já formatado pelo backend, ou `—`. */
export function fmtText(value: string | null | undefined): string {
  const trimmed = value?.trim();
  return trimmed ? trimmed : EMPTY;
}

/**
 * Segundos → `3 d 4 h` / `4 h 12 min` / `<1 min`.
 *
 * As abreviaturas (`d`, `h`, `min`) são as mesmas nos quatro locales da app,
 * por isso não passam pelo i18n — traduzir "d" só criaria divergência.
 */
export function fmtUptime(totalSecs: number | null | undefined): string {
  if (totalSecs == null || !Number.isFinite(totalSecs) || totalSecs < 0) return EMPTY;
  const days = Math.floor(totalSecs / 86_400);
  const hours = Math.floor((totalSecs % 86_400) / 3_600);
  const minutes = Math.floor((totalSecs % 3_600) / 60);
  if (days > 0) return hours > 0 ? `${days} d ${hours} h` : `${days} d`;
  if (hours > 0) return minutes > 0 ? `${hours} h ${minutes} min` : `${hours} h`;
  if (minutes > 0) return `${minutes} min`;
  return '<1 min';
}

/** `2026-07-24` → `24/07/2026` (ordem do locale activo). */
export function fmtIsoDate(iso: string | null | undefined): string {
  if (!iso) return EMPTY;
  const [y, m, d] = iso.split('-').map(Number);
  if (!y || !m || !d) return EMPTY;
  const date = new Date(Date.UTC(y, m - 1, d));
  if (Number.isNaN(date.getTime())) return EMPTY;
  try {
    return new Intl.DateTimeFormat(getLanguage(), {
      day: '2-digit',
      month: '2-digit',
      year: 'numeric',
      timeZone: 'UTC',
    }).format(date);
  } catch {
    return iso;
  }
}

/** `3120` × `2080` → `3120 × 2080`. `0` (não lido) → `—`.
 *
 *  Sem separador de milhares de propósito: uma resolução não é uma quantidade
 *  ("3,120" lê-se como três vírgula doze). */
export function fmtResolution(width: number, height: number): string {
  if (!width || !height) return EMPTY;
  return `${width} × ${height}`;
}
