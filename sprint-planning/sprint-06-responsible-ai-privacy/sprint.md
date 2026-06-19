# Sprint 6 — Responsible AI & Privacy

## Sprint Metadata

| Field | Value |
|-------|-------|
| **Sprint Name** | Responsible AI & Privacy |
| **Sprint Goal** | Fix accessibility violations, secure the OpenAI API key, and add a consent dialog and privacy policy for hardware telemetry |
| **Duration Estimate** | 2 weeks (10 working days) |
| **Priority** | P1 — Privacy and accessibility compliance. |
| **Sprint Type** | Feature / Compliance |
| **Primary Owner** | Frontend engineer (accessibility + consent UI) |
| **Secondary Owner** | Privacy/legal reviewer |

## Sprint Goal Statement

The app currently has keyboard-inaccessible toggle controls, stores the OpenAI API key in plaintext in `localStorage`, and sends hardware telemetry to OpenAI with no consent dialog or privacy policy. By the end of this sprint, all toggle rows are keyboard-operable, the API key is stored securely (not in `localStorage`), and a first-run consent dialog with a linked privacy policy governs all telemetry sent to OpenAI.

---

## Background

Three high-severity Responsible AI findings: (R1) toggle rows use `<div onClick>` with no role/tabIndex/onKeyDown, (R2) OpenAI API key stored as plaintext in `localStorage`, (R3) hardware telemetry sent to OpenAI with no consent dialog or privacy policy. These violate WCAG accessibility standards, expose credentials to XSS, and bypass user consent for data sharing.

---

## Tickets

### S6-001 — Make toggle rows keyboard-accessible

| Field | Value |
|-------|-------|
| **Ticket ID** | S6-001 |
| **Title** | Replace `<div onClick>` toggle rows with keyboard-operable controls (role, tabIndex, onKeyDown) |
| **Priority** | P1 |
| **Type** | Bug / Accessibility |
| **Estimated Effort** | M |

#### Description

In `src/components/TouchpadSettings.tsx` and `src/components/DisplaySettings.tsx`, toggle rows are implemented as `<div onClick={...}>` with no `role`, `tabIndex`, or `onKeyDown` handler. This means keyboard users cannot focus or activate the toggles, violating WCAG 2.1 Level A (Success Criterion 2.1.1 Keyboard) and Level AA (4.1.2 Name, Role, Value).

#### Affected Files and Line Ranges

- `src/components/TouchpadSettings.tsx` — toggle row components.
- `src/components/DisplaySettings.tsx` — toggle row components.
- Any other settings component using the same pattern (audit).

#### Root Cause Analysis

Using `<div onClick>` is a common shortcut that works for mouse users but is completely inaccessible to keyboard and screen-reader users. A `<div>` has no semantic role, is not focusable by default, and does not respond to Enter/Space. The correct approach is either a native `<button>` or a `<div>` with `role="switch"`, `tabIndex={0}`, and an `onKeyDown` handler that toggles on Enter/Space.

#### Acceptance Criteria

- [ ] All toggle rows use either a native `<button>` (preferred) or a `<div role="switch" tabIndex={0} aria-checked={...} onKeyDown={...}>`.
- [ ] The `onKeyDown` handler toggles the state on `Enter` and `Space` (preventing default scroll on Space).
- [ ] Each toggle has an accessible name (`aria-label` or visible label associated via `htmlFor`/`id`).
- [ ] Focus styles are visible (focus ring) — do not remove `:focus-visible` outlines.
- [ ] Keyboard-only test: tab through all settings; every toggle is focusable and operable via Enter/Space.
- [ ] Screen reader test (NVDA or VoiceOver): the toggle announces as a switch with its current state.
- [ ] axe-core or Lighthouse accessibility audit passes with no "keyboard" or "aria" violations on these components.
- [ ] Unit test: render a toggle, simulate `keydown` Enter; assert state toggled.

#### Implementation Notes

- Prefer extracting a reusable `<ToggleRow>` component to ensure consistency and prevent regression.
- If using a `<div role="switch">`, also add `aria-checked={isChecked}` for screen readers.
- Ensure the focus order is logical (DOM order matches visual order).

#### Testing Strategy

- **Keyboard test**: tab + Enter/Space through all toggles.
- **Screen reader test**: NVDA/VoiceOver announcement.
- **Automated audit**: axe-core browser extension or Lighthouse.
- **Unit test**: keydown simulation.

#### Dependencies

- None.

---

### S6-002 — Secure the OpenAI API key storage

| Field | Value |
|-------|-------|
| **Ticket ID** | S6-002 |
| **Title** | Move OpenAI API key out of `localStorage` into a secure backend store |
| **Priority** | P1 |
| **Type** | Security / Privacy |
| **Estimated Effort** | M |

#### Description

In `src/hooks/useSettings.ts` (~line 121), the OpenAI API key is stored in `localStorage` as plaintext. `localStorage` is accessible to any JavaScript running in the page, including XSS payloads, and persists indefinitely. This ticket moves the key to a secure backend store (e.g. Windows Credential Manager via Tauri, or an encrypted config file with restrictive ACL).

#### Affected Files and Line Ranges

- `src/hooks/useSettings.ts` — API key storage (~line 121).
- A new Tauri command for credential get/set (e.g. `src-tauri/src/commands/credentials.rs`).
- `src-tauri/Cargo.toml` — add a credential store crate (e.g. `keyring`).

#### Root Cause Analysis

`localStorage` is the easiest frontend persistence mechanism, but it provides no isolation: any script in the page (including injected XSS) can read `localStorage.getItem('openai_api_key')`. For a desktop app, the backend has access to OS-level credential stores (Windows Credential Manager, macOS Keychain) that encrypt and isolate secrets per-application.

