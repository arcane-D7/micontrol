# Sprint 11 — Medium Priority: Architecture, Performance & Quality

## Sprint Metadata

| Field                 | Value                                                                                                                                   |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| **Sprint Name**       | Medium Priority — Architecture, Performance & Quality                                                                                   |
| **Sprint Goal**       | Endereçar todos os 40+ findings MEDIUM do relatório de estabilidade, focando em arquitetura, performance, code quality e UX refinements |
| **Duration Estimate** | 3 semanas (15 dias úteis)                                                                                                               |
| **Priority**          | P2 — Melhorias importantes de qualidade e dívida técnica                                                                                |
| **Sprint Type**       | Multi-domain                                                                                                                            |
| **Primary Owner**     | Full-stack engineer                                                                                                                     |
| **Secondary Owner**   | Architecture reviewer                                                                                                                   |
| **Source**            | `docs/stability-report-2026-06-24.md`                                                                                                   |
| **Depends On**        | Sprint 9, Sprint 10                                                                                                                     |

## Sprint Goal Statement

Com os blockers CRITICAL e HIGH resolvidos, este sprint endereça a dívida técnica MEDIUM: migração de error types, polling em tiers, otimizações de React, DevOps refinements, acessibilidade adicional, code quality (dead code, clippy, registry helper), e testes faltantes. Estas melhorias elevam a aplicação a um padrão de qualidade superior.

---

## Tickets

### S11-001 — Migrate hw/ modules from anyhow to HardwareResult<T>

| Field                | Value                                                             |
| -------------------- | ----------------------------------------------------------------- |
| **Ticket ID**        | S11-001                                                           |
| **Title**            | Migrar módulos `hw/` de `anyhow::Result` para `HardwareResult<T>` |
| **Priority**         | P2                                                                |
| **Type**             | Architecture                                                      |
| **Estimated Effort** | XL (3-5 dias)                                                     |
| **Source Finding**   | A1 (HIGH→MEDIUM after Sprint 10)                                  |

#### Description

`HardwareError` enum existe em `hw/errors.rs` com 15 typed variants, mas apenas `wifi.rs` usa. 14/16 módulos usam `anyhow::Result`, perdendo informação de erro tipada.

#### Acceptance Criteria

- [x] Migrar cada módulo `hw/*.rs` de `anyhow::Result` para `HardwareResult<T>`
- [x] Adicionar conversões `From` para erros de WMI, registry, HID, etc.
- [x] Commands layer converte `HardwareError` para `ErrorResponse` JSON
- [x] Frontend recebe `code` + `message` tipados em vez de strings opacas
- [x] `cargo test` passa com testes atualizados

---

### S11-002 — Implement tiered polling (fast 2s + slow 15-30s)

| Field                | Value                                                   |
| -------------------- | ------------------------------------------------------- |
| **Ticket ID**        | S11-002                                                 |
| **Title**            | Implementar polling em tiers: fast (2s) + slow (15-30s) |
| **Priority**         | P2                                                      |
| **Type**             | Performance / Architecture                              |
| **Estimated Effort** | M                                                       |
| **Source Finding**   | A5 (LOW-MEDIUM)                                         |

#### Acceptance Criteria

- [x] Fast tier (2s): fan speed, CPU temp, CPU usage, GPU temp
- [x] Slow tier (15s): battery info, display info, touchpad info
- [x] Poll-once: system info (model, CPU name, RAM)
- [x] `useHardware.ts` implementa dois intervals independentes
- [x] Reduz WMI churn para dados estáticos

---

### S11-003 — Split useMemo into logical groups

| Field                | Value                                                     |
| -------------------- | --------------------------------------------------------- |
| **Ticket ID**        | S11-003                                                   |
| **Title**            | Split `useMemo` em grupos lógicos para reduzir re-renders |
| **Priority**         | P2                                                        |
| **Type**             | Performance                                               |
| **Estimated Effort** | L                                                         |
| **Source Finding**   | P4 (MEDIUM)                                               |

#### Acceptance Criteria

