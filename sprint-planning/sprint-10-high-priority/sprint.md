# Sprint 10 — High Priority: Security, Performance & DevOps Hardening

## Sprint Metadata

| Field                 | Value                                                                                                               |
| --------------------- | ------------------------------------------------------------------------------------------------------------------- |
| **Sprint Name**       | High Priority — Security, Performance & DevOps Hardening                                                            |
| **Sprint Goal**       | Corrigir todos os 30+ findings HIGH do relatório de estabilidade, cobrindo segurança, performance, DevOps, RAI e UX |
| **Duration Estimate** | 2.5 semanas (12.5 dias úteis)                                                                                       |
| **Priority**          | P1 — Essencial para qualidade pós-GA, mas não bloqueia release                                                      |
| **Sprint Type**       | Multi-domain                                                                                                        |
| **Primary Owner**     | Full-stack engineer                                                                                                 |
| **Secondary Owner**   | DevOps engineer                                                                                                     |
| **Source**            | `docs/stability-report-2026-06-24.md`                                                                               |
| **Depends On**        | Sprint 9 (GA Blockers)                                                                                              |

## Sprint Goal Statement

Após eliminar os blockers CRITICAL no Sprint 9, este sprint endereça todos os findings HIGH restantes. Eles cobrem: segurança (ACL, elevation bypass, unsafe pointers, CSP, IPC), performance (WMI cache bypass), DevOps (assinatura, audit, pre-commit, crash reporting, branch protection), RAI (deleção de dados, logs, acessibilidade), UX (sidebar, loading, i18n, RTL), e code quality (unwrap, packed structs).

---

## Tickets

### S10-001 — Fix ACL restriction: use SetSecurityInfo instead of icacls

| Field                | Value                                                                   |
| -------------------- | ----------------------------------------------------------------------- |
| **Ticket ID**        | S10-001                                                                 |
| **Title**            | Substituir `icacls` por `SetSecurityInfo` Win32 API na restrição de ACL |
| **Priority**         | P1                                                                      |
| **Type**             | Security                                                                |
| **Estimated Effort** | M                                                                       |
| **Source Finding**   | S2 (HIGH, CWE-391)                                                      |

#### Description

`restrict_file_acl` em `auth.rs:150-170` executa `icacls.exe` e ignora falhas silenciosamente. Se `icacls` falhar (antivirus, path length), o arquivo de chave herda a ACL default do `%LOCALAPPDATA%`.

#### Acceptance Criteria

- [x] Substituir shell-out para `icacls` por `SetSecurityInfo` / `SetNamedSecurityInfo` Win32 API
- [x] Retornar `Result` em vez de ignorar falhas
- [x] Se a restrição falhar, retornar erro e não usar a chave
- [x] Teste: simular falha na restrição — deve retornar erro

---

### S10-002 — Always pass --request-id to scheduled task; add nonce anti-replay

| Field                | Value                                                                        |
| -------------------- | ---------------------------------------------------------------------------- |
| **Ticket ID**        | S10-002                                                                      |
| **Title**            | Sempre passar `--request-id` ao scheduled task e adicionar nonce anti-replay |
| **Priority**         | P1                                                                           |
| **Type**             | Security                                                                     |
| **Estimated Effort** | M                                                                            |
| **Source Finding**   | S3 (HIGH, CWE-73)                                                            |

#### Description

`elevated.rs:95-120` — quando nenhum `--request-id` é fornecido, o helper pega o arquivo mais novo. Um atacante pode dropar um arquivo mais novo e ter seu comando executado.

#### Acceptance Criteria

- [x] Sempre passar `--request-id` ao scheduled task
- [x] Adicionar nonce no comando e verificar no helper (rejeitar duplicados)
- [x] Limitar o scan de diretório a no máximo 10 arquivos
- [x] Teste: comando sem `--request-id` deve falhar

---

### S10-003 — Fix unsafe pointer casts in keyboard hook and IPC header

| Field                | Value                                                               |
| -------------------- | ------------------------------------------------------------------- |
| **Ticket ID**        | S10-003                                                             |
| **Title**            | Corrigir unsafe pointer casts em keyboard hook e IPC header parsing |
| **Priority**         | P1                                                                  |
| **Type**             | Security                                                            |
| **Estimated Effort** | M                                                                   |
| **Source Finding**   | S4/S6 (HIGH, CWE-822)                                               |

