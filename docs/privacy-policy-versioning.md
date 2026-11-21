# Privacy Policy Versioning

## How It Works

The backend tracks a `POLICY_VERSION` constant in `src-tauri/src/util/consent_audit.rs` (currently `2`). Every consent event written to the audit log records the policy version that was in force at the time, so a later policy change can be correlated with the consent records it invalidates.

When the privacy policy text changes:

1. Increment `POLICY_VERSION` in `src-tauri/src/util/consent_audit.rs`.
2. Update the privacy policy text in `src/components/ConsentDialog.tsx` and `src/pages/PrivacyPolicy.tsx`.
3. Add translations for the updated text in `src/i18n/{en,pt,es,fr}.json`.
4. Run `pnpm run i18n:check` to confirm no key is missing.

## Audit Log

Consent events are appended to `%LOCALAPPDATA%\MiControl\consent_audit.log` as tab-separated lines:

```text
<unix-ts>	CONSENT_GRANTED	policy_version=2	hmac=<hex>
```

- **Timestamp** — Unix seconds.
- **Event type** — `CONSENT_GRANTED` or `CONSENT_REVOKED`.
- **Policy version** — the value of `POLICY_VERSION` when the event was written.
- **HMAC** — integrity tag over the line, keyed by a sub-key derived from the master key.

The signing sub-key lives next to the log as `consent_audit.key`; a rotation preserves the previous key as `consent_audit.key.old` so older lines still verify. Both files are ACL-restricted to the current user.

## Frontend Integration

- Consent itself is stored in the **OS credential store** under the key `telemetry_consent`, with the plain values `"granted"` / `"denied"`. Legacy JSON payloads (`{"value":"granted","date":"…","policyVersion":2}`) are still accepted on read for backwards compatibility, but new writes are plain values.
- `useTelemetryConsent()` (`src/hooks/useTelemetryConsent.ts`) exposes `getTelemetryConsent()`, `setTelemetryConsent()`, `revokeTelemetryConsent()` and `checkTelemetryConsent()`.
- `MainWindow.tsx` reads the consent on mount and shows `ConsentDialog` when the value is `null` (first launch, or after a revoke).
- `setTelemetryConsent()` dispatches a `micontrol:consent-changed` window event; `App.tsx` listens for it and enables/disables Sentry without an app restart.

> **Known gap (2026-11-11):** the frontend does **not** compare a stored policy version against `POLICY_VERSION` — there is no `getStoredPolicyVersion()` and `MainWindow` only checks for a missing consent. Bumping `POLICY_VERSION` therefore changes what the audit log records, but does **not** by itself re-prompt existing users. Implementing the re-consent flow requires storing the version alongside the consent and comparing it on mount.

## Adding a New Policy Version

1. Increment `POLICY_VERSION` in `src-tauri/src/util/consent_audit.rs`.
2. Update the consent dialog text in `src/components/ConsentDialog.tsx` and the policy page in `src/pages/PrivacyPolicy.tsx`.
3. Update translations in `src/i18n/{en,pt,es,fr}.json` and run `pnpm run i18n:check`.
4. If the re-consent flow is implemented, verify that an existing user is prompted once and that the new `policy_version` appears in `consent_audit.log`.