- [x] Dividir return de `useHardware()` em: `batteryState`, `displayState`, `audioState`, `fanState`, `touchpadState`, `systemState`
- [x] Ou migrar para `useSyncExternalStore` com subscriptions granulares
- [x] Componentes que usam apenas `touchpadState` não re-renderizam quando `batteryState` muda
- [x] Teste de performance: re-renders reduzidos em 50%+

---

### S11-004 — Cache HID device handle for haptics writes

| Field                | Value                                                            |
| -------------------- | ---------------------------------------------------------------- |
| **Ticket ID**        | S11-004                                                          |
| **Title**            | Manter handle HID aberto com RAII em vez de reabrir a cada write |
| **Priority**         | P2                                                               |
| **Type**             | Performance                                                      |
| **Estimated Effort** | M                                                                |
| **Source Finding**   | P5 (MEDIUM)                                                      |

#### Acceptance Criteria

- [x] Manter `CreateFileW` handle aberto para o lifetime da gesture thread
- [x] Usar `Rc<HANDLE>` com RAII cleanup (Drop fecha handle)
- [x] Reabrir apenas se device for removido (verificar erro `HidD_SetFeature`)
- [x] Medir latência de haptics write antes/depois

---

### S11-005 — Optimize localStorage analysis logs

| Field                | Value                                                                  |
| -------------------- | ---------------------------------------------------------------------- |
| **Ticket ID**        | S11-005                                                                |
| **Title**            | Otimizar analysis logs: trim para 100 entries ou migrar para IndexedDB |
| **Priority**         | P2                                                                     |
| **Type**             | Performance                                                            |
| **Estimated Effort** | M                                                                      |
| **Source Finding**   | P6 (MEDIUM)                                                            |

#### Acceptance Criteria

- [x] Reduzir max entries de 500 para 100
- [x] Ou migrar para IndexedDB com write assíncrono
- [x] Debounce writes (não escrever a cada poll)
- [x] Medir tempo de JSON.parse antes/depois

---

### S11-006 — Add ECRAM address range validation

| Field                | Value                                                    |
| -------------------- | -------------------------------------------------------- |
| **Ticket ID**        | S11-006                                                  |
| **Title**            | Adicionar validação de range de endereço em `read_ecram` |
| **Priority**         | P2                                                       |
| **Type**             | Security                                                 |
| **Estimated Effort** | S                                                        |
| **Source Finding**   | S7 (MEDIUM, CWE-20)                                      |

#### Acceptance Criteria

- [x] `read_ecram` valida que `phys_addr` está dentro de regiões conhecidas (ERAM, SMA2, IoT)
- [x] Fail-closed se endereço está fora das regiões conhecidas
- [x] Adicionar override explícito para raw reads (já existe via env var)
- [x] Teste: endereço arbitrário deve ser rejeitado

---

### S11-007 — Fix IoT pipe polling with overlapped I/O

| Field                | Value                                                         |
| -------------------- | ------------------------------------------------------------- |
| **Ticket ID**        | S11-007                                                       |
| **Title**            | Substituir busy-wait polling por `OVERLAPPED` I/O no IoT pipe |
| **Priority**         | P2                                                            |
| **Type**             | Security / Performance                                        |
| **Estimated Effort** | M                                                             |
| **Source Finding**   | S8 (MEDIUM, CWE-835)                                          |

#### Acceptance Criteria

- [x] Substituir loop de 10ms sleeps por `ReadFile` com `OVERLAPPED`
- [x] Usar `WaitForSingleObject` no evento overlapped
- [x] Eliminar 500 iterações de polling em 5s
- [x] Teste: comunicação IoT funciona corretamente

---

### S11-008 — Route API key through backend Tauri command

| Field                | Value                                                |
| -------------------- | ---------------------------------------------------- |
| **Ticket ID**        | S11-008                                              |
| **Title**            | Roteirizar chamadas OpenAI via backend Tauri command |
| **Priority**         | P2                                                   |
| **Type**             | Security / RAI                                       |
| **Estimated Effort** | L                                                    |
| **Source Finding**   | S9 (MEDIUM, CWE-312)                                 |

