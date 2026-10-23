# Sprint 12 — Low Priority: Polish, Documentation & Technical Debt

## Sprint Metadata

| Field                 | Value                                                                               |
| --------------------- | ----------------------------------------------------------------------------------- |
| **Sprint Name**       | Low Priority — Polish, Documentation & Technical Debt                               |
| **Sprint Goal**       | Endereçar todos os 20+ findings LOW do relatório de estabilidade e itens de backlog |
| **Duration Estimate** | 1.5 semanas (7.5 dias úteis)                                                        |
| **Priority**          | P3 — Nice-to-have, polish e documentação                                            |
| **Sprint Type**       | Multi-domain                                                                        |
| **Primary Owner**     | Full-stack engineer                                                                 |
| **Secondary Owner**   | Tech writer                                                                         |
| **Source**            | `docs/stability-report-2026-06-24.md`                                               |
| **Depends On**        | Sprint 9, 10, 11                                                                    |

## Sprint Goal Statement

Com todos os blockers CRITICAL, HIGH e MEDIUM resolvidos, este sprint final endereça a dívida técnica LOW: polish de UX, documentação faltante, otimizações menores de performance, e limpeza final. Após este sprint, a aplicação estará em um estado de qualidade excelente para GA.

---

## Tickets

### S12-001 — Gate Copilot remap modifier release on GetAsyncKeyState

| Field                | Value                                                          |
| -------------------- | -------------------------------------------------------------- |
| **Ticket ID**        | S12-001                                                        |
| **Title**            | Gatear modifier key-up em Copilot remap via `GetAsyncKeyState` |
| **Priority**         | P3                                                             |
| **Type**             | Security                                                       |
| **Estimated Effort** | XS                                                             |
| **Source Finding**   | S12 (LOW, CWE-367)                                             |

#### Acceptance Criteria

- [x] Em `do_remap_keydown`, só injetar `LShift up` / `LWin up` se `GetAsyncKeyState` indicar que estão pressionados
- [x] Não interferir com sequências de teclas simultâneas legítimas

---

### S12-002 — Add WMI HID listener reconnection logic

| Field                | Value                                                      |
| -------------------- | ---------------------------------------------------------- |
| **Ticket ID**        | S12-002                                                    |
| **Title**            | Adicionar reconnection logic para WMI HID listener threads |
| **Priority**         | P3                                                         |
| **Type**             | Security                                                   |
| **Estimated Effort** | S                                                          |
| **Source Finding**   | S13 (LOW, CWE-665)                                         |

#### Acceptance Criteria

- [x] Adicionar retry loop com exponential backoff (1s, 2s, 4s, 8s, max 30s)
- [x] Logar quando WMI connection é perdida e reconectada
- [x] 4 threads (uma por WMI event class) têm reconexão independente

---

### S12-003 — Add DSDT-based ERAM address auto-discovery fallback

| Field                | Value                                             |
| -------------------- | ------------------------------------------------- |
| **Ticket ID**        | S12-003                                           |
| **Title**            | Adicionar auto-discovery DSDT para endereços ERAM |
| **Priority**         | P3                                                |
| **Type**             | Security                                          |
| **Estimated Effort** | L                                                 |
| **Source Finding**   | S14 (LOW, CWE-1104)                               |

#### Acceptance Criteria

- [x] Implementar parsing de DSDT para descobrir ERAM base address
- [x] Usar endereços hardcoded como fallback se DSDT parsing falhar
- [x] Logar warning se endereço descoberto difere do hardcoded
- [x] Documentar modelos Xiaomi suportados

---

### S12-004 — Consolidate logging backends to fern only

| Field                | Value                                                                  |
| -------------------- | ---------------------------------------------------------------------- |
| **Ticket ID**        | S12-004                                                                |
| **Title**            | Consolidar logging backends — remover `env_logger`, usar apenas `fern` |
| **Priority**         | P3                                                                     |
| **Type**             | Architecture                                                           |
| **Estimated Effort** | XS                                                                     |
| **Source Finding**   | A6 (LOW)                                                               |

#### Acceptance Criteria

- [x] Remover `env_logger` do `Cargo.toml`
- [x] Usar `fern` para todos modos (dev + production)
- [x] Verificar que log levels funcionam corretamente
- [x] `cargo test` passa

---

### S12-005 — Fix Vec allocation in gesture loop with stack buffer

| Field                | Value                                                         |
| -------------------- | ------------------------------------------------------------- |
| **Ticket ID**        | S12-005                                                       |
| **Title**            | Substituir `vec![0u8; size]` por stack buffer no gesture loop |
| **Priority**         | P3                                                            |
| **Type**             | Performance                                                   |
| **Estimated Effort** | XS                                                            |
| **Source Finding**   | P7 (LOW)                                                      |

