# Auditoria documentação × código — 2026-11-11

**Branch:** `f5/docs-audit` (worktree `C:/Projects/miPC/wt/wt/f5-docs-audit`, base `integrate/audit-fixes`)
**Âmbito:** todos os 99 ficheiros `.md` do repositório (49 na raiz/`docs/`/`src-tauri/`/`public/` + 50 em `sprint-planning/`).
**Método:** cada afirmação verificável foi confrontada com o código real — `package.json` scripts, `scripts/health-check.mjs`, `.github/workflows/*.yml`, `.husky/pre-commit`, `src-tauri/src/**` (módulos `hw/`, `commands/`, `bin/`, `util/`), `src/**` (tabs, hooks, i18n, estilos) e `src-tauri/Cargo.toml`. Links relativos verificados com um scanner próprio (99 ficheiros, 11 links quebrados, todos em documentos históricos — ver §5.4).

**Regra aplicada:** relatórios datados (auditorias, RE, research) **não** foram reescritos. Onde o estado mudou desde a data, foi acrescentado um banner de estado — o conteúdo original fica intacto.

---

## 1. Inventário

`Estado`: **OK** = verificado e correcto · **corrigido** = tinha erros, foi corrigido nesta auditoria · **histórico** = snapshot datado, não deve ser actualizado · **deprecated** = substituído, marcado como tal · **pendente** = precisa de decisão do dono.

| Ficheiro                                                          | Última edição     | Bytes | Estado                              |
| ----------------------------------------------------------------- | ----------------- | ----- | ----------------------------------- |
| `README.md`                                                       | 2026-08-07        | 17504 | corrigido                           |
| `CONTRIBUTING.md`                                                 | 2026-08-07        | 5427  | corrigido                           |
| `AGENTS.md`                                                       | 2026-10-23        | 8584  | pendente (ficheiro protegido)       |
| `CHANGELOG.md`                                                    | 2026-11-08        | 63078 | OK                                  |
| `PRODUCT.md`                                                      | 2026-09-13        | 2362  | OK                                  |
| `SECURITY.md`                                                     | 2026-06-26        | 1538  | pendente (política)                 |
| `CODE_OF_CONDUCT.md`                                              | 2026-06-25        | 2292  | OK                                  |
| `AUDIT-REPORT.md`                                                 | 2026-08-03        | 25970 | histórico                           |
| `audit-i18n.md`                                                   | 2026-11-09        | 11616 | histórico                           |
| `memory-leak-audit.md`                                            | 2026-10-24        | 18157 | histórico                           |
| `audit-20261107/STAB-ROUND-1.md`                                  | 2026-11-10        | 33907 | histórico                           |
| `.github/PULL_REQUEST_TEMPLATE.md`                                | 2026-06-25        | 1209  | OK                                  |
| `docs/architecture.md`                                            | 2026-08-03        | 13865 | corrigido                           |
| `docs/frontend-architecture.md`                                   | 2026-06-30        | 11238 | corrigido                           |
| `docs/LOGGING.md`                                                 | 2026-11-07        | 10328 | OK                                  |
| `docs/adding-a-hardware-feature.md`                               | 2026-06-30        | 3189  | corrigido                           |
| `docs/ai-features.md`                                             | 2026-06-25        | 10187 | OK                                  |
| `docs/crash-reporting.md`                                         | 2026-06-24        | 1540  | OK                                  |
| `docs/privacy-policy-versioning.md`                               | 2026-06-24        | 3073  | corrigido                           |
| `docs/release.md`                                                 | 2026-08-07        | 11794 | corrigido                           |
| `docs/branch-protection.md`                                       | 2026-06-26        | 3533  | corrigido                           |
| `docs/installer-decoupling.md`                                    | 2026-10-23        | 5800  | OK                                  |
| `docs/edge-slide-protocol-re.md`                                  | 2026-11-08        | 21686 | corrigido (paths)                   |
| `docs/iotservice-re-analysis.md`                                  | 2026-06-30        | 21038 | corrigido (paths)                   |
| `docs/FACE_UNLOCK_PLAN.md`                                        | 2026-08-03        | 13431 | corrigido (estado)                  |
| `docs/HARDWARE_GAP_ANALYSIS.md`                                   | 2026-11-07        | 62173 | histórico + banner                  |
| `docs/CROSS_DEVICE_ALTERNATIVES_REPORT.md`                        | 2026-08-03        | 55398 | histórico + banner                  |
| `docs/HARDWARE_INVESTIGATION.md`                                  | 2026-08-03        | 63976 | pendente (ver §5)                   |
| `docs/RE_ANALYSIS_REPORT.md`                                      | 2026-08-03        | 17756 | OK (IOCTLs confirmados)             |
| `docs/EC_COMMAND_PROTOCOL_RE.md`                                  | 2026-08-03        | 7509  | OK (16 cmd_ids confirmados)         |
| `docs/UNDEREXPLOITED_HARDWARE_RESEARCH.md`                        | 2026-09-02        | 28565 | histórico                           |
| `docs/IOT_HARDWARE_OPPORTUNITIES_REPORT.md`                       | 2026-08-29        | 17185 | histórico                           |
| `docs/STABILITY_REPORT_v2.md`                                     | 2026-06-25        | 31150 | histórico + banner                  |
| `docs/STABILITY_REPORT_v3.md`                                     | 2026-06-25        | 30473 | histórico + banner                  |
| `docs/STABILITY_REPORT_v4.md`                                     | 2026-06-30        | 51432 | histórico (já tinha banner)         |
| `docs/stability-report-2026-06-24.md`                             | 2026-10-23        | 52226 | histórico                           |
| `docs/stability-report-2026-06-24-post-sprints.md`                | 2026-06-24        | 42351 | histórico                           |
| `docs/stability-report-2026-06-24-post-sprints-13-15.md`          | 2026-06-25        | 40665 | histórico                           |
| `docs/audit/post-sprint-22-25-audit-v3.md`                        | 2026-06-25        | 49791 | histórico                           |
| `docs/audit/battery-limit-debug.md`                               | 2026-11-10        | 16325 | OK (fix presente no código)         |
| `docs/audit/iot-command-audit.md`                                 | 2026-11-08        | 5789  | OK (comandos removidos confirmados) |
| `docs/code-review/2026-06-19-micontrol-comprehensive-audit.md`    | 2026-06-19        | 27952 | histórico                           |
| `docs/code-review/2026-06-19-micontrol-security-audit.md`         | 2026-06-19        | 27727 | histórico                           |
| `docs/code-review/2026-07-19-auth-bridge-uac-regression-audit.md` | 2026-07-19        | 39709 | histórico                           |
| `docs/deprecated/IOT_SHIM_REPORT.md`                              | 2026-06-30        | 7984  | deprecated (marcado)                |
| `docs/deprecated/README_legacy_2026-05-23.md`                     | 2026-06-30        | 51814 | deprecated (marcado)                |
| `src-tauri/bundle-targets.md`                                     | 2026-06-24        | 992   | OK (`targets: ["nsis"]` confirmado) |
| `public/fonts/README.md`                                          | 2026-09-14        | 338   | OK                                  |
| `sprint-planning/**` (50 ficheiros)                               | 2026-06 → 2026-11 | —     | histórico (planos de sprint)        |

