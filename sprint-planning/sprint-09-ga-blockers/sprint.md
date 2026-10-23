# Sprint 9 — GA Blockers: Critical Fixes

## Sprint Metadata

| Field                 | Value                                                                                             |
| --------------------- | ------------------------------------------------------------------------------------------------- |
| **Sprint Name**       | GA Blockers — Critical Fixes                                                                      |
| **Sprint Goal**       | Eliminar todos os 14 findings CRITICAL do relatório de estabilidade para desbloquear o release GA |
| **Duration Estimate** | 1.5 semanas (7.5 dias úteis)                                                                      |
| **Priority**          | P0 — Blocker de GA. Sem estas correções o release não pode prosseguir.                            |
| **Sprint Type**       | Security + RAI + DevOps + Code Quality                                                            |
| **Primary Owner**     | Full-stack engineer (Rust + TS)                                                                   |
| **Secondary Owner**   | Security reviewer                                                                                 |
| **Source**            | `docs/stability-report-2026-06-24.md`                                                             |

## Sprint Goal Statement

Catorze findings CRITICAL foram identificados na auditoria de estabilidade. Eles cobrem 5 domínios: Segurança (HMAC race), RAI/Privacidade (dark pattern, consent bypass, focus trap, storageNote enganosa), DevOps (updater quebrado), UX (silent failures), e Code Quality (unwrap em mutex, mem::zeroed, unsafe sem documentação). Ao final deste sprint, todos os 14 blockers devem estar corrigidos com testes de regressão, desbloqueando o caminho para GA.

---

## Tickets

### S9-001 — Fix HMAC key bootstrap race with cross-process file locking

| Field                | Value                                                                         |
| -------------------- | ----------------------------------------------------------------------------- |
| **Ticket ID**        | S9-001                                                                        |
| **Title**            | Eliminar TOCTOU race em `get_or_create_key` usando file locking cross-process |
| **Priority**         | P0                                                                            |
| **Type**             | Security                                                                      |
| **Estimated Effort** | M                                                                             |
| **Source Finding**   | S1 (CRITICAL, CWE-367)                                                        |

#### Description

`get_or_create_key` em `auth.rs:30-45` pode ser chamado simultaneamente pelo processo principal e pelo helper elevado durante a primeira inicialização. Ambos podem gerar chaves diferentes e escrever no mesmo arquivo. O último escritor vence, quebrando o canal HMAC desde a primeira chamada.

#### Affected Files

- `src-tauri/src/util/auth.rs` — `get_or_create_key` (~lines 30-45)
- `src-tauri/src/elev_bridge.rs` — `read_key()` calls
- `src-tauri/src/elevated.rs` — key loading

#### Acceptance Criteria

- [x] `get_or_create_key` usa `fs2::FileExt::lock_exclusive` no arquivo de chave com retry loop
- [x] Se o lock não for obtido em 5s, retorna erro em vez de gerar chave nova
- [x] Teste de regressão: simular chamada concorrente de 2 threads e verificar que ambas obtêm a mesma chave
- [x] Adicionar `fs2` crate ao `Cargo.toml`

#### Dependencies

- None (foundational fix)

---

### S9-002 — Remove auto-focus from "Allow" button in ConsentDialog

| Field                | Value                                                               |
| -------------------- | ------------------------------------------------------------------- |
| **Ticket ID**        | S9-002                                                              |
| **Title**            | Remover auto-focus do botão "Allow" no ConsentDialog (dark pattern) |
| **Priority**         | P0                                                                  |
| **Type**             | RAI / UX                                                            |
| **Estimated Effort** | XS                                                                  |
| **Source Finding**   | R1/U1 (CRITICAL, GDPR Art.7)                                        |

#### Description

`ConsentDialog.tsx:8-10` executa `allowRef.current?.focus()` no mount. Um usuário que pressione Enter imediatamente após o diálogo abrir concederá consentimento sem ler. Isso é um dark pattern que viola GDPR Art.7 (consentimento livre).

#### Affected Files

- `src/components/ConsentDialog.tsx` — `useEffect` focus (~lines 8-10)
- `src/components/ConsentDialog.tsx` — button styling (~lines 105-111)

#### Acceptance Criteria