#### Acceptance Criteria

- [x] Criar Tauri command `analyze_system` no backend que lê API key do keyring
- [x] Frontend não carrega API key em JS memory
- [x] Backend faz chamada HTTP para OpenAI e retorna resultado
- [x] Consent check no backend antes de fazer a chamada

---

### S11-009 — Fix telemetry collection before consent check

| Field                | Value                                                   |
| -------------------- | ------------------------------------------------------- |
| **Ticket ID**        | S11-009                                                 |
| **Title**            | Verificar consent antes de qualquer coleta de telemetry |
| **Priority**         | P2                                                      |
| **Type**             | RAI                                                     |
| **Estimated Effort** | S                                                       |
| **Source Finding**   | S10 (MEDIUM, CWE-276)                                   |

#### Acceptance Criteria

- [x] Mover consent check para antes de `buildPrompt()` em `analyzeSystem` e `analyzeWithLogs`
- [x] Se consent revogado, limpar logs em memória e disco
- [x] Não coletar system context antes do consent

---

### S11-010 — Verify scheduled task elevation at runtime

| Field                | Value                                           |
| -------------------- | ----------------------------------------------- |
| **Ticket ID**        | S11-010                                         |
| **Title**            | Verificar RunLevel do scheduled task em runtime |
| **Priority**         | P2                                              |
| **Type**             | Security                                        |
| **Estimated Effort** | S                                               |
| **Source Finding**   | S11 (MEDIUM, CWE-754)                           |

#### Acceptance Criteria

- [x] Logar RunLevel configurado do task no startup
- [x] Adicionar self-test ping que verifica se o task está genuinely elevated
- [x] Se RunLevel não for Highest, logar erro claro

---

### S11-011 — Consolidate global state into Tauri AppState

| Field                | Value                                                  |
| -------------------- | ------------------------------------------------------ |
| **Ticket ID**        | S11-011                                                |
| **Title**            | Consolidar estado global fragmentado em Tauri AppState |
| **Priority**         | P2                                                     |
| **Type**             | Architecture                                           |
| **Estimated Effort** | L                                                      |
| **Source Finding**   | A2 (MEDIUM)                                            |

#### Acceptance Criteria

- [x] Mover `OnceLock<RwLock<HardwareProfile>>` de `discovery.rs` para `AppState`
- [x] Documentar ordem de inicialização em `lib.rs::run()`
- [x] `global_profile()` não retorna `None` após `init()`
- [x] Reduzir uso de `AtomicBool` statics onde possível

---

### S11-012 — Use ErrorResponse serialization at IPC boundary

| Field                | Value                                                 |
| -------------------- | ----------------------------------------------------- |
| **Ticket ID**        | S11-012                                               |
| **Title**            | Usar `ErrorResponse` serialization no boundary de IPC |
| **Priority**         | P2                                                    |
| **Type**             | Architecture                                          |
| **Estimated Effort** | M                                                     |
| **Source Finding**   | A3 (MEDIUM)                                           |

#### Acceptance Criteria

- [x] Após S11-001, substituir `.to_string()` por `ErrorResponse` JSON em todos commands
- [x] Frontend recebe `{ code: "...", message: "..." }` em vez de string opaca
- [x] Atualizar error handling no frontend para usar `code` para i18n

---

### S11-013 — Add retry mechanism for flaky operations

| Field                | Value                                                 |
| -------------------- | ----------------------------------------------------- |
| **Ticket ID**        | S11-013                                               |
| **Title**            | Adicionar retry para operações flaky (WMI, pipe, HID) |
| **Priority**         | P2                                                    |
| **Type**             | Architecture                                          |
| **Estimated Effort** | M                                                     |
| **Source Finding**   | A8 (LOW)                                              |

#### Acceptance Criteria

- [x] Adicionar 1 retry com 100ms delay para WMI queries
- [x] Adicionar 1 retry para named pipe reads
- [x] Adicionar 1 retry para HID writes
- [x] Logar quando retry é usado

---

