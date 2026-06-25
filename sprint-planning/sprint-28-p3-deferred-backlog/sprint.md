# Sprint 28 — P3 LOW: Deferred Backlog Cleanup (Post-Audit v3)

## Sprint Metadata

| Field                 | Value                                                                       |
| --------------------- | --------------------------------------------------------------------------- |
| **Sprint Name**       | Deferred Backlog Cleanup                                                    |
| **Sprint Goal**       | Resolve all remaining deferred findings from v1/v2 audits not yet addressed |
| **Duration Estimate** | ~5 days                                                                     |
| **Priority**          | P3 — Low                                                                    |
| **Sprint Type**       | Multi-domain (Backend, Frontend, Architecture, DevOps, RAI)                 |
| **Primary Owner**     | Full-stack engineer                                                         |
| **Source**            | `sprint-planning/sprint-overview.md` — Deferred to Sprint 20+ section       |
| **Depends On**        | Sprint 27                                                                   |

## ⚠️ MANDATORY COMPLETION REQUIREMENT

> **OBRIGATÓRIO: 100% dos tickets desta sprint devem ser concluídos. A sprint não será aceita como entregue se qualquer ticket permanecer incompleto.**
>
> **MANDATORY: 100% of the tickets in this sprint MUST be completed. The sprint will NOT be accepted as delivered if any ticket remains incomplete.**

Every ticket must pass its acceptance criteria AND the full health check suite (9/9) before the sprint commit is made.

---

## Sprint Goal Statement

A thorough investigation of the "Deferred to Sprint 20+" backlog revealed that 6 items are already RESOLVED, 3 are PARTIALLY RESOLVED, and 11 are STILL ISSUES. This sprint addresses all remaining unresolved items across 4 batches:

- **Batch A (Frontend i18n & Accessibility):** EcrDebugPanel i18n, AiConfigForm hardcoded strings, type extraction
- **Batch B (Architecture & Refactoring):** useSettings split, IoT command consolidation, hotkeys.rs module split, global statics consolidation
- **Batch C (Backend Hardening):** Configurable EC RAM safe list, E2E test setup
- **Batch D (DevOps & RAI):** LICENSE file, AI feedback mechanism, AI response caching, model version logging, AI documentation

### Already Resolved (no action needed)

| Finding                            | Resolution                                                              |
| ---------------------------------- | ----------------------------------------------------------------------- |
| U4: OnboardingWizard accessibility | ✅ Resolved in S24-010 (role="dialog", focus trap, Escape handler)      |
| U5: ConsentDialog focus ring       | ✅ Resolved (global `*:focus-visible` CSS, no bare `outline: none`)     |
| Q10: Duplicate type definitions    | ✅ Resolved (each type defined once, imported where needed)             |
| S7: shell:default capability       | ✅ Resolved (only `core:default` granted, no shell permissions exposed) |
| S13: Support scripts in root       | ✅ Resolved (all scripts in `scripts/` directory)                       |
| S14: Rust crate versions           | ✅ Acceptable (Cargo.lock committed, standard practice)                 |

### Partially Resolved (minor items, included in batch)

| Finding                 | Status                             | Action                              |
| ----------------------- | ---------------------------------- | ----------------------------------- |
| Q11: TODO in hotkeys.rs | Roadmap items, not broken code     | Document as known roadmap (S28-009) |
| T16-T19: Stability      | osd.rs expect() already in S27-006 | No additional action                |
| D11-D12: DevOps         | CI comprehensive, LICENSE missing  | S28-010                             |

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

## Batch A — Frontend i18n & Accessibility (S28-001 through S28-003)

### S28-001 — Add i18n to EcrDebugPanel

| Field                | Value                                                                   |
| -------------------- | ----------------------------------------------------------------------- |
| **Ticket ID**        | S28-001                                                                 |
| **Title**            | Add `useI18n` and `t()` to EcrDebugPanel, replace all hardcoded English |
| **Priority**         | P3                                                                      |
| **Type**             | i18n / Accessibility                                                    |
| **Estimated Effort** | S                                                                       |
| **Source Finding**   | U6 (LOW)                                                                |

#### Context

`src/components/EcrDebugPanel.tsx` has zero i18n — every user-visible string is hardcoded English. The file does not import `useI18n` or `t()`. Additionally, no `aria-label` attributes exist on any button or input.