- [x] Remover `allowRef.current?.focus()` do `useEffect`
- [x] Focar um elemento neutro (ex: o título do diálogo ou nenhum elemento)
- [x] Equalizar peso visual dos botões "Allow" e "Deny" (ambos `btn-primary` ou ambos `btn-ghost`)
- [x] Teste manual: abrir diálogo, pressionar Enter imediatamente — NÃO deve conceder consentimento

#### Dependencies

- None

---

### S9-003 — Add consent check to testConnection()

| Field                | Value                                                  |
| -------------------- | ------------------------------------------------------ |
| **Ticket ID**        | S9-003                                                 |
| **Title**            | Adicionar verificação de consent em `testConnection()` |
| **Priority**         | P0                                                     |
| **Type**             | RAI                                                    |
| **Estimated Effort** | XS                                                     |
| **Source Finding**   | R2 (CRITICAL, GDPR Art.6)                              |

#### Description

`testConnection()` em `useSettings.ts:262-287` envia um prompt minimal para OpenAI para verificar a API key, mas nunca verifica o consent de telemetria. Isso é uma transmissão de dados sem autorização do usuário.

#### Affected Files

- `src/hooks/useSettings.ts` — `testConnection()` (~lines 262-287)

#### Acceptance Criteria

- [x] Adicionar `getTelemetryConsent()` check no início de `testConnection()`
- [x] Se consent não concedido, retornar erro amigável: "Consent required to test connection"
- [x] Teste: com consent negado, `testConnection()` não deve fazer chamada API

#### Dependencies

- None

---

### S9-004 — Fix misleading storageNote string

| Field                | Value                                                                 |
| -------------------- | --------------------------------------------------------------------- |
| **Ticket ID**        | S9-004                                                                |
| **Title**            | Corrigir string `storageNote` enganosa sobre armazenamento da API key |
| **Priority**         | P0                                                                    |
| **Type**             | RAI                                                                   |
| **Estimated Effort** | XS                                                                    |
| **Source Finding**   | R5 (CRITICAL, Transparency)                                           |

#### Description

`en.json:541` diz: "API key is stored locally in this browser's localStorage and never leaves your device." Isso é INCORRETO — a key está no Windows Credential Manager (via keyring) e SIM sai do dispositivo quando enviada para OpenAI em requisições API.

#### Affected Files

- `src/i18n/en.json` — `storageNote` (~line 541)
- `src/i18n/pt.json` — corresponding key
- `src/i18n/es.json` — corresponding key
- `src/i18n/fr.json` — corresponding key

#### Acceptance Criteria

- [x] Corrigir string para: "API key is stored securely in Windows Credential Manager. It is sent to the AI provider when making analysis requests."
- [x] Atualizar traduções em pt, es, fr
- [x] Verificar que nenhuma outra string menciona localStorage para API key

#### Dependencies

- None

---

### S9-005 — Implement focus trap and :focus-visible in modals

| Field                | Value                                                           |
| -------------------- | --------------------------------------------------------------- |
| **Ticket ID**        | S9-005                                                          |
| **Title**            | Implementar focus trap e indicadores de foco visíveis em modais |
| **Priority**         | P0                                                              |
| **Type**             | RAI / UX / Accessibility                                        |
| **Estimated Effort** | L                                                               |
| **Source Finding**   | R3/R4/U1 (CRITICAL, WCAG 2.4.3 + 2.4.7)                         |

#### Description

`ConsentDialog.tsx` e `InfoModal.tsx` não implementam focus trap — Tab pode escapar para elementos atrás do diálogo. Além disso, nenhum componente tem `:focus-visible` styling, falhando WCAG 2.4.7 (Focus Visible).

#### Affected Files

- `src/components/ConsentDialog.tsx` — focus trap implementation
- `src/components/InfoModal.tsx` — focus trap implementation
- `src/globals.css` — `:focus-visible` styles

#### Acceptance Criteria

- [x] Implementar focus trap em `ConsentDialog`: Tab cicla apenas entre elementos do diálogo
- [x] Implementar focus trap em `InfoModal`: mesmo comportamento
- [x] Adicionar `:focus-visible` outline em todos elementos interativos (buttons, toggles, inputs, links)
- [x] Adicionar `:focus-visible` em `globals.css` com `outline: 2px solid var(--accent)` e `outline-offset: 2px`
- [x] Teste manual: navegar por Tab através do diálogo — foco não deve escapar
- [x] Teste manual: Tab através da página principal — outline visível em cada elemento focado