### S11-014 — Add audit log for consent grant/revoke

| Field                | Value                                                    |
| -------------------- | -------------------------------------------------------- |
| **Ticket ID**        | S11-014                                                  |
| **Title**            | Adicionar audit log para eventos de consent grant/revoke |
| **Priority**         | P2                                                       |
| **Type**             | RAI                                                      |
| **Estimated Effort** | S                                                        |
| **Source Finding**   | R15 (MEDIUM, GDPR Art.30)                                |

#### Acceptance Criteria

- [x] Registrar evento quando consent é concedido (timestamp, policy version)
- [x] Registrar evento quando consent é revogado
- [x] Armazenar em arquivo de audit log separado
- [x] UI para visualizar histórico de consent

---

### S11-015 — Implement policy versioning and re-prompt on change

| Field                | Value                                                   |
| -------------------- | ------------------------------------------------------- |
| **Ticket ID**        | S11-015                                                 |
| **Title**            | Versionar privacy policy e re-prompt consent em mudança |
| **Priority**         | P2                                                      |
| **Type**             | RAI                                                     |
| **Estimated Effort** | M                                                       |
| **Source Finding**   | R16 (MEDIUM)                                            |

#### Acceptance Criteria

- [x] Definir `POLICY_VERSION` como constante (atualizar para 2)
- [x] Ao iniciar, comparar policy version do consent com a versão atual
- [x] Se diferente, re-exibir ConsentDialog
- [x] Documentar processo de bump de policy version

---

### S11-016 — Add label htmlFor associations and landmark regions

| Field                | Value                                                     |
| -------------------- | --------------------------------------------------------- |
| **Ticket ID**        | S11-016                                                   |
| **Title**            | Adicionar `<label htmlFor>` e landmark regions semânticas |
| **Priority**         | P2                                                        |
| **Type**             | RAI / UX                                                  |
| **Estimated Effort** | M                                                         |
| **Source Finding**   | R17/R18 (MEDIUM, WCAG)                                    |

#### Acceptance Criteria

- [x] Associar todos `<label>` com `htmlFor` aos respectivos inputs
- [x] Adicionar `<nav>`, `<main>`, `<aside>` onde apropriado
- [x] Adicionar `role="banner"` no header
- [x] Teste com screen reader: navegação por landmarks funciona

---

### S11-017 — Migrate font sizes from px to rem

| Field                | Value                                     |
| -------------------- | ----------------------------------------- |
| **Ticket ID**        | S11-017                                   |
| **Title**            | Migrar font sizes de px absoluto para rem |
| **Priority**         | P2                                        |
| **Type**             | RAI / UX                                  |
| **Estimated Effort** | M                                         |
| **Source Finding**   | R19 (MEDIUM)                              |

#### Acceptance Criteria

- [x] Definir type scale em `globals.css` com `rem` (0.75rem, 0.875rem, 1rem, 1.25rem, 1.5rem)
- [x] Substituir todos `fontSize: 13`, `fontSize: 12` etc. por classes CSS
- [x] Respeitar zoom do usuário no Windows
- [x] Harmonizar tamanhos inconsistentes (10px-30px → type scale)

---

### S11-018 — Add prefers-reduced-motion media query

| Field                | Value                                          |
| -------------------- | ---------------------------------------------- |
| **Ticket ID**        | S11-018                                        |
| **Title**            | Adicionar `prefers-reduced-motion` media query |
| **Priority**         | P2                                             |
| **Type**             | RAI / UX                                       |
| **Estimated Effort** | XS                                             |
| **Source Finding**   | R20 (MEDIUM)                                   |

#### Acceptance Criteria

- [x] Adicionar `@media (prefers-reduced-motion: reduce)` em `globals.css`
- [x] Desabilitar `card-rise`, `tab-in`, fade animations
- [x] `transition: none` para usuários com motion sensitivity

---

### S11-019 — Add custom API endpoint privacy disclosure

