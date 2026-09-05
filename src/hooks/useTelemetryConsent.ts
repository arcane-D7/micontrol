/**
 * useTelemetryConsent
 *
 * Manages telemetry consent state in the OS credential store.
 * Extracted from `src/hooks/useSettings.ts` (S28-004).
 */

import { invoke } from '@tauri-apps/api/core';

const TELEMETRY_CONSENT_KEY = 'telemetry_consent';

export type TelemetryConsentValue = 'granted' | 'denied' | null;

export function useTelemetryConsent() {
  /** Load telemetry consent from the OS credential store. */
  async function getTelemetryConsent(): Promise<TelemetryConsentValue> {
    const tryFetch = async (): Promise<TelemetryConsentValue> => {
      try {
        const result = await invoke<string | null>('get_secret', { key: TELEMETRY_CONSENT_KEY });
        if (!result) return null;
        // Handle both plain values ("granted"/"denied") and legacy JSON
        // payloads ({"value":"granted","date":"...","policyVersion":2}).
        if (result === 'granted') return 'granted';
        if (result === 'denied') return 'denied';
        try {
          const parsed = JSON.parse(result);
          if (parsed?.value === 'granted') return 'granted';
          if (parsed?.value === 'denied') return 'denied';
        } catch {
          // Not JSON — treat as unknown
        }
        return null;
      } catch {
        return null;
      }
    };

    // First attempt
    let result = await tryFetch();
    // If null (could be race condition on F5), retry after 500ms
    if (result === null) {
      await new Promise((resolve) => setTimeout(resolve, 500));
      result = await tryFetch();
    }
    return result;
  }

  /** Store telemetry consent in the OS credential store. */
  async function setTelemetryConsent(value: 'granted' | 'denied'): Promise<void> {
    try {
      await invoke('set_secret', { key: TELEMETRY_CONSENT_KEY, value });
      // S49: notify live listeners (App-level Sentry init reacts to this and
      // enables/disables crash reporting without an app restart).
      window.dispatchEvent(new CustomEvent('micontrol:consent-changed'));
    } catch (err) {
      console.error('[settings] Failed to store telemetry consent:', err);
    }
  }

  /** Revoke telemetry consent — deletes the consent secret and disables AI features. */
  async function revokeTelemetryConsent(): Promise<void> {
    try {
      await invoke('delete_secret', { key: TELEMETRY_CONSENT_KEY });
      window.dispatchEvent(new CustomEvent('micontrol:consent-changed'));
    } catch {
      // Ignore — key may not exist yet
    }
  }

  return {
    getTelemetryConsent,
    setTelemetryConsent,
    revokeTelemetryConsent,
    checkTelemetryConsent: () => getTelemetryConsent().then((c) => c === 'granted'),
  };
}
