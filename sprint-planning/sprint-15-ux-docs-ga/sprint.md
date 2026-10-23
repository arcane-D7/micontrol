# Sprint 15 — UX, Documentation & GA Readiness (P2)

## Sprint Metadata

| Field                 | Value                                                                                                                                                                                                                    |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Sprint Name**       | UX, Documentation & GA Readiness                                                                                                                                                                                         |
| **Sprint Goal**       | Prepare the application for General Availability — rewrite README, create CHANGELOG, bump to v1.0.0, implement onboarding wizard, add Rust doc comments, fix all remaining UX/RAI findings, and add Authenticode signing |
| **Duration Estimate** | ~10 days                                                                                                                                                                                                                 |
| **Priority**          | P2 — Final sprint before GA                                                                                                                                                                                              |
| **Sprint Type**       | Multi-domain (UX, Documentation, Product, RAI, DevOps)                                                                                                                                                                   |
| **Primary Owner**     | Full-stack engineer                                                                                                                                                                                                      |
| **Secondary Owner**   | Tech writer + UX designer                                                                                                                                                                                                |
| **Source**            | `docs/stability-report-2026-06-24-post-sprints.md`                                                                                                                                                                       |
| **Depends On**        | Sprint 13, Sprint 14                                                                                                                                                                                                     |

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **OBRIGATÓRIO: 100% dos tickets desta sprint devem ser concluídos. A sprint não será aceita como entregue se qualquer ticket permanecer incompleto.**
>
> **MANDATORY: 100% of the tickets in this sprint MUST be completed. The sprint will NOT be accepted as delivered if any ticket remains incomplete.**

Every ticket must pass its acceptance criteria AND the full health check suite (9/9) before the sprint commit is made. This is the final sprint before GA — no incomplete tickets will be tolerated. The version bump to 1.0.0 must only happen after ALL other tickets are complete.

---

## Sprint Goal Statement

The post-audit stability report identified 4 CRITICAL documentation findings, 3 CRITICAL product findings, and numerous UX/RAI MEDIUM findings. The README is in Portuguese and reads like a changelog, there is no CHANGELOG.md, the version is still 0.1.0, and there is no onboarding experience. This sprint addresses all remaining findings and prepares the application for General Availability at v1.0.0.

---