| Field                | Value                                                    |
| -------------------- | -------------------------------------------------------- |
| **Ticket ID**        | S11-019                                                  |
| **Title**            | Adicionar aviso de privacidade para custom API endpoints |
| **Priority**         | P2                                                       |
| **Type**             | RAI                                                      |
| **Estimated Effort** | XS                                                       |
| **Source Finding**   | R22 (MEDIUM)                                             |

#### Acceptance Criteria

- [x] Adicionar warning em `SettingsPage.tsx:162` quando custom endpoint é configurado
- [x] Texto: "Data sent to non-OpenAI endpoints is not covered by OpenAI's privacy policy"
- [x] Adicionar i18n keys para a mensagem

---

### S11-020 — Extract util/registry.rs helper

| Field                | Value                                                                  |
| -------------------- | ---------------------------------------------------------------------- |
| **Ticket ID**        | S11-020                                                                |
| **Title**            | Extrair `util/registry.rs` helper para eliminar 200+ linhas duplicadas |
| **Priority**         | P2                                                                     |
| **Type**             | Code Quality                                                           |
| **Estimated Effort** | M                                                                      |
| **Source Finding**   | C11 (MEDIUM)                                                           |

#### Acceptance Criteria

- [x] Criar `util/registry.rs` com `read_reg_dword`, `write_reg_dword`, `open_reg_key`
- [x] Substituir 7+ implementações duplicadas em `charging.rs`, `performance.rs`, `display.rs`, `fan.rs`, `touchpad.rs`, `startup.rs`, `battery.rs`
- [x] Eliminar ~200 linhas de código duplicado
- [x] `cargo test` passa

---

### S11-021 — Add tests for hw/osd.rs

| Field                | Value                                                        |
| -------------------- | ------------------------------------------------------------ |
| **Ticket ID**        | S11-021                                                      |
| **Title**            | Adicionar testes para `hw/osd.rs` (700+ linhas, zero testes) |
| **Priority**         | P2                                                           |
| **Type**             | Code Quality                                                 |
| **Estimated Effort** | L                                                            |
| **Source Finding**   | C12 (MEDIUM)                                                 |

#### Acceptance Criteria

- [x] Testes unitários para OSD state machine
- [x] Testes para OSD positioning logic
- [x] Testes para fade transition timing
- [x] Mock GDI calls onde necessário
- [x] Cobertura mínima de 50% para `osd.rs`

---

### S11-022 — Add integration tests for elevated bridge

| Field                | Value                                                            |
| -------------------- | ---------------------------------------------------------------- |
| **Ticket ID**        | S11-022                                                          |
| **Title**            | Adicionar testes de integração para o elevated bridge round-trip |
| **Priority**         | P2                                                               |
| **Type**             | Code Quality                                                     |
| **Estimated Effort** | L                                                                |
| **Source Finding**   | C14 (MEDIUM)                                                     |

#### Acceptance Criteria

- [x] Teste round-trip: command → HMAC sign → write → read → verify → execute → response
- [x] Teste de rejeição: comando sem HMAC é rejeitado
- [x] Teste de replay: nonce reusado é rejeitado
- [x] Teste de timestamp: comando expirado é rejeitado

---

### S11-023 — Add log rotation for dev trace logs

| Field                | Value                                             |
| -------------------- | ------------------------------------------------- |
| **Ticket ID**        | S11-023                                           |
| **Title**            | Adicionar log rotation para `tauri-dev-trace.log` |
| **Priority**         | P2                                                |
| **Type**             | Code Quality                                      |
| **Estimated Effort** | S                                                 |
| **Source Finding**   | C15 (MEDIUM)                                      |

#### Acceptance Criteria

- [x] Configurar `fern` com rotation (max 5MB por arquivo, 3 arquivos)
- [x] Ou usar `log-panics` + `tracing-appender` para rotation
- [x] Logs antigos são automaticamente rotacionados

---

### S11-024 — Fix clippy warnings and set warning budget

| Field                | Value                                                 |
| -------------------- | ----------------------------------------------------- |
| **Ticket ID**        | S11-024                                               |
| **Title**            | Corrigir 50+ clippy warnings e definir warning budget |
| **Priority**         | P2                                                    |
| **Type**             | Code Quality / DevOps                                 |
| **Estimated Effort** | L                                                     |
| **Source Finding**   | C10/D9 (MEDIUM)                                       |