#### Description

`hotkeys.rs:1030` faz `l_param.0 as *const KBDLLHOOKSTRUCT` sem validação. `iotservice.rs:168` faz cast similar para `IpcWireHeader`.

#### Acceptance Criteria

- [x] Usar `std::ptr::read_unaligned` com size validation em `hotkeys.rs:1030`
- [x] Usar `bytemuck::from_bytes` ou validação manual para `IpcWireHeader` em `iotservice.rs:168`
- [x] Adicionar `bytemuck` crate ao `Cargo.toml` se necessário
- [x] Safety comments em todos os blocos modificados

---

### S10-004 — Remove unsafe-inline from CSP

| Field                | Value                                              |
| -------------------- | -------------------------------------------------- |
| **Ticket ID**        | S10-004                                            |
| **Title**            | Remover `unsafe-inline` do Content Security Policy |
| **Priority**         | P1                                                 |
| **Type**             | Security                                           |
| **Estimated Effort** | S                                                  |
| **Source Finding**   | S5 (HIGH, CWE-79)                                  |

#### Description

`tauri.conf.json` CSP inclui `style-src 'self' 'unsafe-inline'`. Isso abre porta para CSS injection attacks.

#### Acceptance Criteria

- [x] Remover `'unsafe-inline'` de `style-src`
- [x] Se necessário, usar nonce ou hash para estilos inline
- [x] Verificar que todos os estilos carregam corretamente sem `unsafe-inline`
- [x] Teste: app funciona sem erros de CSP no console

---

### S10-005 — Route fan.rs and processes.rs through WMI cache

| Field                | Value                                                     |
| -------------------- | --------------------------------------------------------- |
| **Ticket ID**        | S10-005                                                   |
| **Title**            | Roteirizar `fan.rs` e `processes.rs` através do WMI cache |
| **Priority**         | P1                                                        |
| **Type**             | Performance                                               |
| **Estimated Effort** | M                                                         |
| **Source Finding**   | P1/P2 (HIGH, +100-400ms/batch)                            |

#### Description

`fan.rs:138-150` e `processes.rs:34-48` criam COM+WMI fresh a cada query, bypassando o cache thread-local. Isso adiciona 100-400ms por batch poll.

#### Acceptance Criteria

- [x] Substituir `COMLibrary::new()` + `WMIConnection::new()` por `wmi_cache::with_cimv2()` em `fan.rs`
- [x] Mesmo para `get_esif_readings()` em `fan.rs:104-112` — usar `wmi_cache::with_wmi()`
- [x] Mesmo para `processes.rs:34-48`
- [x] Medir tempo de batch antes/depois — deve reduzir 200-400ms
- [x] `cargo test` passa

---

### S10-006 — Add cargo audit and npm audit to CI

| Field                | Value                                                   |
| -------------------- | ------------------------------------------------------- |
| **Ticket ID**        | S10-006                                                 |
| **Title**            | Adicionar `cargo audit` e `npm audit` ao pipeline de CI |
| **Priority**         | P1                                                      |
| **Type**             | DevOps                                                  |
| **Estimated Effort** | S                                                       |
| **Source Finding**   | D4 (HIGH, CVE exposure)                                 |

#### Acceptance Criteria

- [x] Adicionar step `cargo audit` no job `rust` do `ci.yml`
- [x] Adicionar step `npm audit --audit-level=moderate` no job `frontend`
- [x] Falhar CI se vulnerabilidades HIGH/CRITICAL forem encontradas
- [x] Adicionar `cargo-audit` ao CI (instalar via `cargo install cargo-audit`)

---

### S10-007 — Add Authenticode code signing to release workflow

| Field                | Value                                                 |
| -------------------- | ----------------------------------------------------- |
| **Ticket ID**        | S10-007                                               |
| **Title**            | Adicionar assinatura Authenticode no release workflow |
| **Priority**         | P1                                                    |
| **Type**             | DevOps                                                |
| **Estimated Effort** | L                                                     |
| **Source Finding**   | D3 (HIGH, SmartScreen)                                |

#### Acceptance Criteria