---

## 2. Verificações factuais — o que estava errado

### 2.1 `README.md`

| Afirmação                                                                                                                       | Realidade                                                                                                                       | Acção                                                            |
| ------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| Node.js **20+**                                                                                                                 | `engines.node >= 24.0.0`, `.nvmrc` = 24.15.0                                                                                    | corrigido para 24+ e pnpm 11.5.0                                 |
| `cd miPC` após o clone                                                                                                          | o repositório chama-se `miControl`                                                                                              | corrigido                                                        |
| "Running Checks" listava só comandos soltos                                                                                     | `scripts/health-check.mjs` é a fonte única (CI e release invocam-no)                                                            | acrescentado `pnpm run health-check` + flags `--list`/`--step`   |
| Features: 8 bullets, sem Overview, Task Manager, OS Turbo, Startup Manager, Color, Cleanup, Security, Cross-Device, Face Unlock | 23 destinos de sidebar / 24 tabs lazy                                                                                           | 9 features novas documentadas a partir do código                 |
| "Manage … keyboard backlight"                                                                                                   | o nível de backlight é **só de leitura** (`ecram.rs` → `keyboard_backlight_level`, mostrado no painel IoT)                      | corrigido para "read the keyboard backlight level"               |
| "Frontend … Tailwind CSS"                                                                                                       | não existe Tailwind em `package.json` nem no lockfile; o CSS é escrito à mão com tokens                                         | corrigido                                                        |
| Acknowledgments: Tailwind CSS                                                                                                   | idem                                                                                                                            | substituído por GSAP/three.js/lucide/Sentry (dependências reais) |
| Pre-commit: "Run `rustfmt` on staged Rust files"                                                                                | `.husky/pre-commit` corre `lint-staged`, `cargo fmt` (workspace inteiro), `tsc --noEmit` e `version:check`                      | secção reescrita e movida para Contributing                      |
| Bloco "Full Tauri build / These checks run automatically in CI" pendurado **depois** de Acknowledgments                         | conteúdo duplicado e fora de sítio                                                                                              | removido                                                         |
| Tabela de documentação com 12 entradas                                                                                          | faltavam LOGGING, edge-slide, installer-decoupling, branch-protection, HARDWARE_GAP_ANALYSIS, FACE_UNLOCK_PLAN e este relatório | 7 linhas acrescentadas                                           |

