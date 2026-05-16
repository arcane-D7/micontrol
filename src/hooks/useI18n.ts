import { useSyncExternalStore } from "react";
import en from "../i18n/en.json";
import pt from "../i18n/pt.json";
import es from "../i18n/es.json";
import fr from "../i18n/fr.json";

// ── Locale registry ──────────────────────────────────────────────────────────

export const LOCALES = { en, pt, es, fr } as const;
export type Locale = keyof typeof LOCALES;

export const SUPPORTED_LOCALES: { code: Locale; nativeLabel: string }[] = [
  { code: "en", nativeLabel: "English" },
  { code: "pt", nativeLabel: "Português" },
  { code: "es", nativeLabel: "Español" },
  { code: "fr", nativeLabel: "Français" },
];

const LANG_KEY = "micontrol_lang";

// ── Type helpers (keys always derived from English as source of truth) ───────

type Strings = typeof en;
type NestedKeyOf<T, Prefix extends string = ""> = T extends object
  ? {
      [K in keyof T]: K extends string
        ? T[K] extends object
          ? NestedKeyOf<T[K], `${Prefix}${K}.`>
          : `${Prefix}${K}`
        : never;
    }[keyof T]
  : never;

export type StringKey = NestedKeyOf<Strings>;

// ── Module-level mutable state ───────────────────────────────────────────────

function detectLocale(): Locale {
  try {
    const stored = localStorage.getItem(LANG_KEY) as Locale | null;
    if (stored && stored in LOCALES) return stored;
    const nav = navigator.language.split("-")[0] as Locale;
    return nav in LOCALES ? nav : "en";
  } catch {
    return "en";
  }
}

let _locale: Locale = detectLocale();
let _strings: Record<string, unknown> = LOCALES[_locale] as Record<string, unknown>;
const _listeners = new Set<() => void>();

function _subscribe(listener: () => void) {
  _listeners.add(listener);
  return () => { _listeners.delete(listener); };
}

function _snapshot() { return _locale; }

// ── Public API ───────────────────────────────────────────────────────────────

export function setLanguage(lang: Locale) {
  if (!(lang in LOCALES)) return;
  _locale = lang;
  _strings = LOCALES[lang] as Record<string, unknown>;
  try { localStorage.setItem(LANG_KEY, lang); } catch { /* ignore */ }
  _listeners.forEach((l) => l());
}

export function getLanguage(): Locale { return _locale; }

function getNestedValue(obj: unknown, path: string): string {
  const parts = path.split(".");
  let current: unknown = obj;
  for (const part of parts) {
    if (current == null || typeof current !== "object") {
      // Fall back to English
      let fb: unknown = en;
      for (const p of parts) {
        if (fb == null || typeof fb !== "object") return path;
        fb = (fb as Record<string, unknown>)[p];
      }
      return typeof fb === "string" ? fb : path;
    }
    current = (current as Record<string, unknown>)[part];
  }
  if (typeof current !== "string") {
    // Key exists in structure but value is wrong — fall back
    let fb: unknown = en;
    for (const p of parts) {
      if (fb == null || typeof fb !== "object") return path;
      fb = (fb as Record<string, unknown>)[p];
    }
    return typeof fb === "string" ? fb : path;
  }
  return current;
}

/** Translate a key with optional variable interpolation. */
export function t(key: StringKey, vars?: Record<string, string>): string {
  let value = getNestedValue(_strings, key);
  if (vars) {
    for (const [k, v] of Object.entries(vars)) {
      value = value.replace(`{${k}}`, v);
    }
  }
  return value;
}

/** Reactive hook — subscribe to locale changes. Call in top-level component. */
export function useLanguage() {
  const locale = useSyncExternalStore(_subscribe, _snapshot);
  return { locale, setLanguage, supported: SUPPORTED_LOCALES };
}

export function useI18n() {
  return { t };
}