Hardcoded strings include: "🔧 EC Debug Panel", "Direct EC RAM access (advanced)", "Read ECRAM", "Write ECRAM", "📋 Read ECRAM Map", "Result", and all placeholder texts.

#### Acceptance Criteria

- [ ] `useI18n` imported and `t()` used for all user-visible strings
- [ ] New i18n keys added to all 4 locale files (`en.json`, `pt.json`, `es.json`, `fr.json`)
- [ ] `aria-label` added to all buttons and inputs
- [ ] `npx tsc --noEmit` passes, `npm run lint` passes, `npm run build` passes

---

### S28-002 — Fix hardcoded English in AiConfigForm

| Field                | Value                                                            |
| -------------------- | ---------------------------------------------------------------- |
| **Ticket ID**        | S28-002                                                          |
| **Title**            | Replace hardcoded PRESET_MODELS labels and aria-label with `t()` |
| **Priority**         | P3                                                               |
| **Type**             | i18n                                                             |
| **Estimated Effort** | XS                                                               |
| **Source Finding**   | U6 (LOW)                                                         |

#### Context

`src/components/AiConfigForm.tsx` imports `t()` and uses it for most labels, but:

- Lines 12-17: `PRESET_MODELS` labels are hardcoded English (`"GPT-4o Mini (fast, cheap)"`, etc.)
- Line 135: `aria-label` is hardcoded English (`"Show API key"` / `"Hide API key"`) while `title` correctly uses `t()`

#### Acceptance Criteria

- [ ] PRESET_MODELS labels use `t()` or i18n keys
- [ ] `aria-label` uses `t('settings.showKey')` / `t('settings.hideKey')` (or appropriate keys)
- [ ] New keys added to all 4 locale files if they don't exist
- [ ] `npx tsc --noEmit` passes, `npm run lint` passes, `npm run build` passes

---

### S28-003 — Extract co-located types to `src/types/`

| Field                | Value                                                            |
| -------------------- | ---------------------------------------------------------------- |
| **Ticket ID**        | S28-003                                                          |
| **Title**            | Move hardware and settings types from hook files to `src/types/` |
| **Priority**         | P3                                                               |
| **Type**             | Architecture / Code Quality                                      |
| **Estimated Effort** | M                                                                |
| **Source Finding**   | Q16 (LOW)                                                        |

#### Context

`src/types/` directory contains only `error.ts`. 17+ types are co-located in `src/hooks/useHardware.ts` (PerformanceMode, SystemInfo, BatteryInfo, DisplayInfo, FanInfo, etc.) and `src/hooks/useSettings.ts` (AppSettings, SystemContext, AnalysisLogEntry). Types should be extracted to dedicated files.

#### Acceptance Criteria

- [ ] `src/types/hardware.ts` created with all hardware-related types
- [ ] `src/types/settings.ts` created with AppSettings, SystemContext, AnalysisLogEntry
- [ ] All imports updated across the codebase
- [ ] `npx tsc --noEmit` passes, `npm run lint` passes, `npm run build` passes

---

## Batch B — Architecture & Refactoring (S28-004 through S28-007)

### S28-004 — Split useSettings hook into focused hooks

| Field                | Value                                                                             |
| -------------------- | --------------------------------------------------------------------------------- |
| **Ticket ID**        | S28-004                                                                           |
| **Title**            | Extract AI analysis and consent management from `useSettings` into separate hooks |
| **Priority**         | P3                                                                                |
| **Type**             | Architecture / Refactoring                                                        |
| **Estimated Effort** | L                                                                                 |
| **Source Finding**   | Q6, A6 (LOW)                                                                      |

#### Context

`src/hooks/useSettings.ts` is a ~430-line "God object" mixing:

1. Settings persistence (loadSettings, persistSettings, STORAGE_KEY)
2. API key migration (migrateApiKey)
3. AI prompt building (buildPrompt — 50+ lines)
4. AI analysis (analyzeSystem, testConnection)
5. AI log analysis (analyzeWithLogs — 100-line function with inline prompt)
6. Telemetry consent management (getTelemetryConsent, setTelemetryConsent, revokeTelemetryConsent)

The hook returns 12 functions/values. It should be split into focused hooks.

#### Acceptance Criteria