#### Dependencies

- S9-002 (auto-focus removal should happen first)

---

### S9-006 — Fix silent failures: replace console.error with user-visible feedback

| Field                | Value                                                                                 |
| -------------------- | ------------------------------------------------------------------------------------- |
| **Ticket ID**        | S9-006                                                                                |
| **Title**            | Eliminar silent failures — substituir `console.error` por feedback visível ao usuário |
| **Priority**         | P0                                                                                    |
| **Type**             | UX                                                                                    |
| **Estimated Effort** | M                                                                                     |
| **Source Finding**   | U2 (CRITICAL)                                                                         |

#### Description

Múltiplos locais capturam erros e apenas logam no console sem feedback ao usuário: `AudioControl.tsx:44`, `MainWindow.tsx:1170`, `MainWindow.tsx:1240`. O usuário nunca sabe que a operação falhou.

#### Affected Files

- `src/components/AudioControl.tsx` — catch blocks (~lines 44-46)
- `src/pages/MainWindow.tsx` — catch blocks (~lines 1170, 1240)
- Any other `console.error` in catch blocks without `addToast`

#### Acceptance Criteria

- [x] Auditar todos `console.error` em catch blocks — identificar os que faltam feedback visual
- [x] Substituir cada `console.error` silencioso por `addToast` com mensagem i18n amigável
- [x] Manter `console.error` para debug, mas SEMPRE adicionar `addToast` para o usuário
- [x] Teste: simular falha em `get_audio_devices` — toast de erro deve aparecer

#### Dependencies

- None

---

### S9-007 — Generate latest.json updater manifest in release workflow

| Field                | Value                                                               |
| -------------------- | ------------------------------------------------------------------- |
| **Ticket ID**        | S9-007                                                              |
| **Title**            | Gerar e uploadar `latest.json` updater manifest no release workflow |
| **Priority**         | P0                                                                  |
| **Type**             | DevOps                                                              |
| **Estimated Effort** | S                                                                   |
| **Source Finding**   | D1 (CRITICAL, Updater broken)                                       |

#### Description

O Tauri updater está configurado em `tauri.conf.json` para buscar de `.../releases/latest/download/latest.json`, mas o release workflow nunca cria ou uploada este arquivo. Updates silenciosamente falham.

#### Affected Files

- `.github/workflows/release.yml` — add `latest.json` generation step

#### Acceptance Criteria

- [x] Adicionar step pós-build que gera `latest.json` com version, pub_date, platforms, signature, url
- [x] Uploadar `latest.json` como asset do GitHub Release
- [x] Teste: após release, `latest.json` deve ser acessível via `releases/latest/download/latest.json`
- [x] Documentar formato do `latest.json` em `docs/release.md`

#### Dependencies

- None

---

### S9-008 — Make Tauri signing key missing a hard error

| Field                | Value                                                       |
| -------------------- | ----------------------------------------------------------- |
| **Ticket ID**        | S9-008                                                      |
| **Title**            | Transformar chave de signing efêmera fallback em hard error |
| **Priority**         | P0                                                          |
| **Type**             | DevOps                                                      |
| **Estimated Effort** | XS                                                          |
| **Source Finding**   | D2 (CRITICAL, Supply chain)                                 |

#### Description

Se `TAURI_SIGNING_PRIVATE_KEY` está vazio, o release workflow gera uma chave efêmera. A chave pública no `tauri.conf.json` não combina com esta chave, fazendo TODOS os updates falharem verificação em produção.

#### Affected Files

- `.github/workflows/release.yml` — signing key fallback (~lines 47-55)

#### Acceptance Criteria

- [x] Remover o fallback de chave efêmera
- [x] Se `TAURI_SIGNING_PRIVATE_KEY` está vazio, falhar o workflow com erro claro
- [x] Adicionar mensagem: "TAURI_SIGNING_PRIVATE_KEY secret is required for releases. See docs/release.md for setup."
- [x] Documentar procedimento de geração de chave em `docs/release.md`

#### Dependencies

- None

---

### S9-009 — Replace REMAP_STATE.lock().unwrap() with lock_or_recover()