#### Acceptance Criteria

- [x] Corrigir 5 warnings de correctness (needless_lifetimes, unit_arg, etc.)
- [x] Corrigir 15 warnings de performance (large_enum_variant, clone_on_copy, etc.)
- [x] Definir `clippy.toml` com thresholds reais
- [x] Remover `continue-on-error: true` do clippy no CI
- [x] Set warning budget: 0 warnings permitidos

---

### S11-025 — Upgrade ESLint rules from warn to error

| Field                | Value                                    |
| -------------------- | ---------------------------------------- |
| **Ticket ID**        | S11-025                                  |
| **Title**            | Migrar lint rules de `warn` para `error` |
| **Priority**         | P2                                       |
| **Type**             | DevOps / Code Quality                    |
| **Estimated Effort** | S                                        |
| **Source Finding**   | D10 (MEDIUM)                             |

#### Acceptance Criteria

- [x] `react-hooks/rules-of-hooks`: `error`
- [x] `react-hooks/exhaustive-deps`: `error`
- [x] `@typescript-eslint/no-floating-promises`: `error`
- [x] Corrigir todas violações existentes antes de mudar para `error`
- [x] `npm run lint` passa sem warnings

---

### S11-026 — Add semver validation to sync-version.cjs

| Field                | Value                                            |
| -------------------- | ------------------------------------------------ |
| **Ticket ID**        | S11-026                                          |
| **Title**            | Adicionar validação semver em `sync-version.cjs` |
| **Priority**         | P2                                               |
| **Type**             | DevOps                                           |
| **Estimated Effort** | XS                                               |
| **Source Finding**   | D12 (MEDIUM)                                     |

#### Acceptance Criteria

- [x] Adicionar regex `/^\d+\.\d+\.\d+/` antes de escrever versão
- [x] Rejeitar versões inválidas com erro claro
- [x] Corrigir filename mismatch nos comments (`.js` → `.cjs`)

---

### S11-027 — Add dependabot/Renovate configuration

| Field                | Value                                            |
| -------------------- | ------------------------------------------------ |
| **Ticket ID**        | S11-027                                          |
| **Title**            | Adicionar configuração de dependabot ou Renovate |
| **Priority**         | P2                                               |
| **Type**             | DevOps                                           |
| **Estimated Effort** | XS                                               |
| **Source Finding**   | D13 (MEDIUM)                                     |

#### Acceptance Criteria

- [x] Criar `.github/dependabot.yml` com configs para npm e cargo
- [x] Schedule semanal para updates
- [x] Agrupar minor updates
- [x] Auto-assign reviewer

---

### S11-028 — Add code coverage reporting

| Field                | Value                                   |
| -------------------- | --------------------------------------- |
| **Ticket ID**        | S11-028                                 |
| **Title**            | Adicionar code coverage reporting em CI |
| **Priority**         | P2                                      |
| **Type**             | DevOps                                  |
| **Estimated Effort** | M                                       |
| **Source Finding**   | D15 (MEDIUM)                            |

#### Acceptance Criteria

- [x] Adicionar `cargo-tarpaulin` no CI para Rust coverage
- [x] Adicionar `vitest --coverage` para frontend coverage
- [x] Uploadar relatório para Codecov ou similar
- [x] Adicionar badge de coverage no README

---

### S11-029 — Complete missing translations (fr, es, pt)

| Field                | Value                                       |
| -------------------- | ------------------------------------------- |
| **Ticket ID**        | S11-029                                     |
| **Title**            | Completar traduções faltantes em fr, es, pt |
| **Priority**         | P2                                          |
| **Type**             | UX                                          |
| **Estimated Effort** | M                                           |
| **Source Finding**   | U18 (MEDIUM)                                |

#### Acceptance Criteria