## Health Check Commands (must pass 9/9 before commit)

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
npx tsc --noEmit
npm run lint
npm run format:check
npm run build
npm run version:check
```

---

## Tickets

### S15-001 — Rewrite README as proper English product README

| Field                | Value                                                  |
| -------------------- | ------------------------------------------------------ |
| **Ticket ID**        | S15-001                                                |
| **Title**            | Rewrite `README.md` as a proper English product README |
| **Priority**         | P2                                                     |
| **Type**             | Documentation / Product                                |
| **Estimated Effort** | M                                                      |
| **Source Finding**   | DOC-004 / DOC-005 / PROD-001 / PROD-008 (CRITICAL)     |

#### Context

The current README is in Portuguese, reads like a running changelog/roadmap, and has no competitive positioning. It does not serve as a product README for a GA release.

#### Acceptance Criteria

- [x] README is written entirely in English
- [x] README follows standard structure: title, badges, description, features, screenshots, installation, usage, configuration, troubleshooting, contributing, license
- [x] Competitive positioning section explains how miPC differs from alternatives
- [x] No changelog/roadmap content in README (moved to CHANGELOG.md)
- [x] Screenshots or GIFs included (or placeholder with TODO if not yet available)
- [x] Installation instructions cover both installer and build-from-source
- [x] Links to documentation and support channels

---

### S15-002 — Create CHANGELOG.md

| Field                | Value                                                   |
| -------------------- | ------------------------------------------------------- |
| **Ticket ID**        | S15-002                                                 |
| **Title**            | Create `CHANGELOG.md` following Keep a Changelog format |
| **Priority**         | P2                                                      |
| **Type**             | Documentation                                           |
| **Estimated Effort** | S                                                       |
| **Source Finding**   | DOC-001 (CRITICAL)                                      |

#### Context

No CHANGELOG.md exists. Release history is only visible through git log.

#### Acceptance Criteria

- [x] `CHANGELOG.md` created at project root
- [x] Follows [Keep a Changelog](https://keepachangelog.com/) format
- [x] Includes all versions from 0.1.0 through 1.0.0
- [x] Sections: Added, Changed, Deprecated, Removed, Fixed, Security
- [x] Sprints 1–15 are summarized in the appropriate version sections
- [x] Unreleased section is present for future changes

---

### S15-003 — Bump version to 1.0.0

| Field                | Value                                         |
| -------------------- | --------------------------------------------- |
| **Ticket ID**        | S15-003                                       |
| **Title**            | Bump version to 1.0.0 across all config files |
| **Priority**         | P2                                            |
| **Type**             | Product                                       |
| **Estimated Effort** | XS                                            |
| **Source Finding**   | PROD-002 (CRITICAL)                           |

#### Context

Version is still 0.1.0 in `package.json`, `Cargo.toml`, and `tauri.conf.json`. This is not a GA version.

#### Acceptance Criteria

- [x] `package.json` version is `1.0.0`
- [x] `Cargo.toml` version is `1.0.0`
- [x] `tauri.conf.json` version is `1.0.0`
- [x] `npm run version:check` passes (all consistent)
- [x] Version is referenced in README and CHANGELOG
- [x] This ticket is done LAST (after all other sprint tickets are complete)

---

### S15-004 — Implement first-run onboarding wizard

| Field                | Value                                 |
| -------------------- | ------------------------------------- |
| **Ticket ID**        | S15-004                               |
| **Title**            | Implement first-run onboarding wizard |
| **Priority**         | P2                                    |
| **Type**             | Product / UX                          |
| **Estimated Effort** | L                                     |
| **Source Finding**   | PROD-004 (HIGH)                       |

#### Context

There is no onboarding or first-run experience. Users are dropped into the main UI without guidance.

#### Acceptance Criteria

- [x] Onboarding wizard appears on first launch (detected via settings flag)
- [x] Wizard includes: welcome screen, feature overview, privacy consent, optional AI setup
- [x] Wizard can be skipped (settings flag set to completed)
- [x] Wizard is accessible from Settings ("Replay onboarding")
- [x] All wizard strings are translatable (i18n keys in all 4 locales)
- [x] `npx tsc --noEmit` passes
- [x] `npm run lint` passes
- [x] `npm run build` passes

---

### S15-005 — Add Rust doc comments to all modules

| Field                | Value                                                   |
| -------------------- | ------------------------------------------------------- |
| **Ticket ID**        | S15-005                                                 |
| **Title**            | Add `//!` module-level doc comments to all Rust modules |
| **Priority**         | P2                                                      |
| **Type**             | Documentation                                           |
| **Estimated Effort** | M                                                       |
| **Source Finding**   | DOC-007 / DOC-008 / DOC-009 (HIGH)                      |

#### Context

Module declarations in `lib.rs` lack doc comments. Module root files (`hw/mod.rs`, `commands/mod.rs`, `util/mod.rs`) and several HW modules lack proper `//!` doc comments.

#### Acceptance Criteria

- [x] Every module in `lib.rs` has a `///` doc comment on the `mod` declaration
- [x] Every module root file (`mod.rs` or equivalent) has a `//!` module-level doc comment
- [x] Every `hw/*.rs` module has a `//!` doc comment explaining its purpose
- [x] Every `commands/*.rs` module has a `//!` doc comment
- [x] Every `util/*.rs` module has a `//!` doc comment
- [x] `cargo doc --manifest-path src-tauri/Cargo.toml` builds without warnings
- [x] `cargo clippy -- -D warnings` passes

---

### S15-006 — Create frontend architecture documentation

| Field                | Value                                      |
| -------------------- | ------------------------------------------ |
| **Ticket ID**        | S15-006                                    |
| **Title**            | Create frontend architecture documentation |
| **Priority**         | P2                                         |
| **Type**             | Documentation                              |
| **Estimated Effort** | M                                          |
| **Source Finding**   | DOC-011 (HIGH)                             |

#### Context

There is no frontend architecture documentation. New contributors have no guide to the React/TypeScript codebase structure.

#### Acceptance Criteria

- [x] `docs/frontend-architecture.md` created
- [x] Documents: component hierarchy, state management, routing, i18n, IPC with Tauri
- [x] Includes a diagram (Mermaid or similar) of component structure
- [x] Documents the hook pattern (`useSettings`, `useAnalysisLogger`, etc.)
- [x] References CONTRIBUTING.md for contribution workflow