| Field                | Value                                                                       |
| -------------------- | --------------------------------------------------------------------------- |
| **Ticket ID**        | S9-009                                                                      |
| **Title**            | Substituir `REMAP_STATE.lock().unwrap()` por `lock_or_recover()` (12 sites) |
| **Priority**         | P0                                                                          |
| **Type**             | Code Quality                                                                |
| **Estimated Effort** | M                                                                           |
| **Source Finding**   | C1 (CRITICAL, Panic risk)                                                   |

#### Description

`hotkeys.rs:2264-2436` usa `REMAP_STATE.lock().unwrap()` em 12 sites. O framework `lock_or_recover()` foi projetado exatamente para isto, mas é completamente ignorado. Um panic enquanto segura o mutex mataria a thread de hotkeys.

#### Affected Files

- `src-tauri/src/hw/hotkeys.rs` — 12 call sites (~lines 2264-2436)

#### Acceptance Criteria

- [x] Substituir todos os 12 `REMAP_STATE.lock().unwrap()` por `lock_or_recover(&REMAP_STATE, "REMAP_STATE")`
- [x] Verificar que `lock_or_recover` está importado de `util::panic`
- [x] `cargo test` passa sem regressões
- [x] `cargo clippy` não introduz novos warnings

#### Dependencies

- None

---

### S9-010 — Validate Raw Input buffer size before pointer cast

| Field                | Value                                                                       |
| -------------------- | --------------------------------------------------------------------------- |
| **Ticket ID**        | S9-010                                                                      |
| **Title**            | Validar tamanho do buffer de Raw Input antes do cast para `*const RAWINPUT` |
| **Priority**         | P0                                                                          |
| **Type**             | Code Quality / Security                                                     |
| **Estimated Effort** | S                                                                           |
| **Source Finding**   | C3 (CRITICAL, CWE-822)                                                      |

#### Description

`hotkeys.rs` e `touchpad.rs` fazem `buf.as_ptr() as *const RAWINPUT` sem validar que o buffer é grande o suficiente para conter um `RAWINPUT`. Se o buffer for menor, isso lê além da alocação.

#### Affected Files

- `src-tauri/src/hw/hotkeys.rs` — `raw_input_wnd_proc` (~line 725)
- `src-tauri/src/hw/touchpad.rs` — `process_raw_input` (~line 858)

#### Acceptance Criteria

- [x] Adicionar check: `if written < size_of::<RAWINPUT>() as u32 { return; }` antes do cast
- [x] Logar warning se buffer for menor que esperado
- [x] Teste: verificar que input inválido não causa crash

#### Dependencies

- None

---

### S9-011 — Add safety invariant comments to all unsafe blocks

| Field                | Value                                                              |
| -------------------- | ------------------------------------------------------------------ |
| **Ticket ID**        | S9-011                                                             |
| **Title**            | Adicionar safety invariant comments em todos os 98 blocos `unsafe` |
| **Priority**         | P0                                                                 |
| **Type**             | Code Quality                                                       |
| **Estimated Effort** | XL                                                                 |
| **Source Finding**   | C4 (CRITICAL, Best practice)                                       |

#### Description

98 blocos `unsafe` em 16 arquivos não têm safety comments. Isso é uma violação de best practice do Rust e impede revisão de segurança. Cada bloco deve documentar as precondições que tornam a operação segura.

#### Affected Files

- All 16 files with `unsafe` blocks: `hotkeys.rs`, `touchpad.rs`, `ecram.rs`, `iotservice.rs`, `charging.rs`, `display.rs`, `audio.rs`, `battery.rs`, `performance.rs`, `discovery.rs`, `osd.rs`, `system_info.rs`, `startup.rs`, `fan.rs`, `elevated.rs`, `elev_bridge.rs`

#### Acceptance Criteria

- [x] Cada bloco `unsafe` tem um comentário `// SAFETY:` explicando as precondições
- [x] Para `mem::zeroed()`: explicar por que zero-inicialização é válida para aquele tipo
- [x] Para pointer casts: explicar por que o ponteiro é válido e alinhado
- [x] Para FFI calls: explicar quais invariantes o caller deve manter
- [x] `cargo test` passa sem regressões

#### Dependencies

- None (can be done in parallel with other tickets)

---

### S9-012 — Replace std::mem::zeroed() on registry handles with MaybeUninit