### 2.2 `docs/architecture.md`

- Tabela HAL tinha **21 módulos**; `src-tauri/src/hw/` tem **55** (mais `hotkeys/` e `face/`). Reescrita em 5 grupos (energia/térmica, display/áudio/input, EC/firmware, sistema/manutenção, conectividade), com a descrição tirada do `//!` de cada módulo.
- `Build: npm, Cargo` → `pnpm, Cargo`.
- Faltava a camada de processos: acrescentada tabela `micontrol.exe`, `--elevated`, `micontrol_bridge.exe`, `ecram_service.exe`, `micontrol_face_svc.exe`, `micontrol_facecp.dll`, `micontrol_setup.exe` + mapa de `commands/`.
- Confirmado como correcto: IOCTLs `0x22E000`/`0x22E004`, pipe `\\.\pipe\ecram_service`, limitação ERAM/SMA2, modelo de segurança.

### 2.3 `docs/frontend-architecture.md`

- **`react-i18next` não existe.** O i18n é um módulo próprio (`src/hooks/useI18n.ts`) sobre `useSyncExternalStore`, com chaves tipadas a partir de `en.json`. Corrigido.
- **`Tailwind CSS` não existe.** Corrigido para CSS escrito à mão + tokens (`src/styles/globals.css`, `primitives.css`).
- "18 total tabs" → 23 destinos em 5 grupos, 24 tabs lazy. Diagrama e lista actualizados; `UpdatesTab` e `SetupTab` (que já não existem no `renderTab()`) removidos.
- Hooks: `useLanguage` não existe; acrescentados `useOverview`, `useAiAnalysis`, `useAutoUpdate`, `useBatteryCare`, `usePowerProfiles`, `useTelemetryConsent`.
- Routing: `NAV_ITEMS` → `NAV_GROUPS`, persistência em `localStorage` (`micontrol_active_tab`), atalhos `Alt+1..9`, busca na sidebar.
- "Type definitions are co-located in the hook files … rather than a separate `types/` folder" → existe `src/types/` com 4 ficheiros. Corrigido.
- "single-window" → três janelas (main, tray, brightness OSD).

### 2.4 `docs/release.md` — o documento mais desactualizado

Descrevia um pipeline que **não existe**:

- `auto-release.yml` — não existe em `.github/workflows/` (só `ci`, `codeql`, `driverstore-integrity`, `landing`, `release`, `warm-cache`).
- **release-please** — não há configuração nenhuma no repositório; o único sítio que ainda o menciona é um comentário em `scripts/release.cjs`.
- O fluxo real: `pnpm run release <patch|minor|major|X.Y.Z>` → `scripts/release.cjs` (exige árvore limpa, branch `main`, sincronizado com `origin/main`) → `sync-version.cjs` actualiza `package.json` + `Cargo.toml` + `tauri.conf.json` → commit `chore(release): vX.Y.Z` → tag anotada → push de `main` e da tag → `release.yml` (tag `v*` ou `workflow_dispatch` com input `tag`).

Reescritos: intro, diagrama, tabela de tipos de commit (a coluna "Release Trigger" era automática; agora é "bump a passar ao script"), secção de workflows (jobs reais `quality`/`test` do `ci.yml`, passos reais do `release.yml`, tabela dos restantes workflows com os triggers verificados), "Cutting a Release" e "Re-running a release build".

### 2.5 `docs/branch-protection.md`

- Checks obrigatórios `rust`, `frontend`, `version:check` **não existem**. O `ci.yml` tem dois jobs: `quality` ("Quality checks") e `test` ("Tests + coverage"), ambos a chamar `health-check.mjs`. Corrigido na tabela, na lista de regras e no comando `gh api`.
- "Require review from code owners: Yes (when CODEOWNERS file is added)" e "Create a `CODEOWNERS` file" → `.github/CODEOWNERS` **já existe**. Corrigido, com o conteúdo real.

### 2.6 `docs/privacy-policy-versioning.md`