- [ ] `src/hooks/useAiAnalysis.ts` created — contains analyzeSystem, analyzeWithLogs, testConnection, buildPrompt
- [ ] `src/hooks/useTelemetryConsent.ts` created — contains consent management functions
- [ ] `src/lib/aiPromptBuilder.ts` created — contains buildPrompt and prompt construction logic
- [ ] `useSettings.ts` retains only settings persistence and API key management
- [ ] All callers updated to import from new hooks
- [ ] `npx tsc --noEmit` passes, `npm run lint` passes, `npm run build` passes

---

### S28-005 — Consolidate granular IoT IPC commands

| Field                | Value                                                         |
| -------------------- | ------------------------------------------------------------- |
| **Ticket ID**        | S28-005                                                       |
| **Title**            | Consolidate ~25 granular IoT commands into composite commands |
| **Priority**         | P3                                                            |
| **Type**             | Architecture / API Design                                     |
| **Estimated Effort** | M                                                             |
| **Source Finding**   | A7 (LOW)                                                      |

#### Context

`src-tauri/src/commands/hardware.rs` has ~25 IoT-specific commands, many of which are thin wrappers. For example, `get_iot_model`, `get_iot_fw_version`, `get_iot_device_id`, `get_iot_bind_status`, and `get_iot_device_status` are all simple property getters that could be a single `get_iot_device_info` command. WiFi commands (`get_iot_wifi_status`, `get_iot_wifi_count`, `get_iot_wifi_by_index`) could be a single `get_iot_wifi_list`. Power event commands could be a single `iot_notify_event` with an enum.

#### Acceptance Criteria

- [ ] Property getter commands consolidated into `get_iot_device_info` (returns composite struct)
- [ ] WiFi commands consolidated into `get_iot_wifi_list` (returns list with status)
- [ ] Power event commands consolidated into `iot_notify_event` with enum parameter
- [ ] Old commands kept as deprecated wrappers (or removed if no frontend callers)
- [ ] Frontend callers updated
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S28-006 — Split hotkeys.rs into submodules

| Field                | Value                                                    |
| -------------------- | -------------------------------------------------------- |
| **Ticket ID**        | S28-006                                                  |
| **Title**            | Split `hotkeys.rs` (~2700 lines) into focused submodules |
| **Priority**         | P3                                                       |
| **Type**             | Architecture / Code Quality                              |
| **Estimated Effort** | L                                                        |
| **Source Finding**   | A9-A12 (LOW)                                             |

#### Context

`src-tauri/src/hw/hotkeys.rs` is ~2700 lines (tests start at line 2388). It handles config persistence, hook installation, keyboard hook proc, raw input, WMI HID listener, HID device reader, action dispatch, script security, and key remapping — all in one file.

#### Acceptance Criteria

- [ ] `src-tauri/src/hw/hotkeys/mod.rs` — re-exports, module-level docs
- [ ] `src-tauri/src/hw/hotkeys/config.rs` — HotkeyMap, KeyBinding, config persistence
- [ ] `src-tauri/src/hw/hotkeys/hook.rs` — hook installation, keyboard_hook_proc, raw_input_wnd_proc
- [ ] `src-tauri/src/hw/hotkeys/actions.rs` — dispatch_action, HotkeyAction handling
- [ ] `src-tauri/src/hw/hotkeys/wmi.rs` — WMI HID listener
- [ ] `src-tauri/src/hw/hotkeys/hid_reader.rs` — direct HID device reader
- [ ] `src-tauri/src/hw/hotkeys/script_security.rs` — script allowlist, consent
- [ ] `src-tauri/src/hw/hotkeys/remap.rs` — RemapToKey logic
- [ ] All tests pass, no behavior changes
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S28-007 — Consolidate global statics into state structs

| Field                | Value                                                       |
| -------------------- | ----------------------------------------------------------- |
| **Ticket ID**        | S28-007                                                     |
| **Title**            | Group 48 global statics into state structs held in AppState |
| **Priority**         | P3                                                          |
| **Type**             | Architecture / Code Quality                                 |
| **Estimated Effort** | L                                                           |
| **Source Finding**   | A5 (LOW)                                                    |

#### Context

48 `static` declarations exist across `src-tauri/src/`. The worst offenders are `hw/osd.rs` (10 statics for OSD window state) and `hw/hotkeys.rs` (12 statics for hook/remap state). These could be consolidated into state structs.

#### Acceptance Criteria