#### Acceptance Criteria

- [x] Substituir `let mut buf = vec![0u8; size as usize]` por `let mut buf = [0u8; 4096]`
- [x] Verificar que `size` nunca excede 4096 (logar se exceder)
- [x] Eliminar heap allocation a cada WM_INPUT frame

---

### S12-006 — Consolidate useEffect ref hooks

| Field                | Value                                                     |
| -------------------- | --------------------------------------------------------- |
| **Ticket ID**        | S12-006                                                   |
| **Title**            | Consolidar 6 `useEffect` separados para ref tracking em 1 |
| **Priority**         | P3                                                        |
| **Type**             | Performance                                               |
| **Estimated Effort** | XS                                                        |
| **Source Finding**   | P8 (LOW)                                                  |

#### Acceptance Criteria

- [x] Combinar 6 `useEffect` em `useHardware.ts:555-566` em um único `useEffect`
- [x] Atualizar todos refs em uma chamada

---

### S12-007 — Remove redundant getAudioState calls

| Field                | Value                                                                       |
| -------------------- | --------------------------------------------------------------------------- |
| **Ticket ID**        | S12-007                                                                     |
| **Title**            | Remover `getAudioState()` redundante após `setMasterVolume`/`setMasterMute` |
| **Priority**         | P3                                                                          |
| **Type**             | Performance                                                                 |
| **Estimated Effort** | XS                                                                          |
| **Source Finding**   | P9 (LOW)                                                                    |

#### Acceptance Criteria

- [x] Remover `await getAudioState()` em `useHardware.ts:722,732`
- [x] Confiar no próximo poll batched para atualizar audio state
- [x] Teste: volume slider ainda atualiza visualmente

---

### S12-008 — Reduce TRACE logging in hot path

| Field                | Value                                              |
| -------------------- | -------------------------------------------------- |
| **Ticket ID**        | S12-008                                            |
| **Title**            | Reduzir TRACE logging no hot path de 2s para DEBUG |
| **Priority**         | P3                                                 |
| **Type**             | Performance                                        |
| **Estimated Effort** | XS                                                 |
| **Source Finding**   | P10 (LOW)                                          |

#### Acceptance Criteria

- [x] Mudar `log::trace!` para `log::debug!` em `battery.rs` hot path
- [x] Mesmo para outros módulos no batch poll path
- [x] TRACE continua disponível via `MICONTROL_DEV_TRACE=1`

---

### S12-009 — Add Vite code splitting for tab contents

| Field                | Value                                              |
| -------------------- | -------------------------------------------------- |
| **Ticket ID**        | S12-009                                            |
| **Title**            | Adicionar code splitting no Vite para tab contents |
| **Priority**         | P3                                                 |
| **Type**             | Performance                                        |
| **Estimated Effort** | S                                                  |
| **Source Finding**   | P11 (LOW)                                          |

#### Acceptance Criteria

- [x] Usar `React.lazy()` + `Suspense` para tab contents em `MainWindow.tsx`
- [x] TrayPopup carrega apenas componentes necessários
- [x] Reduzir initial JS payload do TrayPopup em ~40%

---

### S12-010 — Parallelize batch queries with rayon

| Field                | Value                                                |
| -------------------- | ---------------------------------------------------- |
| **Ticket ID**        | S12-010                                              |
| **Title**            | Paralelizar queries sequenciais no batch com `rayon` |
| **Priority**         | P3                                                   |
| **Type**             | Performance                                          |
| **Estimated Effort** | M                                                    |
| **Source Finding**   | P13 (LOW)                                            |

#### Acceptance Criteria

- [x] Usar `rayon::scope` para paralelizar queries independentes no `get_hardware_state_batch`
- [x] Battery (3 WMI queries) e display (IGCL + registry) em paralelo
- [x] Medir latência do batch antes/depois

---

### S12-011 — Add lang attribute to html element

| Field                | Value                                  |
| -------------------- | -------------------------------------- |
| **Ticket ID**        | S12-011                                |
| **Title**            | Adicionar `lang` attribute no `<html>` |
| **Priority**         | P3                                     |
| **Type**             | RAI                                    |
| **Estimated Effort** | XS                                     |
| **Source Finding**   | R24 (LOW)                              |

#### Acceptance Criteria

- [x] Adicionar `lang={lang}` no `<html>` em `App.tsx` ou `index.html`
- [x] Atualizar dinamicamente quando idioma muda

---

### S12-012 — Add option to delete specific analysis results

