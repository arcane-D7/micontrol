export type ClassValue = string | false | null | undefined;

/** Concatena classNames sem dependências (sem clsx, sem tailwind-merge). */
export function cx(...values: ClassValue[]): string {
  return values.filter(Boolean).join(' ');
}