- [ ] `OsdState` struct created, holding OSD_HWND, OSD_LEVEL, OSD_HIDE_VER, OSD_ALPHA, etc.
- [ ] `HotkeyState` struct created, holding HOOK_HANDLE, RAW_INPUT_ACTIVE, HOOK_THREAD_ID, etc.
- [ ] State structs held in `AppState` or passed via Tauri managed state
- [ ] Thread-safe access patterns maintained (Mutex/RwLock as needed)
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

## Batch C — Backend Hardening (S28-008 through S28-009)

### S28-008 — Make EC RAM safe write list configurable

| Field                | Value                                                                 |
| -------------------- | --------------------------------------------------------------------- |
| **Ticket ID**        | S28-008                                                               |
| **Title**            | Load EC RAM safe write offsets from config file instead of hardcoding |
| **Priority**         | P3                                                                    |
| **Type**             | Architecture / Configurability                                        |
| **Estimated Effort** | S                                                                     |
| **Source Finding**   | S10 (LOW)                                                             |

#### Context

In `src-tauri/src/commands/hardware.rs:432-441`, the safe write list is hardcoded: 9 EC RAM offsets (`0x1B, 0x40, 0x42, 0x4A, 0x4B, 0x68, 0x96, 0xAE, 0xB2`). Adding a new safe offset requires a code change and recompilation. This should be configurable via a JSON file loaded at startup, similar to `hotkeys.json` and `driverstore-allowlist.json`.

#### Acceptance Criteria

- [ ] `scripts/ecram-safe-writes.json` created with the 9 current offsets
- [ ] `is_known_safe_single_byte_write` loads offsets from config file
- [ ] Falls back to hardcoded defaults if config file is missing
- [ ] Config file loaded once at startup (cached)
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S28-009 — Set up E2E testing with Playwright

| Field                | Value                                                      |
| -------------------- | ---------------------------------------------------------- |
| **Ticket ID**        | S28-009                                                    |
| **Title**            | Add Playwright E2E test framework with initial smoke tests |
| **Priority**         | P3                                                         |
| **Type**             | Testing / Quality                                          |
| **Estimated Effort** | M                                                          |
| **Source Finding**   | T11 (LOW)                                                  |

#### Context

No E2E testing framework exists. Only `vitest` unit/component tests are present. The app has no end-to-end tests verifying the full Tauri application flow (window launch, tab navigation, settings persistence).

#### Acceptance Criteria

- [ ] `@playwright/test` added to devDependencies
- [ ] `playwright.config.ts` created
- [ ] At least 3 smoke tests: app launches, tab navigation works, settings persist
- [ ] E2E tests added to CI workflow (separate job, allowed to fail initially)
- [ ] `npx tsc --noEmit` passes

---

## Batch D — DevOps & RAI (S28-010 through S28-014)

### S28-010 — Add LICENSE file

| Field                | Value                                        |
| -------------------- | -------------------------------------------- |
| **Ticket ID**        | S28-010                                      |
| **Title**            | Create MIT LICENSE file referenced in README |
| **Priority**         | P3                                           |
| **Type**             | DevOps / Legal                               |
| **Estimated Effort** | XS                                           |
| **Source Finding**   | D11-D12 (LOW)                                |

#### Context

`README.md:172` references `[MIT](LICENSE) © miPC contributors` and the badge at line 5 references `https://img.shields.io/github/license/Freitas-MA/miPC`, but no `LICENSE` file exists in the repository.

#### Acceptance Criteria

- [ ] `LICENSE` file created with MIT license text
- [ ] Copyright year and holder match README reference

---

### S28-011 — Add AI response feedback mechanism

| Field                | Value                                                      |
| -------------------- | ---------------------------------------------------------- |
| **Ticket ID**        | S28-011                                                    |
| **Title**            | Add thumbs up/down feedback buttons on AI analysis results |
| **Priority**         | P3                                                         |
| **Type**             | AI Responsibility / UX                                     |
| **Estimated Effort** | S                                                          |
| **Source Finding**   | R6-R12 (LOW)                                               |

#### Context

Users cannot rate or provide feedback on AI analysis quality. There is no feedback mechanism in `AiAnalysis.tsx` or `AiAdvisor.tsx`. Adding a simple thumbs up/down allows users to signal quality issues.

#### Acceptance Criteria