| Field                | Value                                                                            |
| -------------------- | -------------------------------------------------------------------------------- |
| **Ticket ID**        | S9-012                                                                           |
| **Title**            | Substituir `std::mem::zeroed()` em registry handles por `MaybeUninit` (24 sites) |
| **Priority**         | P0                                                                               |
| **Type**             | Code Quality                                                                     |
| **Estimated Effort** | L                                                                                |
| **Source Finding**   | C2 (CRITICAL, CWE-824 / UB)                                                      |

#### Description

24 call sites em 10 arquivos usam `let mut hkey = std::mem::zeroed()` para registry handles. Se a API falha, o handle zeroed é passado para `RegCloseKey` — isso é UB. Windows pode ignorar handles inválidos em debug, mas produção pode crashar ou vazar kernel handles.

#### Affected Files

- 10 files in `src-tauri/src/hw/`: `charging.rs`, `performance.rs`, `display.rs`, `fan.rs`, `touchpad.rs`, `startup.rs`, `battery.rs`, `discovery.rs`, `osd.rs`, `system_info.rs`

#### Acceptance Criteria

- [x] Criar helper `util/registry.rs` com `fn open_reg_key() -> Result<Option<HKEY>>` que usa `MaybeUninit`
- [x] Substituir todos os 24 `std::mem::zeroed()` para registry handles pelo helper
- [x] Garantir que `RegCloseKey` só é chamado se o handle foi inicializado com sucesso
- [x] `cargo test` passa sem regressões
- [x] Eliminar código duplicado de registry read/write (também resolve C11)

#### Dependencies

- None (but pairs well with S9-011 for safety comments)

---

### S9-013 — Add Escape key handler and dismiss options to modals

| Field                | Value                                                      |
| -------------------- | ---------------------------------------------------------- |
| **Ticket ID**        | S9-013                                                     |
| **Title**            | Adicionar Escape key handler e opções de dismiss em modais |
| **Priority**         | P0                                                         |
| **Type**             | RAI / UX                                                   |
| **Estimated Effort** | S                                                          |
| **Source Finding**   | R8/U20 (HIGH, WCAG)                                        |

#### Description

`ConsentDialog.tsx` e `InfoModal.tsx` não fecham com Escape. O consent dialog também não tem botão X nem backdrop click. WCAG requer que todos os diálogos sejam dismissable via keyboard.

#### Affected Files

- `src/components/ConsentDialog.tsx` — add `onKeyDown` Escape handler
- `src/components/InfoModal.tsx` — add `onKeyDown` Escape handler

#### Acceptance Criteria

- [x] Adicionar `onKeyDown` handler em `ConsentDialog` que fecha (deny) com Escape
- [x] Adicionar `onKeyDown` handler em `InfoModal` que fecha com Escape
- [x] Adicionar backdrop click-to-dismiss em `InfoModal`
- [x] Teste manual: pressionar Escape fecha os modais

#### Dependencies

- S9-005 (focus trap should be implemented first)

---

### S9-014 — Add React Error Boundary

| Field                | Value                                                              |
| -------------------- | ------------------------------------------------------------------ |
| **Ticket ID**        | S9-014                                                             |
| **Title**            | Adicionar React Error Boundary para prevenir white-screen-of-death |
| **Priority**         | P0                                                                 |
| **Type**             | Code Quality                                                       |
| **Estimated Effort** | S                                                                  |
| **Source Finding**   | C13 (MEDIUM, but critical for stability)                           |

#### Description

Sem Error Boundary, se qualquer componente crashar durante o render, a janela inteira desmonta para uma tela branca. Isso é inaceitável para um app de controle de hardware.

#### Affected Files

- `src/App.tsx` — wrap with ErrorBoundary
- `src/components/ErrorBoundary.tsx` — new file

#### Acceptance Criteria

- [x] Criar `ErrorBoundary.tsx` com fallback UI amigável (mensagem + botão "Reload")
- [x] Wrap `App.tsx` com `ErrorBoundary`
- [x] Logar erro no console + toast
- [x] Teste: forçar throw em um componente — ErrorBoundary deve capturar e mostrar fallback

#### Dependencies

- None

---

## Sprint Exit Criteria

- [x] Todos os 14 tickets completos com acceptance criteria verificadas
- [x] `cargo test` passa (142+ testes, sem regressões)
- [x] `npm run build` + `npm run lint` + `npm run format:check` passam
- [x] `cargo clippy` sem novos warnings
- [x] `npm run version:check` passa
- [x] Relatório de estabilidade re-auditado: 0 findings CRITICAL restantes