- [x] Adicionar step de signing com `signtool` ou `azure-sign-tool` no `release.yml`
- [x] Usar secrets `WINDOWS_CERTIFICATE` e `WINDOWS_CERTIFICATE_PASSWORD`
- [x] Alinhar `docs/release.md` com o processo real de CI
- [x] Teste: release artifact deve ser assinado (verificar com `signtool verify`)

---

### S10-008 — Add pre-commit hooks (husky + lint-staged)

| Field                | Value                                              |
| -------------------- | -------------------------------------------------- |
| **Ticket ID**        | S10-008                                            |
| **Title**            | Adicionar pre-commit hooks com husky + lint-staged |
| **Priority**         | P1                                                 |
| **Type**             | DevOps                                             |
| **Estimated Effort** | S                                                  |
| **Source Finding**   | D5 (HIGH)                                          |

#### Acceptance Criteria

- [x] Instalar `husky` e `lint-staged`
- [x] Pre-commit: `cargo fmt --check`, `eslint --fix`, `prettier --write`
- [x] Configurar `lint-staged` para rodar apenas em arquivos staged
- [x] Documentar setup no README

---

### S10-009 — Add crash reporting (Sentry or similar)

| Field                | Value                                      |
| -------------------- | ------------------------------------------ |
| **Ticket ID**        | S10-009                                    |
| **Title**            | Adicionar crash reporting externo (Sentry) |
| **Priority**         | P1                                         |
| **Type**             | DevOps                                     |
| **Estimated Effort** | L                                          |
| **Source Finding**   | D6 (HIGH)                                  |

#### Acceptance Criteria

- [x] Integrar Sentry SDK no Rust backend (via `sentry` crate)
- [x] Integrar Sentry SDK no frontend (via `@sentry/react`)
- [x] Crash reports respeitam consent de telemetria
- [x] Configurar DSN via environment variable
- [x] Documentar setup em `docs/`

---

### S10-010 — Document branch protection rules

| Field                | Value                                           |
| -------------------- | ----------------------------------------------- |
| **Ticket ID**        | S10-010                                         |
| **Title**            | Documentar e configurar branch protection rules |
| **Priority**         | P1                                              |
| **Type**             | DevOps                                          |
| **Estimated Effort** | S                                               |
| **Source Finding**   | D7 (HIGH)                                       |

#### Acceptance Criteria

- [x] Documentar required status checks em `docs/branch-protection.md`
- [x] Lista de checks obrigatórios: `rust`, `frontend`, `version:check`
- [x] Documentar required reviews (mínimo 1)
- [x] Documentar regras para `master` branch

---

### S10-011 — Fix tauri-smoke continue-on-error

| Field                | Value                                                |
| -------------------- | ---------------------------------------------------- |
| **Ticket ID**        | S10-011                                              |
| **Title**            | Corrigir `tauri-smoke` com `continue-on-error: true` |
| **Priority**         | P1                                                   |
| **Type**             | DevOps                                               |
| **Estimated Effort** | XS                                                   |
| **Source Finding**   | D8 (HIGH)                                            |

#### Acceptance Criteria

- [x] Remover `continue-on-error: true` do job `tauri-smoke`
- [x] Ou mover para workflow nightly separado
- [x] Se mantido em PR, deve falhar CI se build quebrar

---

### S10-012 — Add "Delete All My Data" + log retention policy

| Field                | Value                                                                |
| -------------------- | -------------------------------------------------------------------- |
| **Ticket ID**        | S10-012                                                              |
| **Title**            | Adicionar mecanismo de deleção de dados e retention policy para logs |
| **Priority**         | P1                                                                   |
| **Type**             | RAI                                                                  |
| **Estimated Effort** | L                                                                    |
| **Source Finding**   | R6/R7 (HIGH, GDPR Art.17)                                            |

#### Acceptance Criteria

- [x] Adicionar botão "Delete All My Data" em SettingsPage
- [x] Deletar: localStorage logs, AppData JSONL files, credential store entries, schedule data
- [x] Implementar auto-rotação de `ai_perf_logs` (30 dias)
- [x] Adicionar UI para deletar logs individualmente
- [x] Limpar schedule data quando consent for revogado

---

### S10-013 — Add aria-labels to emoji icons and sidebar nav