| Field                | Value                                           |
| -------------------- | ----------------------------------------------- |
| **Ticket ID**        | S12-012                                         |
| **Title**            | Adicionar opção de deletar análises específicas |
| **Priority**         | P3                                              |
| **Type**             | RAI                                             |
| **Estimated Effort** | S                                               |
| **Source Finding**   | R25 (LOW)                                       |

#### Acceptance Criteria

- [x] Adicionar botão "Delete" em cada entry do analysis log
- [x] Remover entry do localStorage e atualizar UI
- [x] Confirmar antes de deletar

---

### S12-013 — Add pluralization support to i18n

| Field                | Value                                     |
| -------------------- | ----------------------------------------- |
| **Ticket ID**        | S12-013                                   |
| **Title**            | Adicionar suporte a pluralization no i18n |
| **Priority**         | P3                                        |
| **Type**             | UX                                        |
| **Estimated Effort** | M                                         |
| **Source Finding**   | U23 (LOW)                                 |

#### Acceptance Criteria

- [x] Adicionar ICU MessageFormat ou implementar pluralization simples
- [x] Suportar `{count, plural, one {1 item} other {# items}}`
- [x] Atualizar strings existentes que precisam de pluralization
- [x] Atualizar traduções

---

### S12-014 — Add hover states to volume mute button and toggle switch

| Field                | Value                                                        |
| -------------------- | ------------------------------------------------------------ |
| **Ticket ID**        | S12-014                                                      |
| **Title**            | Adicionar hover states em volume mute button e toggle switch |
| **Priority**         | P3                                                           |
| **Type**             | UX                                                           |
| **Estimated Effort** | XS                                                           |
| **Source Finding**   | U25/U26 (LOW)                                                |

#### Acceptance Criteria

- [x] Adicionar `:hover` no mute button em `AudioControl.tsx`
- [x] Adicionar `:hover` no `.toggle-switch` label em `globals.css`
- [x] Adicionar `:active` states para feedback tátil

---

### S12-015 — Pin GitHub Actions to commit SHAs

| Field                | Value                                               |
| -------------------- | --------------------------------------------------- |
| **Ticket ID**        | S12-015                                             |
| **Title**            | Pinar `softprops/action-gh-release` para commit SHA |
| **Priority**         | P3                                                  |
| **Type**             | DevOps                                              |
| **Estimated Effort** | XS                                                  |
| **Source Finding**   | D17 (LOW)                                           |

#### Acceptance Criteria

- [x] Pinar `softprops/action-gh-release@<sha>` em vez de `@v2`
- [x] Mesmo para `actions/checkout`, `actions/cache`, etc.
- [x] Documentar processo de atualização de SHAs

---

### S12-016 — Add .cargo/config.toml for target-cpu optimization

| Field                | Value                                          |
| -------------------- | ---------------------------------------------- |
| **Ticket ID**        | S12-016                                        |
| **Title**            | Adicionar `.cargo/config.toml` para target-cpu |
| **Priority**         | P3                                             |
| **Type**             | DevOps                                         |
| **Estimated Effort** | XS                                             |
| **Source Finding**   | D19 (LOW)                                      |

#### Acceptance Criteria

- [x] Criar `.cargo/config.toml` com `[build]` settings
- [x] Considerar `target-cpu=x86-64` para release builds
- [x] Não afetar dev builds

---

### S12-017 — Restrict bundle.targets if not signing MSIX/AppX

| Field                | Value                                                  |
| -------------------- | ------------------------------------------------------ |
| **Ticket ID**        | S12-017                                                |
| **Title**            | Restringir `bundle.targets` se não assinando MSIX/AppX |
| **Priority**         | P3                                                     |
| **Type**             | DevOps                                                 |
| **Estimated Effort** | XS                                                     |
| **Source Finding**   | D20 (LOW)                                              |

#### Acceptance Criteria

- [x] Se apenas NSIS é assinado, mudar `bundle.targets` de `"all"` para `["nsis"]`
- [x] Ou manter `"all"` mas documentar que MSIX/AppX não são assinados

---

### S12-018 — Add CONTRIBUTING.md and architecture overview

| Field                | Value                                               |
| -------------------- | --------------------------------------------------- |
| **Ticket ID**        | S12-018                                             |
| **Title**            | Adicionar `CONTRIBUTING.md` e architecture overview |
| **Priority**         | P3                                                  |
| **Type**             | Documentation                                       |
| **Estimated Effort** | M                                                   |
| **Source Finding**   | C18/C19 (LOW)                                       |

#### Acceptance Criteria