- [x] Comparar todas as keys em `en.json` com `fr.json`, `es.json`, `pt.json`
- [x] Adicionar keys faltantes (fr notably lacks `nav.iot`, `nav.wifi`, etc.)
- [x] Revisar traduções existentes para precisão
- [x] Adicionar script de validação de completude de i18n

---

### S11-030 — Standardize error handling pattern and add retry buttons

| Field                | Value                                                 |
| -------------------- | ----------------------------------------------------- |
| **Ticket ID**        | S11-030                                               |
| **Title**            | Padronizar error handling e adicionar botões de Retry |
| **Priority**         | P2                                                    |
| **Type**             | UX                                                    |
| **Estimated Effort** | M                                                     |
| **Source Finding**   | U12/U13 (MEDIUM)                                      |

#### Acceptance Criteria

- [x] Criar padrão unificado: `addToast` com mensagem i18n + botão "Retry"
- [x] Substituir raw error strings por mensagens amigáveis
- [x] Adicionar botão Retry em operações falhadas (volume, brightness, hardware commands)
- [x] Manter detalhe técnico em canal secundário (console ou expandable)

---

### S11-031 — Refactor SettingsPage and extract KeyBindingRow

| Field                | Value                                                                  |
| -------------------- | ---------------------------------------------------------------------- |
| **Ticket ID**        | S11-031                                                                |
| **Title**            | Refatorar `SettingsPage.tsx` (god component) e extrair `KeyBindingRow` |
| **Priority**         | P2                                                                     |
| **Type**             | UX / Code Quality                                                      |
| **Estimated Effort** | L                                                                      |
| **Source Finding**   | U11/U22 (MEDIUM/LOW)                                                   |

#### Acceptance Criteria

- [x] Extrair `KeyBindingRow` (~300 linhas) para `src/components/KeyBindingRow.tsx`
- [x] Extrair AI config form para `src/components/AiConfigForm.tsx`
- [x] Extrair privacy consent section para `src/components/PrivacyConsentSection.tsx`
- [x] `SettingsPage.tsx` fica com <200 linhas

---

### S11-032 — Add CSS spinner animations and consent dialog entrance

| Field                | Value                                                                     |
| -------------------- | ------------------------------------------------------------------------- |
| **Ticket ID**        | S11-032                                                                   |
| **Title**            | Adicionar CSS spinner animations e animation de entrada no consent dialog |
| **Priority**         | P2                                                                        |
| **Type**             | UX                                                                        |
| **Estimated Effort** | S                                                                         |
| **Source Finding**   | U19/U26 (MEDIUM/LOW)                                                      |

#### Acceptance Criteria

- [x] Adicionar `@keyframes spinner` em `globals.css`
- [x] Criar classe `.spinner` reutilizável
- [x] Adicionar fade-in/scale animation no `ConsentDialog`
- [x] Refatorar string split hack em `ConsentDialog.tsx:56-62` para keys separadas

---

### S11-033 — Clean up dead code and #[allow(dead_code)]

| Field                | Value                                               |
| -------------------- | --------------------------------------------------- |
| **Ticket ID**        | S11-033                                             |
| **Title**            | Limpar dead code e remover 25 `#[allow(dead_code)]` |
| **Priority**         | P2                                                  |
| **Type**             | Code Quality                                        |
| **Estimated Effort** | M                                                   |
| **Source Finding**   | C9 (MEDIUM)                                         |

#### Acceptance Criteria

- [x] Remover 9 dead helpers em `ecram.rs`
- [x] Remover 6 dead items em `iotservice.rs`
- [x] Remover 3 dead items em `hotkeys.rs`
- [x] Remover empty `useEffect` em `TrayPopup.tsx`
- [x] Remover diretório `bin/` vazio ou adicionar `.gitkeep`
- [x] `cargo test` + `npm run build` passam

---

## Sprint Exit Criteria

- [x] Todos os 33 tickets completos
- [x] `cargo test` + `cargo clippy` (0 warnings) + `npm run build` + `npm run lint` (0 warnings) passam
- [x] Code coverage > 60% Rust, > 40% frontend
- [x] Relatório de estabilidade: 0 findings MEDIUM restantes