| Field                | Value                                                           |
| -------------------- | --------------------------------------------------------------- |
| **Ticket ID**        | S10-013                                                         |
| **Title**            | Adicionar `aria-label` em emoji icons e `aria-label` na sidebar |
| **Priority**         | P1                                                              |
| **Type**             | RAI / UX                                                        |
| **Estimated Effort** | S                                                               |
| **Source Finding**   | R9/U6/R26 (HIGH, WCAG)                                          |

#### Acceptance Criteria

- [x] Adicionar `aria-hidden="true"` em todos os spans de emoji
- [x] Adicionar `aria-label` descritivo em cada item de nav
- [x] Adicionar `aria-label="Main navigation"` na sidebar `<nav>`
- [x] Teste com screen reader: navegação deve ser anunciada corretamente

---

### S10-014 — Add role="alert" to dynamic status messages

| Field                | Value                                                     |
| -------------------- | --------------------------------------------------------- |
| **Ticket ID**        | S10-014                                                   |
| **Title**            | Adicionar `role="alert"` em mensagens dinâmicas de status |
| **Priority**         | P1                                                        |
| **Type**             | RAI                                                       |
| **Estimated Effort** | XS                                                        |
| **Source Finding**   | R13 (HIGH, WCAG)                                          |

#### Acceptance Criteria

- [x] Adicionar `role="alert"` em mensagens de erro/sucesso em `AiAdvisor.tsx`
- [x] Adicionar `role="alert"` em mensagens de erro em `SettingsPage.tsx`
- [x] Verificar que screen readers anunciam mudanças dinâmicas

---

### S10-015 — Add high-contrast mode support

| Field                | Value                                            |
| -------------------- | ------------------------------------------------ |
| **Ticket ID**        | S10-015                                          |
| **Title**            | Adicionar suporte a high-contrast mode (Windows) |
| **Priority**         | P1                                               |
| **Type**             | RAI / UX                                         |
| **Estimated Effort** | M                                                |
| **Source Finding**   | R11 (HIGH, WCAG)                                 |

#### Acceptance Criteria

- [x] Adicionar `@media (prefers-contrast: more)` em `globals.css`
- [x] Garantir que CSS variables respondem a forced colors mode
- [x] Substituir inline styles que bypassam forced colors por CSS classes
- [x] Teste com Windows High Contrast Mode ativo

---

### S10-016 — Reduce min window width and add responsive breakpoints

| Field                | Value                                                 |
| -------------------- | ----------------------------------------------------- |
| **Ticket ID**        | S10-016                                               |
| **Title**            | Reduzir min-width e adicionar breakpoints responsivos |
| **Priority**         | P1                                                    |
| **Type**             | UX                                                    |
| **Estimated Effort** | M                                                     |
| **Source Finding**   | R12/U17 (HIGH)                                        |

#### Acceptance Criteria

- [x] Reduzir `minWidth` de 800 para 600 em `tauri.conf.json`
- [x] Adicionar media queries em `globals.css` para sidebar colapsar abaixo de 700px
- [x] `grid-2` colapsa para 1 coluna abaixo de 600px
- [x] Teste em janela 600x560 — layout deve ser usável

---

### S10-017 — Group sidebar into sections and gate debug tools

| Field                | Value                                                                  |
| -------------------- | ---------------------------------------------------------------------- |
| **Ticket ID**        | S10-017                                                                |
| **Title**            | Agrupar sidebar em seções e gatear debug tools atrás de developer mode |
| **Priority**         | P1                                                                     |
| **Type**             | UX                                                                     |
| **Estimated Effort** | L                                                                      |
| **Source Finding**   | U3/U9 (HIGH, cognitive overload)                                       |

#### Acceptance Criteria

- [x] Agrupar 18 tabs em seções: Core (Overview, Performance, Battery, Display, Audio, Touchpad, Fan), Settings (WiFi, Keyboard, Startup, AI, Settings), Diagnostics (EC Debug, Screen Cast, Update)
- [x] Adicionar section headers visuais na sidebar
- [x] Gatear "EC Debug Panel" atrás de flag `MICONTROL_DEV_MODE` ou setting
- [x] Adicionar skip-to-content link

---

### S10-018 — Add loading indicators for audio device list and battery info

| Field                | Value                                                          |
| -------------------- | -------------------------------------------------------------- |
| **Ticket ID**        | S10-018                                                        |
| **Title**            | Adicionar loading indicators para audio devices e battery info |
| **Priority**         | P1                                                             |
| **Type**             | UX                                                             |
| **Estimated Effort** | S                                                              |
| **Source Finding**   | U4/U14 (HIGH)                                                  |

