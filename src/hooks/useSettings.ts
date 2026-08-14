/**
 * useSettings — settings persistence and API key management.
 *
 * Refactored in S28-004:
 * - AI prompt building → `src/lib/aiPromptBuilder.ts`
 * - AI analysis functions → `src/hooks/useAiAnalysis.ts`
 * - Telemetry consent → `src/hooks/useTelemetryConsent.ts`
 *
 * This hook composes the sub-hooks internally so that existing callers
 * continue to receive the same return shape (backward compatible).
 * New code should import the focused hooks directly.
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { AppSettings } from '../types/settings';
import { useAiAnalysis } from './useAiAnalysis';
import { useTelemetryConsent } from './useTelemetryConsent';

// ── Types (re-exported from src/types/settings.ts for backward compat) ────────

export type { AppSettings, SystemContext, AnalysisLogEntry } from '../types/settings';

// ── Re-exports for direct import ──────────────────────────────────────────────

export { useAiAnalysis } from './useAiAnalysis';
export { useTelemetryConsent } from './useTelemetryConsent';
export type { TelemetryConsentValue } from './useTelemetryConsent';
export { buildPrompt, buildLogPrompt } from '../lib/aiPromptBuilder';

// ── Persisted settings ────────────────────────────────────────────────────────

const STORAGE_KEY = 'micontrol_settings_v2';
const STORAGE_KEY_V1 = 'micontrol_settings_v1';
const CREDENTIAL_KEY = 'openai_api_key';

export const DEFAULT_SETTINGS: AppSettings = {
  onboardingCompleted: false,
  openai_api_key: '',
  openai_base_url: 'https://api.openai.com/v1',
  openai_model: 'gpt-4o-mini',
  perf_mode_ac: null,
  perf_mode_dc: null,
  auto_switch_perf: false,
  tray_opacity: 1.0,
  ai_analysis_enabled: false,
  ai_poll_interval_sec: 60,
  ai_daily_analyses: 2,
  mcp_integration_enabled: false,
};

/**
 * Load non-secret settings from localStorage.
 */
function loadSettings(): AppSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY) ?? localStorage.getItem(STORAGE_KEY_V1);
    return raw ? { ...DEFAULT_SETTINGS, ...JSON.parse(raw) } : DEFAULT_SETTINGS;
  } catch {
    return DEFAULT_SETTINGS;
  }
}

/** Persist non-secret settings to localStorage (API key excluded). */
function persistSettings(settings: AppSettings): void {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  const { openai_api_key, ...safe } = settings;
  localStorage.setItem(STORAGE_KEY, JSON.stringify(safe));
}

/**
 * Migrate an API key stored in the old localStorage JSON blob to the
 * OS credential store, then remove it from localStorage. Idempotent.
 */
async function migrateApiKey(): Promise<void> {
  try {
    for (const key of [STORAGE_KEY, STORAGE_KEY_V1]) {
      const raw = localStorage.getItem(key);
      if (!raw) continue;
      let parsed: Record<string, unknown>;
      try {
        parsed = JSON.parse(raw);
      } catch {
        continue;
      }
      const oldKey = parsed.openai_api_key;
      if (typeof oldKey === 'string' && oldKey.trim().length > 0) {
        await invoke('set_secret', { key: CREDENTIAL_KEY, value: oldKey });
        delete parsed.openai_api_key;
        localStorage.setItem(key, JSON.stringify(parsed));
        console.info('[settings] Migrated OpenAI API key to secure credential store');
      }
    }
  } catch (err) {
    console.warn('[settings] API key migration failed (non-fatal):', err);
  }
}

/**
 * Fetch the API key from the OS credential store.
 * Returns empty string if no key is stored or on error.
 */
async function loadApiKey(): Promise<string> {
  try {
    const result = await invoke<string | null>('get_secret', { key: CREDENTIAL_KEY });
    return result ?? '';
  } catch {
    return '';
  }
}

// ── Hook ──────────────────────────────────────────────────────────────────────

export function useSettings() {
  const [settings, setSettingsState] = useState<AppSettings>(() => ({
    ...loadSettings(),
    openai_api_key: '', // loaded asynchronously below
  }));

  // Compose focused sub-hooks
  const ai = useAiAnalysis(settings);
  const telemetry = useTelemetryConsent();

  /** Persist non-secret settings to localStorage and API key to credential store. */
  const saveSettings = useCallback(async (updated: AppSettings): Promise<void> => {
    setSettingsState(updated);
    persistSettings(updated);
    if (updated.openai_api_key) {
      try {
        await invoke('set_secret', { key: CREDENTIAL_KEY, value: updated.openai_api_key });
      } catch (err) {
        console.error('[settings] Failed to store API key in credential store:', err);
      }
    } else {
      try {
        await invoke('delete_secret', { key: CREDENTIAL_KEY });
      } catch {
        // Ignore — key may not exist yet
      }
    }
  }, []);

  const updateKey = useCallback(
    <K extends keyof AppSettings>(key: K, value: AppSettings[K]) => {
      if (key === 'openai_api_key') {
        // Update locally immediately for responsiveness; background-save to credential store
        const updated = { ...settings, [key]: value };
        setSettingsState(updated);
        persistSettings(updated);
        saveSettings(updated).catch((err) =>
          console.error('[settings] Failed to save API key:', err),
        );
      } else {
        void saveSettings({ ...settings, [key]: value });
      }
    },
    [settings, saveSettings],
  );

  // On mount: migrate legacy localStorage key if present, then load from credential store
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      await migrateApiKey();
      if (cancelled) return;
      const apiKey = await loadApiKey();
      if (cancelled) return;
      if (apiKey !== settings.openai_api_key) {
        setSettingsState((prev) => ({ ...prev, openai_api_key: apiKey }));
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const setOnboardingCompleted = useCallback(
    (completed: boolean): void => {
      updateKey('onboardingCompleted', completed);
    },
    [updateKey],
  );

  return useMemo(
    () => ({
      settings,
      saveSettings,
      updateKey,
      setOnboardingCompleted,
      // AI analysis (delegated to useAiAnalysis)
      analyzeSystem: ai.analyzeSystem,
      analyzeWithLogs: ai.analyzeWithLogs,
      testConnection: ai.testConnection,
      isConfigured: Boolean(settings.openai_api_key.trim()),
      // Telemetry consent (delegated to useTelemetryConsent)
      getTelemetryConsent: telemetry.getTelemetryConsent,
      setTelemetryConsent: telemetry.setTelemetryConsent,
      revokeTelemetryConsent: telemetry.revokeTelemetryConsent,
      checkTelemetryConsent: telemetry.checkTelemetryConsent,
    }),
    [settings, ai, telemetry, saveSettings, updateKey, setOnboardingCompleted],
  );
}