- [x] Criar `CONTRIBUTING.md` com: setup, build, test, commit conventions, PR process
- [x] Criar `docs/architecture.md` com C4 diagram (Context + Container)
- [x] Documentar HAL extension pattern (6-step checklist)
- [x] Adicionar badges de CI status no README

---

### S12-019 — Document HAL extension pattern

| Field                | Value                                           |
| -------------------- | ----------------------------------------------- |
| **Ticket ID**        | S12-019                                         |
| **Title**            | Documentar padrão de extensão de hardware (HAL) |
| **Priority**         | P3                                              |
| **Type**             | Architecture / Documentation                    |
| **Estimated Effort** | S                                               |
| **Source Finding**   | A4 (MEDIUM→LOW after docs)                      |

#### Acceptance Criteria

- [x] Criar `docs/adding-a-hardware-feature.md` com 6-step checklist
- [x] Documentar: criar `hw/new_mod.rs`, criar commands, registrar em `lib.rs`, adicionar em `elevated.rs`, criar componente React, adicionar i18n
- [x] Adicionar template de arquivo

---

### S12-020 — Add rollback procedure to release docs

| Field                | Value                                                   |
| -------------------- | ------------------------------------------------------- |
| **Ticket ID**        | S12-020                                                 |
| **Title**            | Adicionar procedimento de rollback em `docs/release.md` |
| **Priority**         | P3                                                      |
| **Type**             | DevOps / Documentation                                  |
| **Estimated Effort** | XS                                                      |
| **Source Finding**   | D16 (MEDIUM→LOW)                                        |

#### Acceptance Criteria

- [x] Documentar: como reverter um release (delete GitHub Release, revert tag, etc.)
- [x] Documentar: como reverter updater manifest (`latest.json`)
- [x] Documentar: processo de hotfix release

---

## Sprint Exit Criteria

- [x] Todos os 20 tickets completos
- [x] `cargo test` + `cargo clippy` (0 warnings) + `npm run build` + `npm run lint` (0 warnings) passam
- [x] `CONTRIBUTING.md` + `docs/architecture.md` criados
- [x] Relatório de estabilidade: 0 findings LOW restantes
- [x] Aplicação pronta para GA com qualidade excelente

---

## Resumo Consolidado de Sprints (S9-S12)

| Sprint                    | Prioridade    | Tickets        | Esforço Total  | Domínios                                                 |
| ------------------------- | ------------- | -------------- | -------------- | -------------------------------------------------------- |
| **S9 — GA Blockers**      | P0 (CRITICAL) | 14             | ~7.5 dias      | Security, RAI, DevOps, UX, Code Quality                  |
| **S10 — High Priority**   | P1 (HIGH)     | 22             | ~12.5 dias     | Security, Performance, DevOps, RAI, UX, Code Quality     |
| **S11 — Medium Priority** | P2 (MEDIUM)   | 33             | ~15 dias       | Architecture, Performance, RAI, UX, Code Quality, DevOps |
| **S12 — Low Priority**    | P3 (LOW)      | 20             | ~7.5 dias      | Security, Performance, RAI, UX, DevOps, Documentation    |
| **TOTAL**                 | —             | **89 tickets** | **~42.5 dias** | Todos os 7 domínios                                      |

### Ordem de Execução Recomendada

```
Sprint 9 (GA Blockers)     → ~1.5 semanas  → Desbloqueia GA
    ↓
Sprint 10 (High Priority)  → ~2.5 semanas  → Qualidade pós-GA
    ↓
Sprint 11 (Medium Priority) → ~3 semanas    → Dívida técnica
    ↓
Sprint 12 (Low Priority)   → ~1.5 semanas  → Polish final
```

### Mapeamento Findings → Tickets