---

### S15-007 — Fix hardcoded English strings in tab pages

| Field                | Value                                                                                      |
| -------------------- | ------------------------------------------------------------------------------------------ |
| **Ticket ID**        | S15-007                                                                                    |
| **Title**            | Replace hardcoded English strings in `tabs/audio.tsx` and `tabs/wifi.tsx` with `t()` calls |
| **Priority**         | P2                                                                                         |
| **Type**             | UX                                                                                         |
| **Estimated Effort** | XS                                                                                         |
| **Source Finding**   | UX-002 (HIGH)                                                                              |

#### Context

`tabs/audio.tsx` and `tabs/wifi.tsx` have hardcoded English strings that bypass the i18n system.

#### Acceptance Criteria

- [x] All hardcoded strings in `tabs/audio.tsx` replaced with `t()` calls
- [x] All hardcoded strings in `tabs/wifi.tsx` replaced with `t()` calls
- [x] New i18n keys added to `en.json` and translated in `pt.json`, `es.json`, `fr.json`
- [x] i18n checker passes with 0 missing keys
- [x] `npx tsc --noEmit` passes
- [x] `npm run lint` passes

---

### S15-008 — Fix consent dialog button styling

| Field                | Value                                                                       |
| -------------------- | --------------------------------------------------------------------------- |
| **Ticket ID**        | S15-008                                                                     |
| **Title**            | Fix consent dialog button styling (Deny = ghost/secondary, Allow = primary) |
| **Priority**         | P2                                                                          |
| **Type**             | UX                                                                          |
| **Estimated Effort** | XS                                                                          |
| **Source Finding**   | UX-003 (HIGH)                                                               |

#### Context

`ConsentDialog.tsx` uses `btn-primary` for both Allow and Deny buttons, making them visually equal. Deny should be a ghost/secondary style.

#### Acceptance Criteria

- [x] Allow button uses `btn-primary` class
- [x] Deny button uses `btn-ghost` or `btn-secondary` class
- [x] Visual hierarchy clearly indicates Allow is the primary action
- [x] `npm run build` passes
- [x] No visual regressions

---

### S15-009 — Fix watermark pointer events

| Field                | Value                                                        |
| -------------------- | ------------------------------------------------------------ |
| **Ticket ID**        | S15-009                                                      |
| **Title**            | Fix watermark blocking pointer events for underlying content |
| **Priority**         | P2                                                           |
| **Type**             | UX                                                           |
| **Estimated Effort** | XS                                                           |
| **Source Finding**   | UX-001 (CRITICAL)                                            |

#### Context

The watermark in `MainWindow.tsx:320-340` blocks pointer events for underlying content. Users cannot click through the watermark area.

#### Acceptance Criteria

- [x] Watermark has `pointer-events: none` CSS property
- [x] Underlying content is fully clickable through the watermark area
- [x] Watermark remains visually visible
- [x] `npm run build` passes
- [x] No visual regressions

---

### S15-010 — Add accessible labels to WiFi password input

| Field                | Value                                                |
| -------------------- | ---------------------------------------------------- |
| **Ticket ID**        | S15-010                                              |
| **Title**            | Add accessible label and i18n to WiFi password input |
| **Priority**         | P2                                                   |
| **Type**             | UX                                                   |
| **Estimated Effort** | XS                                                   |
| **Source Finding**   | UX-005 (HIGH)                                        |

#### Context

`WiFiManager.tsx:129-138` — WiFi password input lacks an accessible label and uses hardcoded English.

#### Acceptance Criteria

- [x] WiFi password input has `aria-label` or associated `<label>` element
- [x] Label text is translatable (i18n key in all 4 locales)
- [x] `npx tsc --noEmit` passes
- [x] `npm run lint` passes

---

### S15-011 — Add `role="status"` to skeleton loaders

| Field                | Value                                                            |
| -------------------- | ---------------------------------------------------------------- |
| **Ticket ID**        | S15-011                                                          |
| **Title**            | Add `role="status"` and `aria-live="polite"` to skeleton loaders |
| **Priority**         | P2                                                               |
| **Type**             | UX                                                               |
| **Estimated Effort** | XS                                                               |
| **Source Finding**   | UX-006 (MEDIUM)                                                  |

#### Context

Skeleton loaders lack accessible status announcements. Screen reader users are not informed when content is loading.