Três afirmações falsas:

- "`useSettings()` exposes `getStoredPolicyVersion()`" — não existe em lado nenhum.
- "`MainWindow.tsx` checks both consent status and policy version on mount" — só verifica `consent === null`.
- "Update `POLICY_VERSION` in `src/pages/MainWindow.tsx`" — a constante só existe em `src-tauri/src/util/consent_audit.rs` (`= 2`).

Reescrito com o que existe: `POLICY_VERSION` no backend, log TSV `consent_audit.log` com `policy_version` + HMAC, chaves `consent_audit.key`/`.old`, consentimento no credential store (`telemetry_consent`, valores simples; JSON legado tolerado na leitura), `useTelemetryConsent`, evento `micontrol:consent-changed`. O fluxo de re-consentimento por mudança de versão **não está implementado** — registado como lacuna conhecida no próprio documento.

### 2.7 `docs/LOGGING.md` — verificado, sem alterações

Todas as afirmações batem com o código: `DEFAULT_MAX_BYTES = 8 MiB` e `DEFAULT_MAX_FILES = 7` (`util/log_rotation.rs`, com teste que os fixa), `APP_LOG_FILE`/`HELPER_LOG_FILE`/`DEV_*` (`debug_log.rs`), `session.marker` (`util/session_marker.rs`), deduplicação (`util/log_ratelimit.rs`), `duration_field` (`util/log_fields.rs`), `errors.log` (`util/error_log.rs`). É o documento mais preciso do repositório.

### 2.8 `docs/adding-a-hardware-feature.md`

Caminhos e padrões confirmados (`hw/errors.rs`, `commands/hardware.rs`, `generate_handler!` em `lib.rs`, `src/hooks/useHardware.ts`, `src-tauri/tests/`, comentários `WORKING FORM` em `battery.rs`/`ecram.rs`/`fan.rs`/`wmi_ec.rs`/`wmi_cache.rs`). Acrescentado o passo 7 (`pnpm run health-check`) e a nota de que o gate compila com `--features face`; título passou a "7-Step Checklist".

### 2.9 `docs/ai-features.md`, `docs/crash-reporting.md`, `docs/installer-decoupling.md`, `src-tauri/bundle-targets.md`

Verificados sem alterações: modelos preset (`gpt-4o-mini`/`gpt-4o`/`gpt-4-turbo` em `AiConfigForm.tsx`), `MAX_INPUT_LENGTH = 50_000` (`commands/ai.rs`), `consent_denied`, `ai_usage.json` com ACL restrita (`util/ai_usage.rs`); `sentry = "0.34"` + `SENTRY_DSN`/`VITE_SENTRY_DSN` e init condicionado ao consentimento (`App.tsx`); shell/payload e `publish-release.ps1`; `bundle.targets: ["nsis"]`.

### 2.10 Paths antigos

`~/Documents/Projects/miPC/...` → `C:/Projects/miPC/...` em `docs/edge-slide-protocol-re.md` (6 referências) e `docs/iotservice-re-analysis.md` (1). Neste último foi também anotado que `.agents/skills/ghidra/` já não existe no repositório (o `SCRIPTS` tem de apontar para uma cópia local de `ExportStrings.java`).

### 2.11 Estados desactualizados

- `docs/FACE_UNLOCK_PLAN.md`: "Status: Approved — implementation in progress" → **implementado** (`hw/face/`, `bin/micontrol_face_svc.rs`, `cp/`, feature `face`).
- `docs/STABILITY_REPORT_v2.md` e `v3.md`: não estavam marcados como históricos (só o v4 estava). Banners acrescentados. O v2 tem `Date: 2025-06-25` (typo do round de 2026-06-25) — deixado como registado, com nota.
- `docs/HARDWARE_GAP_ANALYSIS.md` e `docs/CROSS_DEVICE_ALTERNATIVES_REPORT.md`: snapshots a 0.1.13 cujos "gaps" já foram fechados em vários casos. Banner de estado a apontar para os módulos reais, sem tocar no conteúdo.

---

## 3. Correções aplicadas (ficheiros tocados)

`README.md`, `CONTRIBUTING.md`, `docs/architecture.md`, `docs/frontend-architecture.md`, `docs/release.md`, `docs/branch-protection.md`, `docs/privacy-policy-versioning.md`, `docs/adding-a-hardware-feature.md`, `docs/edge-slide-protocol-re.md`, `docs/iotservice-re-analysis.md`, `docs/FACE_UNLOCK_PLAN.md`, `docs/HARDWARE_GAP_ANALYSIS.md`, `docs/CROSS_DEVICE_ALTERNATIVES_REPORT.md`, `docs/STABILITY_REPORT_v2.md`, `docs/STABILITY_REPORT_v3.md`, `docs/DOCS-AUDIT-2026-11-11.md` (novo).