- [ ] Thumbs up/down buttons added below AI analysis results
- [ ] Feedback stored in localStorage (or sent to backend)
- [ ] Feedback includes analysis ID and timestamp
- [ ] `npx tsc --noEmit` passes, `npm run lint` passes, `npm run build` passes

---

### S28-012 — Add AI response caching

| Field                | Value                                                  |
| -------------------- | ------------------------------------------------------ |
| **Ticket ID**        | S28-012                                                |
| **Title**            | Cache AI analysis results to avoid redundant API calls |
| **Priority**         | P3                                                     |
| **Type**             | AI Responsibility / Performance                        |
| **Estimated Effort** | S                                                      |
| **Source Finding**   | R6-R12 (LOW)                                           |

#### Context

Each `analyze_system` call makes a new HTTP request to the AI provider. Identical system contexts will trigger redundant API calls, wasting tokens and increasing cost.

#### Acceptance Criteria

- [ ] Cache key based on system context hash (CPU, memory, battery, fan data)
- [ ] Cache stored in memory with 5-minute TTL
- [ ] Cache invalidated when hardware state changes significantly
- [ ] Cache hit logged for debugging
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S28-013 — Log AI model version in usage stats

| Field                | Value                                           |
| -------------------- | ----------------------------------------------- |
| **Ticket ID**        | S28-013                                         |
| **Title**            | Track which AI model was used in `AiUsageStats` |
| **Priority**         | P3                                              |
| **Type**             | AI Responsibility / Observability               |
| **Estimated Effort** | XS                                              |
| **Source Finding**   | R6-R12 (LOW)                                    |

#### Context

`AiUsageStats` tracks `total_requests`, token counts, and `estimated_cost_usd`, but does not record which model was used. The `model` parameter is passed to `analyze_system` but only used for the API request, never logged.

#### Acceptance Criteria

- [ ] `AiUsageStats` gains `model_usage: HashMap<String, u64>` field (model name → request count)
- [ ] `record_usage()` updated to accept and track model name
- [ ] `save_to_file()` / `load_from_file()` updated for new field
- [ ] `cargo check` passes, `cargo clippy -D warnings` passes, `cargo test` passes

---

### S28-014 — Add AI documentation for users

| Field                | Value                                                              |
| -------------------- | ------------------------------------------------------------------ |
| **Ticket ID**        | S28-014                                                            |
| **Title**            | Document AI features, capabilities, limitations, and data handling |
| **Priority**         | P3                                                                 |
| **Type**             | AI Responsibility / Documentation                                  |
| **Estimated Effort** | S                                                                  |
| **Source Finding**   | R6-R12 (LOW)                                                       |

#### Context

`README.md` does not mention which AI model is used or that AI features exist. No documentation about AI capabilities, limitations, or data handling for users. The model name is configurable but this isn't documented anywhere visible to end users.

#### Acceptance Criteria

- [ ] `docs/ai-features.md` created documenting:
  - What AI features exist (system analysis, log analysis)
  - What data is sent to the AI provider (system info, process names, hardware logs)
  - Which models are supported (OpenAI, Ollama, custom)
  - Privacy implications and consent requirements
  - Rate limiting and usage tracking
- [ ] Link from README to AI documentation
- [ ] In-app link to AI documentation from AiAnalysis settings

---

## Sprint Commit

```bash
git add -A
git commit -m "feat(s28): deferred backlog cleanup — i18n, architecture, E2E, RAI (P3)

Batch A - Frontend i18n & Accessibility:
- S28-001: EcrDebugPanel i18n and aria-labels (U6)
- S28-002: AiConfigForm hardcoded strings fix (U6)
- S28-003: Extract co-located types to src/types/ (Q16)

Batch B - Architecture & Refactoring:
- S28-004: Split useSettings into focused hooks (Q6, A6)
- S28-005: Consolidate IoT IPC commands (A7)
- S28-006: Split hotkeys.rs into submodules (A9-A12)
- S28-007: Consolidate global statics into state structs (A5)

Batch C - Backend Hardening:
- S28-008: Configurable EC RAM safe write list (S10)
- S28-009: Playwright E2E test setup (T11)

Batch D - DevOps & RAI:
- S28-010: MIT LICENSE file (D11-D12)
- S28-011: AI response feedback mechanism (R6-R12)
- S28-012: AI response caching (R6-R12)
- S28-013: Log AI model version in usage stats (R6-R12)
- S28-014: AI documentation for users (R6-R12)"
```