#### Acceptance Criteria

- [x] All skeleton loader components have `role="status"` and `aria-live="polite"`
- [x] Loading text is translatable (e.g., "Loading...")
- [x] `npx tsc --noEmit` passes
- [x] `npm run lint` passes

---

### S15-012 — Expand `prefers-reduced-motion` to all animations

| Field                | Value                                                               |
| -------------------- | ------------------------------------------------------------------- |
| **Ticket ID**        | S15-012                                                             |
| **Title**            | Expand `prefers-reduced-motion` media query to cover all animations |
| **Priority**         | P2                                                                  |
| **Type**             | RAI                                                                 |
| **Estimated Effort** | XS                                                                  |
| **Source Finding**   | RAI-007 (MEDIUM)                                                    |

#### Context

`globals.css` `prefers-reduced-motion` only covers two hover effects. All other animations (transitions, transforms, keyframes) are not covered.

#### Acceptance Criteria

- [x] `prefers-reduced-motion: reduce` disables all CSS transitions and animations
- [x] All `transition` and `animation` properties are wrapped in `@media (prefers-reduced-motion: no-preference)` or overridden in the reduce query
- [x] No layout shift when reduced motion is active
- [x] `npm run build` passes

---

### S15-013 — Make ErrorBoundary strings translatable

| Field                | Value                                                                 |
| -------------------- | --------------------------------------------------------------------- |
| **Ticket ID**        | S15-013                                                               |
| **Title**            | Make ErrorBoundary.tsx hardcoded English strings translatable         |
| **Priority**         | P2                                                                    |
| **Type**             | RAI                                                                   |
| **Estimated Effort** | XS                                                                    |
| **Source Finding**   | RAI-005 (MEDIUM)                                                      |
| **Regression of**    | S9-010 (incomplete — ErrorBoundary created but has hardcoded strings) |

#### Context

`ErrorBoundary.tsx` has hardcoded English strings for error messages and reload button.

#### Acceptance Criteria

- [x] All hardcoded strings in `ErrorBoundary.tsx` replaced with `t()` calls
- [x] New i18n keys added to `en.json` and translated in `pt.json`, `es.json`, `fr.json`
- [x] i18n checker passes with 0 missing keys
- [x] `npx tsc --noEmit` passes
- [x] `npm run lint` passes

---

### S15-014 — Disclose Sentry in consent flow

| Field                | Value                                                 |
| -------------------- | ----------------------------------------------------- |
| **Ticket ID**        | S15-014                                               |
| **Title**            | Disclose Sentry crash reporting in the consent dialog |
| **Priority**         | P2                                                    |
| **Type**             | RAI                                                   |
| **Estimated Effort** | S                                                     |
| **Source Finding**   | RAI-008 (MEDIUM) + PROD-006 (HIGH)                    |

#### Context

Sentry is initialized in the backend (`lib.rs`) unconditionally, but the consent dialog does not mention crash reporting. Users are not informed that crash data is sent to Sentry.

#### Acceptance Criteria

- [x] Consent dialog includes text about crash reporting (Sentry)
- [x] Consent text is translatable in all 4 locales
- [x] If user denies consent, Sentry is not initialized (or is disabled at runtime)
- [x] Privacy policy section mentions Sentry and what data is collected
- [x] `npx tsc --noEmit` passes

---

### S15-015 — Add inclusive language and disclaimer to AI prompt

| Field                | Value                                                           |
| -------------------- | --------------------------------------------------------------- |
| **Ticket ID**        | S15-015                                                         |
| **Title**            | Add inclusive language check and AI disclaimer to system prompt |
| **Priority**         | P2                                                              |
| **Type**             | RAI                                                             |
| **Estimated Effort** | XS                                                              |
| **Source Finding**   | RAI-009 (MEDIUM)                                                |

#### Context

The AI analysis system prompt lacks an inclusive language check and does not include a disclaimer that AI output may be inaccurate.

#### Acceptance Criteria

- [x] System prompt includes instruction to use inclusive language
- [x] System prompt includes disclaimer that AI output may be inaccurate
- [x] AI analysis results display a disclaimer in the UI
- [x] Disclaimer is translatable in all 4 locales
- [x] `npx tsc --noEmit` passes

---

### S15-016 — Add keyboard shortcuts for top tabs