Nenhuma especificação foi inventada: cada linha acrescentada tem origem num ficheiro de código, num script ou num workflow.

---

## 4. Documentos em falta

| Tema                                 | Situação                                                                                                                                                                                                                                                               | Recomendação                                           |
| ------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| **Startup Manager**                  | Sem documento próprio. O módulo tem regras não óbvias (blobs `StartupApproved` de 12 bytes, vista WOW64, tarefas "at logon", encaminhamento elevado para `HKLM`/Startup comum, serviços deliberadamente excluídos) que só existem no `//!` de `hw/startup_manager.rs`. | Extrair esse cabeçalho para `docs/startup-manager.md`. |
| **Calibração de cor / ICC**          | Sem documento. `hw/color_calibration.rs` (mscms), `hw/icc_catalog.rs` (validação/download/instalação staged) e `hw/eye_protection.rs` não têm página.                                                                                                                  | `docs/color-calibration.md`.                           |
| **OS Turbo / System Optimization**   | Sem documento. `os_turbo*.rs` (política, stats, log, sugestões) é o subsistema mais complexo sem página.                                                                                                                                                               | `docs/os-turbo.md`.                                    |
| **Face Unlock (operacional)**        | Existe o plano de design, mas não há doc de operação (instalação do serviço, credential provider, modelos ONNX, troubleshooting).                                                                                                                                      | `docs/face-unlock.md`.                                 |
| **Cross-Device**                     | Só existe o relatório de research de 0.1.13; falta a doc do que está implementado (LocalSend, scrcpy, Syncthing, KDE Connect, NFC, BLE, transcrição).                                                                                                                  | `docs/cross-device.md`.                                |
| **Elevated bridge / protocolo HMAC** | Descrito em 3 sítios (README, architecture, installer-decoupling) mas sem especificação única do protocolo (`\\.\pipe\micontrol_bridge`, campos, nonce, allowlist).                                                                                                    | `docs/elevated-bridge.md`.                             |

---

## 5. Pendências que exigem decisão do dono

1. **`AGENTS.md` está protegido contra escrita** — não foi possível corrigir duas afirmações erradas: (a) "8 MiB per file, **3** rotated files kept" (o código usa **7**); (b) "**18** lazy-loaded tabs" (são **24**). Também falta a linha do `tauri-app-helper.log` na tabela de logs. Corrigir manualmente.
2. **`SECURITY.md`** declara suporte para `1.0.x` e "`< 1.0` não suportado", mas a versão actual é **0.2.20**. É uma afirmação de política — não foi alterada.
3. **`docs/HARDWARE_INVESTIGATION.md`** auto-intitula-se "✅ SINGLE SOURCE OF TRUTH — Last updated 2026-06-30", em conflito com `architecture.md`, `RE_ANALYSIS_REPORT.md` e `LOGGING.md`, que também reclamam autoridade sobre partes do mesmo tema. Sugestão: rebaixar para "consolidated findings (snapshot 2026-06-30)" e apontar para `architecture.md` como fonte viva.
4. **Links quebrados em docs históricos** (não corrigidos de propósito, para não reescrever história): `docs/deprecated/IOT_SHIM_REPORT.md` (8 links para `../src-tauri/...`, caminho relativo errado desde sempre) e `docs/deprecated/README_legacy_2026-05-23.md` (2 links para `src/i18n/en.json`). `sprint-planning/sprint-28-p3-deferred-backlog/sprint.md` tem 1 link para `LICENSE` com caminho relativo errado.
5. **Re-consentimento por versão de política** — a doc afirmava que existia; não existe. Ver §2.6.

---

## 6. Verificação de markdown

- `markdownlint` **não** está nas dependências do projecto (`package.json` não o lista) e `node_modules` não está instalado neste worktree — não foi possível correr um linter de markdown. `prettier` e `eslint` só cobrem `src/` (`lint-staged` aplica `prettier` a `src/**/*.md`, não a `docs/`).
- Verificação feita: scanner de links relativos sobre os 99 `.md` → **11 links quebrados**, todos em documentos históricos e listados em §5.4 (o link do README para este relatório foi criado e resolve).
- Tabelas, blocos de código e cabeçalhos dos ficheiros alterados foram revistos manualmente; nenhum bloco ficou aberto.