| Finding                     | Sprint | Ticket  |
| --------------------------- | ------ | ------- |
| S1 (HMAC race)              | S9     | S9-001  |
| R1/U1 (dark pattern)        | S9     | S9-002  |
| R2 (consent bypass)         | S9     | S9-003  |
| R5 (storageNote)            | S9     | S9-004  |
| R3/R4 (focus trap/visible)  | S9     | S9-005  |
| U2 (silent failures)        | S9     | S9-006  |
| D1 (latest.json)            | S9     | S9-007  |
| D2 (signing key)            | S9     | S9-008  |
| C1 (REMAP_STATE unwrap)     | S9     | S9-009  |
| C3 (Raw Input buffer)       | S9     | S9-010  |
| C4 (unsafe comments)        | S9     | S9-011  |
| C2 (mem::zeroed)            | S9     | S9-012  |
| R8/U20 (Escape key)         | S9     | S9-013  |
| C13 (Error Boundary)        | S9     | S9-014  |
| S2 (ACL)                    | S10    | S10-001 |
| S3 (elevation bypass)       | S10    | S10-002 |
| S4/S6 (unsafe pointers)     | S10    | S10-003 |
| S5 (CSP)                    | S10    | S10-004 |
| P1/P2 (WMI cache bypass)    | S10    | S10-005 |
| D4 (cargo/npm audit)        | S10    | S10-006 |
| D3 (Authenticode)           | S10    | S10-007 |
| D5 (pre-commit)             | S10    | S10-008 |
| D6 (crash reporting)        | S10    | S10-009 |
| D7 (branch protection)      | S10    | S10-010 |
| D8 (tauri-smoke)            | S10    | S10-011 |
| R6/R7 (data deletion)       | S10    | S10-012 |
| R9/U6/R26 (aria-labels)     | S10    | S10-013 |
| R13 (role=alert)            | S10    | S10-014 |
| R11 (high-contrast)         | S10    | S10-015 |
| R12/U17 (min-width)         | S10    | S10-016 |
| U3/U9 (sidebar)             | S10    | S10-017 |
| U4/U14 (loading)            | S10    | S10-018 |
| U7 (hardcoded strings)      | S10    | S10-019 |
| U8 (RTL)                    | S10    | S10-020 |
| C5/C6/C7 (unwrap)           | S10    | S10-021 |
| C8 (packed structs)         | S10    | S10-022 |
| A1 (error types)            | S11    | S11-001 |
| A5 (tiered polling)         | S11    | S11-002 |
| P4 (useMemo split)          | S11    | S11-003 |
| P5 (HID handle)             | S11    | S11-004 |
| P6 (localStorage)           | S11    | S11-005 |
| S7 (ECRAM validation)       | S11    | S11-006 |
| S8 (IoT pipe)               | S11    | S11-007 |
| S9 (API key backend)        | S11    | S11-008 |
| S10 (telemetry consent)     | S11    | S11-009 |
| S11 (task elevation)        | S11    | S11-010 |
| A2 (global state)           | S11    | S11-011 |
| A3 (ErrorResponse)          | S11    | S11-012 |
| A8 (retry)                  | S11    | S11-013 |
| R15 (audit log)             | S11    | S11-014 |
| R16 (policy versioning)     | S11    | S11-015 |
| R17/R18 (labels/landmarks)  | S11    | S11-016 |
| R19 (rem fonts)             | S11    | S11-017 |
| R20 (reduced-motion)        | S11    | S11-018 |
| R22 (custom endpoint)       | S11    | S11-019 |
| C11 (registry helper)       | S11    | S11-020 |
| C12 (osd tests)             | S11    | S11-021 |
| C14 (integration tests)     | S11    | S11-022 |
| C15 (log rotation)          | S11    | S11-023 |
| C10/D9 (clippy)             | S11    | S11-024 |
| D10 (lint rules)            | S11    | S11-025 |
| D12 (semver)                | S11    | S11-026 |
| D13 (dependabot)            | S11    | S11-027 |
| D15 (code coverage)         | S11    | S11-028 |
| U18 (translations)          | S11    | S11-029 |
| U12/U13 (error pattern)     | S11    | S11-030 |
| U11/U22 (refactor)          | S11    | S11-031 |
| U19/U26 (spinner/animation) | S11    | S11-032 |
| C9 (dead code)              | S11    | S11-033 |
| S12 (modifier release)      | S12    | S12-001 |
| S13 (WMI reconnection)      | S12    | S12-002 |
| S14 (DSDT auto-discovery)   | S12    | S12-003 |
| A6 (logging backends)       | S12    | S12-004 |
| P7 (stack buffer)           | S12    | S12-005 |
| P8 (useEffect)              | S12    | S12-006 |
| P9 (getAudioState)          | S12    | S12-007 |
| P10 (TRACE logging)         | S12    | S12-008 |
| P11 (code splitting)        | S12    | S12-009 |
| P13 (rayon)                 | S12    | S12-010 |
| R24 (lang attr)             | S12    | S12-011 |
| R25 (delete analysis)       | S12    | S12-012 |
| U23 (pluralization)         | S12    | S12-013 |
| U25/U26 (hover states)      | S12    | S12-014 |
| D17 (pin actions)           | S12    | S12-015 |
| D19 (.cargo/config)         | S12    | S12-016 |
| D20 (bundle targets)        | S12    | S12-017 |
| C18/C19 (CONTRIBUTING)      | S12    | S12-018 |
| A4 (HAL docs)               | S12    | S12-019 |
| D16 (rollback docs)         | S12    | S12-020 |