| Field                | Value                                                 |
| -------------------- | ----------------------------------------------------- |
| **Ticket ID**        | S15-016                                               |
| **Title**            | Add keyboard shortcuts for switching between top tabs |
| **Priority**         | P2                                                    |
| **Type**             | UX                                                    |
| **Estimated Effort** | S                                                     |
| **Source Finding**   | UX-014 (MEDIUM)                                       |

#### Context

There are no keyboard shortcuts for navigating between tabs. Power users cannot switch tabs without mouse.

#### Acceptance Criteria

- [x] Keyboard shortcuts (e.g., `Alt+1` through `Alt+9`) switch between tabs
- [x] Shortcuts are documented in the UI (tooltip or help overlay)
- [x] Shortcut hints are translatable
- [x] `npx tsc --noEmit` passes
- [x] `npm run lint` passes

---

### S15-017 — Add error reporting channel in ErrorBoundary

| Field                | Value                                                    |
| -------------------- | -------------------------------------------------------- |
| **Ticket ID**        | S15-017                                                  |
| **Title**            | Add user-facing error reporting channel in ErrorBoundary |
| **Priority**         | P2                                                       |
| **Type**             | Product                                                  |
| **Estimated Effort** | S                                                        |
| **Source Finding**   | PROD-009 (HIGH)                                          |

#### Context

`ErrorBoundary.tsx` shows an error message but provides no way for users to report the error or get help.

#### Acceptance Criteria

- [x] ErrorBoundary includes a "Report Issue" link or button
- [x] Link opens GitHub Issues page or email client with pre-filled error details
- [x] Error details (stack trace, app version, OS) are included in the report
- [x] "Reload" button is also present
- [x] All strings are translatable
- [x] `npx tsc --noEmit` passes

---

### S15-018 — Add AI cost estimation and usage tracking

| Field                | Value                                     |
| -------------------- | ----------------------------------------- |
| **Ticket ID**        | S15-018                                   |
| **Title**            | Add AI cost estimation and usage tracking |
| **Priority**         | P2                                        |
| **Type**             | Product                                   |
| **Estimated Effort** | M                                         |
| **Source Finding**   | PROD-005 (HIGH)                           |

#### Context

AI feature costs are opaque. There is no usage tracking or cost estimation, making it impossible for users to understand their AI spending.

#### Acceptance Criteria

- [x] AI usage is tracked (number of requests, tokens used)
- [x] Cost estimation is displayed in the UI (based on model pricing)
- [x] Usage data is persisted locally (not sent to any server)
- [x] Usage can be reset by the user
- [x] All strings are translatable
- [x] `npx tsc --noEmit` passes
- [x] `npm run lint` passes

---

### S15-019 — Add CODEOWNERS file

| Field                | Value                                         |
| -------------------- | --------------------------------------------- |
| **Ticket ID**        | S15-019                                       |
| **Title**            | Add `CODEOWNERS` file for code review routing |
| **Priority**         | P2                                            |
| **Type**             | DevOps                                        |
| **Estimated Effort** | XS                                            |
| **Source Finding**   | DEV-014 (LOW)                                 |

#### Context

No CODEOWNERS file exists. PR reviews are not automatically routed to the appropriate maintainers.

#### Acceptance Criteria

- [x] `.github/CODEOWNERS` file created
- [x] Owners are assigned for: `src-tauri/`, `src/`, `.github/`, `docs/`
- [x] File follows GitHub CODEOWNERS format
- [x] Referenced in CONTRIBUTING.md

---

### S15-020 — Add `tsc --noEmit` and `version:check` to pre-commit hooks

| Field                | Value                                                               |
| -------------------- | ------------------------------------------------------------------- |
| **Ticket ID**        | S15-020                                                             |
| **Title**            | Add TypeScript checking and version consistency to pre-commit hooks |
| **Priority**         | P2                                                                  |
| **Type**             | DevOps                                                              |
| **Estimated Effort** | XS                                                                  |
| **Source Finding**   | DEV-007 / DEV-008 (HIGH)                                            |

#### Context

Pre-commit hooks (husky + lint-staged) do not include TypeScript type checking or version consistency validation.

#### Acceptance Criteria

- [x] `npx tsc --noEmit` is added to `.husky/pre-commit`
- [x] `npm run version:check` is added to `.husky/pre-commit`
- [x] Pre-commit hook fails if either check fails
- [x] Hook is fast enough to not significantly slow down commits (or runs in parallel)