#### Acceptance Criteria

- [x] Adicionar spinner/skeleton em `AudioControl.tsx` enquanto carrega device list
- [x] Adicionar skeleton em `BatteryInfo` quando `hw.battery` é null
- [x] Adicionar CSS spinner animation em `globals.css`
- [x] Adicionar loading state para keyboard config

---

### S10-019 — Fix hardcoded English strings (i18n regression)

| Field                | Value                                                 |
| -------------------- | ----------------------------------------------------- |
| **Ticket ID**        | S10-019                                               |
| **Title**            | Corrigir strings hardcoded em inglês (não traduzidas) |
| **Priority**         | P1                                                    |
| **Type**             | UX                                                    |
| **Estimated Effort** | S                                                     |
| **Source Finding**   | U7 (HIGH, i18n regression)                            |

#### Acceptance Criteria

- [x] Auditar `AudioControl.tsx` — envolver "🎵 Audio Control", "Playback Devices" em `t()`
- [x] Auditar `MainWindow.tsx` — envolver "AI Mode Logs", "Logging active", "Configure API key" em `t()`
- [x] Adicionar keys faltantes em `en.json`, `pt.json`, `es.json`, `fr.json`
- [x] Grep por strings não traduzidas em todos componentes

---

### S10-020 — Add RTL support foundation

| Field                | Value                               |
| -------------------- | ----------------------------------- |
| **Ticket ID**        | S10-020                             |
| **Title**            | Adicionar fundação para RTL support |
| **Priority**         | P1                                  |
| **Type**             | UX                                  |
| **Estimated Effort** | M                                   |
| **Source Finding**   | U8 (HIGH)                           |

#### Acceptance Criteria

- [x] Adicionar `dir` attribute no `<html>` baseado no locale
- [x] Migrar propriedades CSS direcionais para logical properties (`margin-inline-start`, `inset-inline-end`)
- [x] Adicionar suporte para `dir="rtl"` em `globals.css`
- [x] Não é necessário adicionar locale RTL agora, apenas preparar a base

---

### S10-021 — Fix unwrap() in serde_json, current_exe, and default_window_icon

| Field                | Value                                                                                  |
| -------------------- | -------------------------------------------------------------------------------------- |
| **Ticket ID**        | S10-021                                                                                |
| **Title**            | Corrigir `unwrap()` em `serde_json::to_string`, `current_exe`, e `default_window_icon` |
| **Priority**         | P1                                                                                     |
| **Type**             | Code Quality                                                                           |
| **Estimated Effort** | S                                                                                      |
| **Source Finding**   | C5/C6/C7 (HIGH, panic risk)                                                            |

#### Acceptance Criteria

- [x] `discovery.rs:933` — substituir `.unwrap()` por `.map_err()?`
- [x] `iotservice.rs:1022` — tratar erro de `current_exe()` e `File::open()` gracefulmente
- [x] `lib.rs:283` — fallback para ícone padrão se `default_window_icon()` retornar None
- [x] `cargo test` passa

---

### S10-022 — Document or fix #[repr(C, packed)] IPC structs

| Field                | Value                                                      |
| -------------------- | ---------------------------------------------------------- |
| **Ticket ID**        | S10-022                                                    |
| **Title**            | Documentar ou corrigir `#[repr(C, packed)]` em IPC structs |
| **Priority**         | P1                                                         |
| **Type**             | Code Quality                                               |
| **Estimated Effort** | M                                                          |
| **Source Finding**   | C8 (HIGH, UB on ARM)                                       |

#### Acceptance Criteria

- [x] Avaliar se `packed` é necessário em `charging.rs:54` e `iotservice.rs`
- [x] Se necessário: documentar por que e adicionar safety comments
- [x] Se não necessário: mudar para `#[repr(C)]`
- [x] Usar `bytemuck` para conversão segura onde aplicável

---

## Sprint Exit Criteria

- [x] Todos os 22 tickets completos
- [x] `cargo test` + `cargo clippy` + `npm run build` + `npm run lint` passam
- [x] Relatório de estabilidade: 0 findings HIGH restantes
- [x] CI pipeline com `cargo audit` + `npm audit` ativos
- [x] Pre-commit hooks funcionando