#### Acceptance Criteria

- [ ] The API key is no longer written to or read from `localStorage`.
- [ ] A Tauri command `set_secret(key: &str, value: &str)` and `get_secret(key: &str) -> Option<String>` are added, backed by the OS credential store (Windows Credential Manager via the `keyring` crate).
- [ ] `useSettings` calls `invoke('set_secret', ...)` / `invoke('get_secret', ...)` instead of `localStorage`.
- [ ] The key is never logged or included in telemetry.
- [ ] A migration path handles existing keys in `localStorage`: on first run after update, read from `localStorage`, store via `set_secret`, then delete from `localStorage`.
- [ ] Manual test: set the API key, restart the app, confirm it persists; inspect `localStorage` and confirm the key is absent.
- [ ] Security test: confirm the key is stored in Credential Manager (via `cmdkey` or the Windows UI) and not in any plaintext file.

#### Implementation Notes

- Use the `keyring` crate (cross-platform, wraps Windows Credential Manager / macOS Keychain / Linux Secret Service).
- The credential "service name" should be the app identifier (e.g. `com.mipc.micontrol`); the "username" field can be a fixed string like `openai_api_key`.
- The migration step should be idempotent and logged.
- Ensure the key is transmitted to the backend only over Tauri's IPC (not exposed to the webview's global scope beyond the brief invoke).

#### Testing Strategy

- **Manual test**: set/restart/verify persistence; `localStorage` inspection.
- **Security audit**: confirm no plaintext key in files or `localStorage`.
- **Migration test**: pre-populate `localStorage` with a key, run the app, confirm migration to credential store and `localStorage` cleanup.

#### Dependencies

- None.

---

### S6-003 — Add telemetry consent dialog and privacy policy

| Field | Value |
|-------|-------|
| **Ticket ID** | S6-003 |
| **Title** | Require explicit user consent before sending hardware telemetry to OpenAI; publish a privacy policy |
| **Priority** | P1 |
| **Type** | Feature / Compliance |
| **Estimated Effort** | L |

#### Description

Hardware telemetry (device state, usage patterns) is sent to OpenAI (via the AI features) with no consent dialog and no published privacy policy. Users are not informed what data is sent, to whom, or for what purpose. This ticket adds a first-run consent dialog and a privacy policy document, and gates all telemetry behind the consent.

#### Affected Files and Line Ranges

- `src/hooks/useSettings.ts` — telemetry consent state.
- A new `src/components/ConsentDialog.tsx`.
- A new `src/pages/PrivacyPolicy.tsx` (or a markdown doc rendered in-app).
- The AI feature invocation site (wherever telemetry is sent to OpenAI).

#### Root Cause Analysis

The AI features were added without a consent flow, likely assuming the user opted in by configuring the API key. But configuring a key is not informed consent for data sharing. Privacy regulations (GDPR, CCPA) and responsible AI principles require explicit, informed consent with the ability to opt out.

#### Acceptance Criteria

- [ ] A first-run consent dialog is shown when the user first accesses an AI feature (or on first app launch if AI is prominent).
- [ ] The dialog clearly states: what data is collected (hardware telemetry, device state), where it's sent (OpenAI), for what purpose (AI-powered features), and that it can be revoked at any time.
- [ ] The dialog has "Allow" and "Deny" options; "Deny" disables AI features gracefully (with a message explaining why).
- [ ] Consent state is stored via the secure backend store (S6-002's credential store or a separate settings file with restrictive ACL), not `localStorage`.
- [ ] A privacy policy document is accessible in-app (Settings → Privacy) and linked from the consent dialog.
- [ ] All telemetry-sending code paths check consent before sending; if consent is denied or revoked, no data is sent.
- [ ] A "Revoke consent" option exists in Settings, immediately stopping telemetry.
- [ ] Manual test: deny consent, use an AI feature, confirm no telemetry is sent (verify via network inspection).
- [ ] Manual test: allow consent, revoke later, confirm telemetry stops.

#### Implementation Notes

- The consent dialog should not be dismissable without a choice (no "remind me later" that allows telemetry in the meantime) — but it can be deferred until the user actually uses an AI feature.
- The privacy policy should be written in plain language and cover: data types, recipients, purpose, retention, user rights, contact.
- Store consent with a timestamp and the policy version, so policy updates can re-prompt.
- Network inspection: use Tauri's devtools or a proxy to confirm no OpenAI requests when consent is denied.

#### Testing Strategy

- **Manual test**: consent flow (allow/deny/revoke) with network inspection.
- **Unit test**: the consent gate function returns false when consent is absent/denied.
- **Documentation review**: privacy policy reviewed by a privacy-aware reviewer.

#### Dependencies

- S6-002 (secure storage for consent state).

---

## Sprint Exit Criteria

- [ ] All 3 tickets merged.
- [ ] `npm run build` and `cargo check` pass.
- [ ] axe-core/Lighthouse accessibility audit passes on settings pages.
- [ ] API key absent from `localStorage` after migration.
- [ ] No OpenAI network requests when consent is denied (verified via network inspection).
- [ ] Privacy policy published and linked in-app.

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Credential store unavailable on some Windows configs | Fall back to an encrypted file with restrictive ACL; document the fallback. |
| Consent dialog annoys power users | Defer until first AI feature use; allow "always allow" for the session. |
| Migration misses keys set under different storage keys | Audit all `localStorage` keys containing "key"/"secret"/"openai". |
| Accessibility fix breaks existing visual design | Use focus-visible styles; preserve visual layout. |