---

### S15-021 — Fix tray opacity (move from documentElement to wrapper)

| Field                | Value                                     |
| -------------------- | ----------------------------------------- |
| **Ticket ID**        | S15-021                                   |
| **Title**            | Fix tray opacity to not affect all layers |
| **Priority**         | P2                                        |
| **Type**             | UX                                        |
| **Estimated Effort** | XS                                        |
| **Source Finding**   | UX-004 (HIGH)                             |

#### Context

`TrayPopup.tsx:64-66` sets opacity on `document.documentElement`, which affects all layers including the tray popup itself.

#### Acceptance Criteria

- [x] Opacity is moved from `document.documentElement` to a wrapper element
- [x] Only the intended background is affected by opacity
- [x] Tray popup and other UI elements are not affected
- [x] `npm run build` passes
- [x] No visual regressions

---

### S15-022 — Fix `latest.json` generation in release workflow

| Field                | Value                                                                |
| -------------------- | -------------------------------------------------------------------- |
| **Ticket ID**        | S15-022                                                              |
| **Title**            | Fix `latest.json` generation and artifact upload in release workflow |
| **Priority**         | P2                                                                   |
| **Type**             | DevOps                                                               |
| **Estimated Effort** | XS                                                                   |
| **Source Finding**   | DEV-011 / DEV-012 (MEDIUM)                                           |

#### Context

Release workflow searches for `.msi` artifacts but `bundle.targets` is NSIS only. The `latest.json` upload includes non-existent artifact types (msi/msix/appx).

#### Acceptance Criteria

- [x] Release workflow only searches for `.nsis` (and `.exe` if applicable) artifacts
- [x] `latest.json` only references artifact types that actually exist
- [x] No errors in release workflow about missing files
- [x] `latest.json` is correctly generated and uploaded

---

## Sprint Completion Checklist

Before committing the sprint, verify ALL of the following:

- [x] All 22 tickets have their acceptance criteria fully met
- [x] `cargo fmt --manifest-path src-tauri/Cargo.toml --check` passes
- [x] `cargo check --manifest-path src-tauri/Cargo.toml` passes
- [x] `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` passes (0 warnings)
- [x] `cargo test --manifest-path src-tauri/Cargo.toml` passes (all tests green)
- [x] `cargo doc --manifest-path src-tauri/Cargo.toml` builds without warnings
- [x] `npx tsc --noEmit` passes (0 errors)
- [x] `npm run lint` passes (0 warnings)
- [x] `npm run format:check` passes
- [x] `npm run build` passes
- [x] `npm run version:check` passes (all at 1.0.0)
- [x] README.md is in English and follows product README structure
- [x] CHANGELOG.md exists and covers all versions
- [x] Version is 1.0.0 in all config files (LAST step)
- [x] No hardcoded English strings in tab pages or ErrorBoundary
- [x] All i18n keys are present in all 4 locales
- [x] No Google Fonts external requests
- [x] Sprint commit message follows format: `feat(sprint-15): <summary>`

---

## Commit Message Template

```
feat(sprint-15): UX, documentation, and GA readiness

S15-001: Rewrite README as proper English product README
S15-002: Create CHANGELOG.md
S15-003: Bump version to 1.0.0
S15-004: Implement first-run onboarding wizard
S15-005: Add Rust doc comments to all modules
S15-006: Create frontend architecture documentation
S15-007: Fix hardcoded English strings in tab pages
S15-008: Fix consent dialog button styling
S15-009: Fix watermark pointer events
S15-010: Add accessible labels to WiFi password input
S15-011: Add role="status" to skeleton loaders
S15-012: Expand prefers-reduced-motion to all animations
S15-013: Make ErrorBoundary strings translatable
S15-014: Disclose Sentry in consent flow
S15-015: Add inclusive language and disclaimer to AI prompt
S15-016: Add keyboard shortcuts for top tabs
S15-017: Add error reporting channel in ErrorBoundary
S15-018: Add AI cost estimation and usage tracking
S15-019: Add CODEOWNERS file
S15-020: Add tsc --noEmit and version:check to pre-commit hooks
S15-021: Fix tray opacity (move from documentElement to wrapper)
S15-022: Fix latest.json generation in release workflow
```
