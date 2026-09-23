# 📋 Relatório de Estabilidade — miPC/micontrol

**Data:** 2026-06-24  
**Versão:** 0.1.0  
**Stack:** Tauri v2 (Rust backend) + React 19 (TypeScript frontend)  
**Escopo:** Auditoria completa — Segurança, Arquitetura, Performance, RAI/Privacidade, DevOps/CI, UX, Qualidade de Código  
**Modelo:** DeepSeek V4 Flash (7 subagentes especializados)  
**Testes:** 142 testes Rust passando, 9 health checks passando, 19 commits à frente de origin/master

---

## Sumário Executivo

O miControl é uma aplicação de controle de hardware para laptops Xiaomi com arquitetura Tauri v2 + Rust + React 19. A auditoria revelou uma base de código com disciplina de engenharia acima da média para um v0.1.0 — o protocolo de elevação com HMAC-SHA256, o sistema de recuperação de panic, e o cache WMI thread-local são destaques arquiteturais. No entanto, foram identificados **~100+ findings** distribuídos em 7 domínios, com **14 CRITICAL**, **30+ HIGH**, e o restante MEDIUM/LOW.

### Distribuição de Risco Consolidada

| Severidade  | Contagem | Domínios Afetados                                            |
| ----------- | -------- | ------------------------------------------------------------ |
| 🔴 CRITICAL | 14       | Segurança (1), RAI (5), DevOps (2), UX (2), Code Quality (4) |
| 🟠 HIGH     | 30+      | Todos os 7 domínios                                          |
| 🟡 MEDIUM   | 40+      | Todos os 7 domínios                                          |
| 🟢 LOW      | 20+      | Todos os 7 domínios                                          |

### Veredito de Prontidão para GA

| Área            | Status                           | Bloqueia GA?                             |
| --------------- | -------------------------------- | ---------------------------------------- |
| Segurança       | ⚠️ 1 CRITICAL + 5 HIGH           | ✅ Sim — fix HMAC race + unsafe pointers |
| Arquitetura     | ✅ Estrutura sólida              | ❌ Não — dívida estrutural               |
| Performance     | ⚠️ 2 HIGH (WMI cache bypass)     | ❌ Não — mas impacta UX                  |
| RAI/Privacidade | 🔴 5 CRITICAL                    | ✅ Sim — dark pattern + GDPR             |
| DevOps/CI       | 🔴 2 CRITICAL (updater quebrado) | ✅ Sim — updates não funcionam           |
| UX              | 🔴 3 CRITICAL                    | ✅ Sim — acessibilidade WCAG             |
| Code Quality    | 🔴 4 CRITICAL (unsafe)           | ⚠️ Parcialmente — UB em produção         |

---

## Matriz de Risco — Top 20 Findings Críticos

| #   | Finding                                               | Domínio      | Severidade  | Arquivo:Linhas                         | CWE/Standard     |
| --- | ----------------------------------------------------- | ------------ | ----------- | -------------------------------------- | ---------------- |
| 1   | HMAC key bootstrap race (TOCTOU)                      | Segurança    | 🔴 CRITICAL | `auth.rs:30-45`                        | CWE-367          |
| 2   | Consent dialog auto-foca botão "Allow" (dark pattern) | RAI/UX       | 🔴 CRITICAL | `ConsentDialog.tsx:8-10`               | GDPR Art.7       |
| 3   | `testConnection()` bypassa consent check              | RAI          | 🔴 CRITICAL | `useSettings.ts:262-287`               | GDPR Art.6       |
| 4   | Sem focus trap em modais (ConsentDialog, InfoModal)   | RAI/UX       | 🔴 CRITICAL | `ConsentDialog.tsx`, `InfoModal.tsx`   | WCAG 2.4.3       |
| 5   | Sem indicadores de foco visíveis                      | RAI/UX       | 🔴 CRITICAL | Todos componentes                      | WCAG 2.4.7       |
| 6   | String `storageNote` enganosa sobre API key           | RAI          | 🔴 CRITICAL | `en.json:541`                          | Transparency     |
| 7   | `latest.json` updater manifest não gerado no release  | DevOps       | 🔴 CRITICAL | `release.yml` (missing)                | Updater broken   |
| 8   | Chave de signing efêmera quebra updater em produção   | DevOps       | 🔴 CRITICAL | `release.yml:47-55`                    | Supply chain     |
| 9   | `unwrap()` em mutex REMAP_STATE (12 sites)            | Code Quality | 🔴 CRITICAL | `hotkeys.rs:2264-2436`                 | Panic risk       |
| 10  | `std::mem::zeroed()` em registry handles (24 sites)   | Code Quality | 🔴 CRITICAL | 10 arquivos `hw/*.rs`                  | UB / CWE-824     |
| 11  | Raw Input buffer sem validação de tamanho             | Code Quality | 🔴 CRITICAL | `hotkeys.rs`, `touchpad.rs`            | CWE-822          |
| 12  | 98 blocos `unsafe` sem safety comments                | Code Quality | 🔴 CRITICAL | 16 arquivos                            | Best practice    |
| 13  | ACL restriction ignora falhas do `icacls`             | Segurança    | 🟠 HIGH     | `auth.rs:150-170`                      | CWE-391          |
| 14  | Elevation bypass via newest-file selection            | Segurança    | 🟠 HIGH     | `elevated.rs:95-120`                   | CWE-73           |
| 15  | Unsafe pointer dereference em keyboard hook           | Segurança    | 🟠 HIGH     | `hotkeys.rs:1030`                      | CWE-822          |
| 16  | CSP permite `unsafe-inline` styles                    | Segurança    | 🟠 HIGH     | `tauri.conf.json`                      | CWE-79           |
| 17  | `fan.rs` e `processes.rs` bypassam WMI cache          | Performance  | 🟠 HIGH     | `fan.rs:138-150`, `processes.rs:34-48` | +100-400ms/batch |
| 18  | Sem assinatura Authenticode no release                | DevOps       | 🟠 HIGH     | `release.yml` (missing)                | SmartScreen      |
| 19  | Sem `cargo audit` / `npm audit` em CI                 | DevOps       | 🟠 HIGH     | `ci.yml` (missing)                     | CVE exposure     |
| 20  | Sem mecanismo de deleção de dados (GDPR Art.17)       | RAI          | 🟠 HIGH     | App-wide                               | GDPR             |

---

## 1. 🔒 Segurança

### Pontos Fortes

- **HMAC-SHA256 signed elevated bridge** com nonces, timestamps (±30s), e comparação constant-time (`auth.rs`)
- **Atomic file writes** com TOCTOU prevention (`.tmp` + rename) (`elev_bridge.rs:100-112`)
- **XML injection prevention** — 5 metacaracteres escapados, SSID ≤32 bytes, WPA2 8-63 char (`util/xml.rs`)
- **ECRAM write allowlist** — 9 offsets seguros, raw writes requerem env var (`ecram.rs:400-450`)
- **IoTService response authentication** — `src_id`/`dst_id` cross-check, fail-closed (`iotservice.rs:340-360`)
- **Script action security** — 3 camadas: feature flag + allowlist + SHA-256 consent (`hotkeys.rs:1150-1200`)
- **Panic recovery** — poisoned mutex recovery, global panic hook, `spawn_with_recovery` (`util/panic.rs`)
- **Credential store** via OS keyring (`keyring` crate) (`commands/credentials.rs`)

### Findings

| #   | Severidade  | Finding                                                                                                                                        | Arquivo:Linhas           | Recomendação                                                           |
| --- | ----------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------ | ---------------------------------------------------------------------- |
| S1  | 🔴 CRITICAL | HMAC key bootstrap race — `get_or_create_key` pode gerar chaves diferentes se chamado simultaneamente pelo processo principal e helper elevado | `auth.rs:30-45`          | Usar `fs2::FileExt::lock_exclusive` no arquivo de chave com retry loop |
| S2  | 🟠 HIGH     | ACL restriction via `icacls` ignora falhas silenciosamente                                                                                     | `auth.rs:150-170`        | Usar `SetSecurityInfo` Win32 API; retornar erro se falhar              |
| S3  | 🟠 HIGH     | Elevation bypass — helper pega arquivo mais novo sem `--request-id`                                                                            | `elevated.rs:95-120`     | Sempre passar `--request-id`; adicionar nonce anti-replay              |
| S4  | 🟠 HIGH     | Unsafe pointer cast em keyboard hook sem validação de tamanho                                                                                  | `hotkeys.rs:1030`        | Usar `read_unaligned` com size validation                              |
| S5  | 🟠 HIGH     | CSP permite `unsafe-inline` para styles                                                                                                        | `tauri.conf.json`        | Remover `'unsafe-inline'`; usar nonce/hash                             |
| S6  | 🟠 HIGH     | Unsafe IPC header parsing via raw pointer cast                                                                                                 | `iotservice.rs:168`      | Usar `bytemuck` ou validação manual com `#[repr(C)]`                   |
| S7  | 🟡 MEDIUM   | ECRAM read aceita `phys_addr` arbitrário                                                                                                       | `ecram.rs:60`            | Adicionar range check fail-closed                                      |
| S8  | 🟡 MEDIUM   | IoT pipe polling consome CPU (500 iterações em 5s)                                                                                             | `iotservice.rs:280-320`  | Usar `OVERLAPPED` I/O                                                  |
| S9  | 🟡 MEDIUM   | API key exposta em JS memory do frontend                                                                                                       | `useSettings.ts:240-300` | Roteirizar chamadas via backend Tauri command                          |
| S10 | 🟡 MEDIUM   | Telemetry coletada antes do consent check                                                                                                      | `useSettings.ts:245-248` | Verificar consent antes de qualquer coleta                             |
| S11 | 🟡 MEDIUM   | Scheduled task elevation não verificada em runtime                                                                                             | `elev_bridge.rs:130-155` | Logar RunLevel; adicionar self-test ping                               |
| S12 | 🟢 LOW      | Copilot remap injeta modifier key-up sem verificação                                                                                           | `hotkeys.rs:1500-1515`   | Gate em `GetAsyncKeyState`                                             |
| S13 | 🟢 LOW      | WMI HID listener sem reconnection logic                                                                                                        | `hotkeys.rs:1625-1640`   | Retry com exponential backoff                                          |
| S14 | 🟢 LOW      | Endereços físicos de memória hardcoded                                                                                                         | `ecram.rs:20-50`         | Auto-discovery DSDT fallback                                           |

### OWASP Top 10 Mapping

| Categoria                      | Status                                        |
| ------------------------------ | --------------------------------------------- |
| A01: Broken Access Control     | ⚠️ HMAC + is_admin, mas finding S3 enfraquece |
| A02: Cryptographic Failures    | ✅ HMAC-SHA256, constant-time, nonces         |
| A03: Injection                 | ✅ XML escaping completo                      |
| A04: Insecure Design           | ⚠️ Key bootstrap race (S1)                    |
| A05: Security Misconfiguration | ⚠️ CSP muito permissivo (S5)                  |
| A07: Auth Failures             | ✅ HMAC + OS keyring                          |
| A08: Data Integrity Failures   | ✅ Signed payloads, atomic writes             |

---

## 2. 🏗️ Arquitetura

### Pontos Fortes

- **Separação em 3 camadas** limpa: Commands → Hardware → Utilities (sem dependências circulares)
- **Elevated bridge** com HMAC auth, atomic writes, UAC fallback, fast-path in-process
- **Panic recovery** production-grade (`lock_or_recover`, `spawn_with_recovery`)
- **Frontend optimistic updates** com snapshot-rollback e `touchpadDirtyUntil`
- **Batched hardware state** — 1 IPC call substitui 8 individuais
- **WMI connection cache** thread-local com auto-invalidation
- **Release profile** otimizado: `panic="unwind"`, `codegen-units=1`, `lto=true`, `strip=true`

### Findings

| #   | Severidade    | Finding                                                                                                     | Esforço         | Recomendação                                        |
| --- | ------------- | ----------------------------------------------------------------------------------------------------------- | --------------- | --------------------------------------------------- |
| A1  | 🟠 HIGH       | Error type fragmentation — `HardwareError` existe mas só `wifi.rs` usa; 14/16 módulos usam `anyhow::Result` | 3-5 dias        | Migrar todos módulos `hw/` para `HardwareResult<T>` |
| A2  | 🟡 MEDIUM     | Estado global fragmentado em 4 mecanismos (AppState, OnceLock, AtomicBool, thread_local)                    | 1-2 dias        | Consolidar em Tauri AppState                        |
| A3  | 🟡 MEDIUM     | Error stringification no boundary — `.to_string()` perde error chain                                        | 1 dia (após A1) | Usar `ErrorResponse` serialization                  |
| A4  | 🟡 MEDIUM     | Sem HAL trait — adicionar feature requer 6 passos manuais                                                   | 0.5 dia         | Documentar workflow em `docs/`                      |
| A5  | 🟡 LOW-MEDIUM | Polling agressivo de 2s para dados estáticos (battery, display)                                             | 0.5 dia         | Polling em tiers: fast (2s) + slow (15-30s)         |
| A6  | 🟢 LOW        | Logging backends duplicados (`env_logger` + `fern`)                                                         | 0.25 dia        | Consolidar para `fern`                              |
| A7  | 🟢 LOW        | Diretório `bin/` vazio                                                                                      | 0.1 dia         | Remover ou adicionar `.gitkeep`                     |
| A8  | 🟢 LOW        | Sem retry logic para operações flaky (WMI, pipe, HID)                                                       | 1 dia           | Adicionar 1 retry em erros transientes              |

### Well-Architected Assessment

| Pilar                  | Rating | Notas                                             |
| ---------------------- | ------ | ------------------------------------------------- |
| Reliability            | ⚠️     | Panic recovery forte, mas error propagation fraca |
| Security               | ✅     | HMAC excelente, mas key rotation ausente          |
| Cost Optimization      | ✅     | Binary size otimizado, deps razoáveis             |
| Operational Excellence | ✅     | Logging bom, mas error observability fraca        |
| Performance Efficiency | ✅     | WMI caching excelente, mas polling agressivo      |

---

## 3. ⚡ Performance

### Pontos Fortes

- **WMI cache thread-local** com auto-invalidation — design correto para COM thread affinity
- **Batched IPC** — `get_hardware_state_batch` em único `spawn_blocking`
- **`spawn_blocking` discipline** — toda operação WMI/COM/registry delegada
- **Visibility-gated polling** — não polla quando janela não está visível
- **Minimal dependencies** — sem lodash, moment, antd, ou charting libs
- **HID preparsed cache** com `Rc<Vec<u8>>` evita cloning

### Findings

| #   | Severidade | Finding                                                                | Arquivo:Linhas               | Impacto                            | Recomendação                                      |
| --- | ---------- | ---------------------------------------------------------------------- | ---------------------------- | ---------------------------------- | ------------------------------------------------- |
| P1  | 🟠 HIGH    | `fan.rs` bypassa WMI cache — cria COM+WMI fresh a cada query           | `fan.rs:138-150`             | +100-200ms/batch                   | Roteirizar via `wmi_cache::with_cimv2()`          |
| P2  | 🟠 HIGH    | `processes.rs` bypassa WMI cache — mesmo padrão                        | `processes.rs:34-48`         | +100-200ms/batch                   | Roteirizar via `wmi_cache::with_cimv2()`          |
| P3  | 🟡 MEDIUM  | `get_esif_readings()` cria WMI connection fresh                        | `fan.rs:104-112`             | +50ms/query                        | Usar `wmi_cache::with_wmi()`                      |
| P4  | 🟡 MEDIUM  | `useMemo` com 30+ deps — todos consumers re-renderizam a cada 2s       | `useHardware.ts:822-880`     | Re-render cascade                  | Split em grupos lógicos ou `useSyncExternalStore` |
| P5  | 🟡 MEDIUM  | HID device handle reaberto a cada haptics write                        | `touchpad.rs:482-510`        | ~5-15ms/write                      | Manter handle aberto com RAII                     |
| P6  | 🟡 MEDIUM  | localStorage analysis logs (500 entries, ~1MB) parseado na main thread | `useAnalysisLogger.ts`       | 5-15ms JSON.parse                  | Usar IndexedDB ou trim para 100 entries           |
| P7  | 🟢 LOW     | `vec![0u8; size]` alocado a cada WM_INPUT frame                        | `touchpad.rs:858`            | Alloc 120x/s @ 120Hz               | Stack buffer `[u8; 4096]`                         |
| P8  | 🟢 LOW     | 6 `useEffect` separados para ref tracking                              | `useHardware.ts:555-566`     | Micro overhead                     | Consolidar em 1 useEffect                         |
| P9  | 🟢 LOW     | `getAudioState()` redundante após `setMasterVolume`/`Mute`             | `useHardware.ts:722,732`     | IPC extra round-trip               | Confiar no próximo poll                           |
| P10 | 🟢 LOW     | TRACE logging no hot path de 2s                                        | `battery.rs`, `debug_log.rs` | 10-20 log lines/poll               | Reduzir para DEBUG                                |
| P11 | 🟢 LOW     | Sem code splitting                                                     | `vite.config.ts`             | TrayPopup carrega JS desnecessário | Dynamic imports por tab                           |
| P12 | 🟢 LOW     | `Box::leak` para PDH libraries                                         | `system_info.rs:40,153`      | ~100KB leaked                      | Aceitável (process lifetime)                      |
| P13 | 🟢 LOW     | Queries sequenciais no batch                                           | `system.rs:228-238`          | Latência = soma                    | Paralelizar com `rayon`                           |

---

## 4. 🤖 Responsible AI & Privacidade

### Pontos Fortes

- **Consent stored in OS keyring** (não localStorage) — Windows Credential Manager
- **Consent record** com ISO timestamp + policy version
- **Consent check** gates toda chamada API (exceto `testConnection` — bug)
- **Revoke é imediato** — deleta secret, para transmissão
- **Consent dialog** é modal verdadeiro (`role="dialog"`, `aria-modal`)
- **AI é advisory-only** — sem controle autônomo de hardware
- **Sem dados demográficos** coletados
- **Multi-language AI responses** — instrução de responder no idioma do UI
- **Temperature 0.3/0.4** — baixa variância, menos bias
- **API key migration** de localStorage para keyring implementada

### Findings

| #   | Severidade  | Finding                                                                                      | Arquivo:Linhas                       | Recomendação                              |
| --- | ----------- | -------------------------------------------------------------------------------------------- | ------------------------------------ | ----------------------------------------- |
| R1  | 🔴 CRITICAL | Consent dialog auto-foca "Allow" — dark pattern                                              | `ConsentDialog.tsx:8-10`             | Remover auto-focus; focar elemento neutro |
| R2  | 🔴 CRITICAL | `testConnection()` envia dados sem consent check                                             | `useSettings.ts:262-287`             | Adicionar consent gate                    |
| R3  | 🔴 CRITICAL | Sem focus trap em ConsentDialog e InfoModal                                                  | `ConsentDialog.tsx`, `InfoModal.tsx` | Implementar focus trap                    |
| R4  | 🔴 CRITICAL | Sem indicadores de foco visíveis (WCAG 2.4.7 FAIL)                                           | Todos componentes                    | Adicionar `:focus-visible` styling        |
| R5  | 🔴 CRITICAL | `storageNote` diz "localStorage" mas key está em Credential Manager e SIM sai do dispositivo | `en.json:541`                        | Corrigir string                           |
| R6  | 🟠 HIGH     | Sem mecanismo de deleção de dados enviados (GDPR Art.17)                                     | App-wide                             | Adicionar "Delete All My Data"            |
| R7  | 🟠 HIGH     | `ai_perf_logs` acumula indefinidamente sem retention policy                                  | `ai_logs.rs:58`                      | Auto-rotação 30 dias + UI de deleção      |
| R8  | 🟠 HIGH     | Sem Escape key em modais                                                                     | `ConsentDialog.tsx`, `InfoModal.tsx` | Adicionar `onKeyDown` Escape handler      |
| R9  | 🟠 HIGH     | Emoji icons sem `aria-label` — screen readers leem aleatoriamente                            | `MainWindow.tsx:38-55`               | `aria-hidden="true"` + `aria-label`       |
| R10 | 🟠 HIGH     | Cor como único indicador de status (success/error)                                           | `SettingsPage.tsx:249-260`           | Adicionar ícones diferenciais             |
| R11 | 🟠 HIGH     | Sem high-contrast mode support                                                               | `globals.css`                        | Media query `prefers-contrast`            |
| R12 | 🟠 HIGH     | Min width 800px exclui telas pequenas                                                        | `tauri.conf.json`                    | Reduzir para 600px ou responsivo          |
| R13 | 🟠 HIGH     | Sem `role="alert"` em mensagens dinâmicas                                                    | `AiAdvisor.tsx`, `SettingsPage.tsx`  | Adicionar `role="alert"`                  |
| R14 | 🟡 MEDIUM   | Process names em logs locais revelam padrões de uso                                          | `useAnalysisLogger.ts:80-90`         | Hash ou truncar nomes                     |
| R15 | 🟡 MEDIUM   | Sem audit log de revogação de consent                                                        | `useSettings.ts:492-498`             | Registrar eventos de grant/revoke         |
| R16 | 🟡 MEDIUM   | Policy version hardcoded (1), sem re-prompt                                                  | `useSettings.ts:484`                 | Versionar + re-prompt em mudança          |
| R17 | 🟡 MEDIUM   | Sem `<label htmlFor>` em form inputs                                                         | `SettingsPage.tsx:96-102`            | Associar labels                           |
| R18 | 🟡 MEDIUM   | Sem landmark regions (`<nav>`, `<main>`, `<aside>`)                                          | App-wide                             | Adicionar semantic HTML                   |
| R19 | 🟡 MEDIUM   | Font sizes em px absoluto (não respeita zoom)                                                | Todos componentes                    | Usar `rem`                                |
| R20 | 🟡 MEDIUM   | Sem `prefers-reduced-motion`                                                                 | `globals.css`                        | Media query                               |
| R21 | 🟡 MEDIUM   | Local logs sem criptografia                                                                  | `ai_logs.rs`                         | Considerar encrypt at rest                |
| R22 | 🟡 MEDIUM   | Custom API endpoints não divulgados                                                          | `SettingsPage.tsx:162`               | Aviso sobre privacidade de terceiros      |
| R23 | 🟡 MEDIUM   | Schedule data persiste após revogação                                                        | `useAnalysisLogger.ts`               | Limpar em revoke                          |
| R24 | 🟢 LOW      | Sem `lang` attribute em `<html>`                                                             | `App.tsx`                            | Adicionar `lang={lang}`                   |
| R25 | 🟢 LOW      | Sem opção de deletar análises específicas                                                    | `useAnalysisLogger.ts`               | Adicionar delete individual               |
| R26 | 🟢 LOW      | Sem `aria-label` na sidebar `<nav>`                                                          | `MainWindow.tsx`                     | Adicionar `aria-label`                    |

### GDPR/CCPA Compliance

| Requisito                           | Status                    |
| ----------------------------------- | ------------------------- |
| Consentimento explícito (Art.7)     | ⚠️ Parcial — dark pattern |
| Direito à informação (Art.13-14)    | ✅ Met                    |
| Direito de acesso (Art.15)          | ⚠️ Parcial                |
| Direito ao apagamento (Art.17)      | ❌ Não atendido           |
| Portabilidade (Art.20)              | ❌ Não atendido           |
| Privacy by Design (Art.25)          | ⚠️ Parcial                |
| Registros de processamento (Art.30) | ❌ Não atendido           |
| CCPA opt-out                        | ✅ Met                    |

---

## 5. 🔧 DevOps & CI/CD

### Pontos Fortes

- **Pipeline CI de 3 jobs** (rust, frontend, tauri-smoke) com responsabilidades claras
- **Cargo registry + git caching** com `hashFiles(Cargo.lock)` na cache key
- **`npm ci`** (instalação determinística)
- **ESLint flat config** + TypeScript strict mode
- **Version sync** — fonte única (`package.json`) propaga para `Cargo.toml` + `tauri.conf.json`
- **Release profile** otimizado (LTO, unwind, strip)
- **Tauri updater signing** com secrets injection
- **Documentação de release** com key rotation procedure

### Findings

| #   | Severidade  | Finding                                                                            | Arquivo                  | Recomendação                                      |
| --- | ----------- | ---------------------------------------------------------------------------------- | ------------------------ | ------------------------------------------------- |
| D1  | 🔴 CRITICAL | `latest.json` updater manifest nunca gerado — updates silenciosamente falham       | `release.yml` (missing)  | Gerar e uploadar `latest.json`                    |
| D2  | 🔴 CRITICAL | Chave de signing efêmera fallback — chave pública no `tauri.conf.json` não combina | `release.yml:47-55`      | Hard error se `TAURI_SIGNING_PRIVATE_KEY` vazio   |
| D3  | 🟠 HIGH     | Sem assinatura Authenticode — SmartScreen warnings                                 | `release.yml` (missing)  | Adicionar `signtool`/`azure-sign-tool`            |
| D4  | 🟠 HIGH     | Sem `cargo audit` / `npm audit` — CVEs não detectados                              | `ci.yml` (missing)       | Adicionar steps de audit                          |
| D5  | 🟠 HIGH     | Sem pre-commit hooks (husky/lint-staged)                                           | `.github/` (missing)     | Adicionar husky + lint-staged                     |
| D6  | 🟠 HIGH     | Sem crash reporting externo (Sentry/Bugsnag)                                       | App-wide                 | Adicionar Sentry                                  |
| D7  | 🟠 HIGH     | Sem branch protection documentado                                                  | N/A                      | Documentar required checks                        |
| D8  | 🟠 HIGH     | `tauri-smoke` com `continue-on-error: true` — build pode estar quebrado em main    | `ci.yml:62`              | Remover continue-on-error ou mover para nightly   |
| D9  | 🟡 MEDIUM   | `clippy` com `continue-on-error: true` (48 warnings)                               | `ci.yml:35`              | Set warning budget; fail se exceder               |
| D10 | 🟡 MEDIUM   | Lint rules downgraded para `warn` (hooks, floating promises)                       | `eslint.config.js:28-36` | Migrar para `error`                               |
| D11 | 🟡 MEDIUM   | `docs/release.md` descreve signing que não existe em CI                            | `release.md:33-36`       | Alinhar docs com CI                               |
| D12 | 🟡 MEDIUM   | Sem semver validation em `sync-version.cjs`                                        | `sync-version.cjs:52`    | Adicionar regex `/^\d+\.\d+\.\d+/`                |
| D13 | 🟡 MEDIUM   | Sem dependabot/Renovate                                                            | `.github/` (missing)     | Adicionar config                                  |
| D14 | 🟡 MEDIUM   | `clippy.toml` efetivamente vazio                                                   | `clippy.toml:1-5`        | Definir thresholds                                |
| D15 | 🟡 MEDIUM   | Sem code coverage reporting                                                        | `ci.yml` (missing)       | Adicionar `cargo-tarpaulin` / `vitest --coverage` |
| D16 | 🟡 MEDIUM   | Sem rollback procedure documentado                                                 | `release.md` (missing)   | Documentar processo                               |
| D17 | 🟢 LOW      | `softprops/action-gh-release@v2` unpinned minor                                    | `release.yml:75`         | Pin to commit SHA                                 |
| D18 | 🟢 LOW      | Filename mismatch em sync-version comments                                         | `sync-version.cjs`       | Corrigir `.js` → `.cjs`                           |
| D19 | 🟢 LOW      | Sem `.cargo/config.toml`                                                           | Missing                  | Adicionar para target-cpu                         |
| D20 | 🟢 LOW      | `bundle.targets: "all"` pode gerar artefatos extras                                | `tauri.conf.json:37`     | Restringir se não assinados                       |

---

## 6. 🎨 UX/UI

### Pontos Fortes

- **Design system completo** — OKLCH tokens, 3 theme modes, glass morphism, animações
- **ToggleRow** implementa WAI-ARIA switch pattern completo (`role="switch"`, `aria-checked`, Enter/Space)
- **i18n architecture** com `useSyncExternalStore`, fallback EN, interpolação `{key}`, type safety
- **Skeleton loading** em TouchpadSettings e DisplaySettings
- **TrayPopup** com `ResizeObserver` + `requestAnimationFrame`
- **Toast system** com `role="log"` e `aria-live="polite"`
- **Transparent window startup** — background síncrono antes do first paint
- **Component decomposition** — tabs extraídos em named functions

### Findings

| #   | Severidade  | Finding                                                                | Arquivo:Linhas                                    | Recomendação                            |
| --- | ----------- | ---------------------------------------------------------------------- | ------------------------------------------------- | --------------------------------------- |
| U1  | 🔴 CRITICAL | Consent dialog: auto-focus Allow + styling assimétrico (dark pattern)  | `ConsentDialog.tsx:8-10,105-111`                  | Foco neutro; peso visual igual          |
| U2  | 🔴 CRITICAL | Silent failures — `console.error` sem feedback ao usuário              | `AudioControl.tsx:44`, `MainWindow.tsx:1170,1240` | Sempre mostrar toast em falhas          |
| U3  | 🟠 HIGH     | 18 itens na sidebar sem agrupamento — cognitive overload               | `MainWindow.tsx:72-90`                            | Agrupar em seções; collapse debug tools |
| U4  | 🟠 HIGH     | Sem loading indicator para audio device list                           | `AudioControl.tsx:38-48`                          | Adicionar spinner/skeleton              |
| U5  | 🟠 HIGH     | Sem skip-to-content link                                               | `MainWindow.tsx`                                  | Adicionar skip nav                      |
| U6  | 🟠 HIGH     | Emoji icons sem `aria-label`                                           | `MainWindow.tsx:72-90`                            | `aria-hidden` + `aria-label`            |
| U7  | 🟠 HIGH     | Strings hardcoded em inglês (não traduzidas)                           | `AudioControl.tsx:76,121`, `MainWindow.tsx:1175`  | Envolver em `t()`                       |
| U8  | 🟠 HIGH     | Sem RTL support                                                        | `globals.css`, `useI18n.ts`                       | CSS logical properties                  |
| U9  | 🟠 HIGH     | Debug tools expostos a todos usuários                                  | `MainWindow.tsx:88-89`                            | Gate atrás de developer mode            |
| U10 | 🟡 MEDIUM   | Styling paradigms inconsistentes (inline vs CSS classes)               | Throughout                                        | Padronizar em CSS classes               |
| U11 | 🟡 MEDIUM   | `SettingsPage.tsx` god component                                       | `SettingsPage.tsx`                                | Extrair sub-componentes                 |
| U12 | 🟡 MEDIUM   | Error messages expõem raw Rust/JS error text                           | `AudioControl.tsx:54`, `DisplaySettings.tsx:94`   | Mensagens i18n amigáveis                |
| U13 | 🟡 MEDIUM   | Sem retry mechanism para operações falhadas                            | Various                                           | Adicionar botão Retry                   |
| U14 | 🟡 MEDIUM   | Sem loading state para battery info                                    | `BatteryInfo.tsx`                                 | Adicionar skeleton                      |
| U15 | 🟡 MEDIUM   | Sidebar sem `aria-label`                                               | `MainWindow.tsx`                                  | `aria-label="Main navigation"`          |
| U16 | 🟡 MEDIUM   | Inline styles podem não respeitar forced colors mode                   | Throughout                                        | Usar CSS variables                      |
| U17 | 🟡 MEDIUM   | Sidebar fixed 192px sem breakpoints                                    | `globals.css`                                     | Media queries                           |
| U18 | 🟡 MEDIUM   | Locales não-EN faltam keys (fr notably)                                | `fr.json`, `es.json`, `pt.json`                   | Completar traduções                     |
| U19 | 🟡 MEDIUM   | Sem CSS spinner animations                                             | `globals.css`                                     | Adicionar keyframe spinner              |
| U20 | 🟡 MEDIUM   | Consent dialog não dismissable (sem X, sem Escape, sem backdrop click) | `ConsentDialog.tsx`                               | Adicionar dismiss options               |
| U21 | 🟡 MEDIUM   | String split hack em consent dialog (`split(':')`)                     | `ConsentDialog.tsx:56-62`                         | Refatorar para keys separadas           |
| U22 | 🟢 LOW      | `KeyBindingRow` 300 linhas dentro de MainWindow                        | `MainWindow.tsx:860-1150`                         | Extrair para arquivo próprio            |
| U23 | 🟢 LOW      | Sem pluralization no i18n                                              | `useI18n.ts`                                      | Adicionar ICU MessageFormat             |
| U24 | 🟢 LOW      | Font size inconsistency (10px-30px sem type scale)                     | Various                                           | Harmonizar type scale                   |
| U25 | 🟢 LOW      | Volume mute button sem hover state                                     | `AudioControl.tsx:83-96`                          | Adicionar `:hover`                      |
| U26 | 🟢 LOW      | Consent dialog sem animation de entrada                                | `ConsentDialog.tsx`                               | Fade-in/scale animation                 |

---

## 7. 📊 Qualidade de Código & Estabilidade

### Pontos Fortes

- **142 testes Rust** cobrindo panic recovery, error serialization, XML injection, ECRAM IOCTL, HMAC auth
- **TypeScript strict mode** com `noUnusedLocals`, `noUnusedParameters`, `noFallthroughCasesInSwitch`
- **ESLint** com `typescript-eslint`, `react-hooks` rules, `no-floating-promises`
- **Panic recovery framework** completo e bem testado (`util/panic.rs`)
- **Logging estruturado** com `fern` + file rotation, targets nomeados
- **Documentação de módulos** com doc comments, research memory detalhado
- **Sem credenciais/secrets hardcoded** — keyring usado corretamente
- **Deadlock analysis** — sem dependências circulares de locks

### Findings

| #   | Severidade  | Finding                                                                                                                                                                                                          | Localização                       | Recomendação                               |
| --- | ----------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------- | ------------------------------------------ |
| C1  | 🔴 CRITICAL | `unwrap()` em `REMAP_STATE.lock()` — 12 sites que NÃO usam `lock_or_recover()`                                                                                                                                   | `hotkeys.rs:2264-2436`            | Substituir por `lock_or_recover()`         |
| C2  | 🔴 CRITICAL | `std::mem::zeroed()` em registry handles — 24 sites, UB se API falha                                                                                                                                             | 10 arquivos `hw/*.rs`             | Usar `MaybeUninit` ou helper seguro        |
| C3  | 🔴 CRITICAL | Raw Input buffer cast sem validação de tamanho mínimo                                                                                                                                                            | `hotkeys.rs`, `touchpad.rs`       | Validar `written >= size_of::<RAWINPUT>()` |
| C4  | 🔴 CRITICAL | 98 blocos `unsafe` sem safety invariant comments                                                                                                                                                                 | 16 arquivos                       | Documentar precondições                    |
| C5  | 🟠 HIGH     | `unwrap()` em `serde_json::to_string(&p)`                                                                                                                                                                        | `discovery.rs:933`                | Usar `.map_err()`                          |
| C6  | 🟠 HIGH     | `unwrap()` em `current_exe()` + `File::open()`                                                                                                                                                                   | `iotservice.rs:1022`              | Tratar erro gracefulmente                  |
| C7  | 🟠 HIGH     | `unwrap()` em `default_window_icon()`                                                                                                                                                                            | `lib.rs:283`                      | Fallback para ícone padrão                 |
| C8  | 🟠 HIGH     | `#[repr(C, packed)]` em IPC structs — UB em ARM                                                                                                                                                                  | `charging.rs:54`, `iotservice.rs` | Documentar ou usar `#[repr(C)]`            |
| C9  | 🟡 MEDIUM   | 25 `#[allow(dead_code)]` — 9 em `ecram.rs`, 6 em `iotservice.rs`                                                                                                                                                 | 5 arquivos                        | Limpar dead code                           |
| C10 | 🟡 MEDIUM   | 50+ clippy warnings (5 correctness, 15 perf, 15 complexity)                                                                                                                                                      | Throughout                        | Address correctness first                  |
| C11 | 🟡 MEDIUM   | Registry DWORD read/write duplicado 7+ vezes                                                                                                                                                                     | 6 arquivos `hw/*.rs`              | Extrair `util/registry.rs`                 |
| C12 | 🟡 MEDIUM   | `hw/osd.rs` (700+ linhas GDI) com ZERO testes                                                                                                                                                                    | `osd.rs`                          | Adicionar testes                           |
| C13 | 🟡 MEDIUM   | Sem React Error Boundary                                                                                                                                                                                         | `App.tsx`                         | Adicionar ErrorBoundary                    |
| C14 | 🟡 MEDIUM   | Sem testes de integração para elevated bridge                                                                                                                                                                    | N/A                               | Adicionar round-trip tests                 |
| C15 | 🟡 MEDIUM   | ~~Sem log rotation — `tauri-dev-trace.log` cresce indefinidamente~~ ✅ **RESOLVIDO (S64)** — `util/log_rotation.rs` (8 MiB × 3) aplicado a `tauri-app.log`, `tauri-dev-trace.log`, `bridge.log` e `watchdog.log` | `debug_log.rs`                    | Adicionar rotation                         |
| C16 | 🟢 LOW      | `Box::leak()` em PDH libraries                                                                                                                                                                                   | `system_info.rs:40,153`           | Aceitável (process lifetime)               |
| C17 | 🟢 LOW      | Empty `useEffect` em TrayPopup                                                                                                                                                                                   | `TrayPopup.tsx`                   | Remover dead code                          |
| C18 | 🟢 LOW      | Sem `CONTRIBUTING.md`                                                                                                                                                                                            | Missing                           | Adicionar                                  |
| C19 | 🟢 LOW      | Sem architecture overview document                                                                                                                                                                               | Missing                           | Adicionar C4 diagram                       |

### Cobertura de Testes

| Área            | Testes  | Qualidade    |
| --------------- | ------- | ------------ |
| `util/panic.rs` | 8       | ✅ Excelente |
| `hw/hotkeys.rs` | ~50+    | ✅ Excelente |
| `hw/errors.rs`  | ~25     | ✅ Bom       |
| `hw/ecram.rs`   | ~15     | ✅ Bom       |
| `hw/battery.rs` | 4       | ✅ Bom       |
| `hw/wifi.rs`    | ~5      | ✅ OK        |
| `hw/osd.rs`     | 0       | ❌ Crítico   |
| Frontend        | 3 files | ⚠️ Limitado  |
| Integration     | 0       | ❌ Faltante  |
| E2E             | 0       | ❌ Faltante  |

---

## 🎯 Plano de Ação Priorizado

### Sprint Imediato — Blockers de GA (CRITICAL)

| #   | Task                                                                        | Esforço  | Domínio      |
| --- | --------------------------------------------------------------------------- | -------- | ------------ |
| 1   | Fix HMAC key bootstrap race com file locking                                | 0.5 dia  | Segurança    |
| 2   | Remover auto-focus do "Allow" no ConsentDialog                              | 0.25 dia | RAI/UX       |
| 3   | Adicionar consent check em `testConnection()`                               | 0.25 dia | RAI          |
| 4   | Corrigir string `storageNote` enganosa (`en.json:541`)                      | 0.1 dia  | RAI          |
| 5   | Implementar focus trap + `:focus-visible` em modais                         | 1 dia    | RAI/UX       |
| 6   | Gerar `latest.json` no release workflow                                     | 0.5 dia  | DevOps       |
| 7   | Hard error se `TAURI_SIGNING_PRIVATE_KEY` vazio                             | 0.25 dia | DevOps       |
| 8   | Substituir `REMAP_STATE.lock().unwrap()` por `lock_or_recover()` (12 sites) | 0.5 dia  | Code Quality |
| 9   | Validar Raw Input buffer size antes do cast                                 | 0.5 dia  | Code Quality |

### Sprint 1 — HIGH Priority

| #   | Task                                                              | Esforço  | Domínio      |
| --- | ----------------------------------------------------------------- | -------- | ------------ |
| 10  | Fix unsafe pointer casts (`hotkeys.rs:1030`, `iotservice.rs:168`) | 1 dia    | Segurança    |
| 11  | Sempre passar `--request-id` ao scheduled task                    | 0.5 dia  | Segurança    |
| 12  | Remover `unsafe-inline` do CSP                                    | 0.5 dia  | Segurança    |
| 13  | Roteirizar `fan.rs`/`processes.rs` via WMI cache                  | 0.5 dia  | Performance  |
| 14  | Adicionar `cargo audit` + `npm audit` em CI                       | 0.5 dia  | DevOps       |
| 15  | Adicionar assinatura Authenticode no release                      | 1 dia    | DevOps       |
| 16  | Adicionar pre-commit hooks (husky + lint-staged)                  | 0.5 dia  | DevOps       |
| 17  | Adicionar "Delete All My Data" + retention policy para logs       | 1 dia    | RAI          |
| 18  | Adicionar Escape key handler em modais                            | 0.25 dia | RAI/UX       |
| 19  | Agrupar sidebar em seções + gate debug tools                      | 1 dia    | UX           |
| 20  | Substituir `std::mem::zeroed()` por `MaybeUninit` (24 sites)      | 2 dias   | Code Quality |
| 21  | Adicionar safety comments em 98 blocos `unsafe`                   | 2 dias   | Code Quality |
| 22  | Adicionar React Error Boundary                                    | 0.5 dia  | Code Quality |

### Sprint 2 — MEDIUM Priority

| #   | Task                                                                | Esforço  | Domínio                 |
| --- | ------------------------------------------------------------------- | -------- | ----------------------- |
| 23  | Migrar `hw/` modules de `anyhow` para `HardwareResult<T>`           | 3-5 dias | Arquitetura             |
| 24  | Implementar polling em tiers (fast 2s + slow 15-30s)                | 0.5 dia  | Performance/Arquitetura |
| 25  | Split `useMemo` em grupos lógicos                                   | 1 dia    | Performance             |
| 26  | Adicionar Sentry/crash reporting                                    | 1 dia    | DevOps                  |
| 27  | Documentar branch protection rules                                  | 0.5 dia  | DevOps                  |
| 28  | Completar traduções faltantes (fr, es, pt)                          | 1 dia    | UX                      |
| 29  | Adicionar `role="alert"` em mensagens dinâmicas                     | 0.25 dia | RAI                     |
| 30  | Extrair `util/registry.rs` helper (eliminar 200+ linhas duplicadas) | 1 dia    | Code Quality            |
| 31  | Adicionar testes para `hw/osd.rs`                                   | 1 dia    | Code Quality            |
| 32  | Adicionar high-contrast mode support                                | 1 dia    | RAI/UX                  |
| 33  | Adicionar retry mechanism para operações flaky                      | 1 dia    | Arquitetura             |

### Backlog — LOW Priority

- Consolidar logging backends (`fern` only)
- Remover dead code (25 `#[allow(dead_code)]`)
- Address 50+ clippy warnings (correctness first)
- Adicionar code splitting no Vite
- Adicionar `CONTRIBUTING.md` + architecture overview
- Adicionar pluralization no i18n
- Adicionar RTL support
- ~~Adicionar log rotation~~ ✅ **RESOLVIDO (S64)** — ver `util/log_rotation.rs`
- Adicionar dependabot/Renovate
- Adicionar code coverage reporting
- Documentar HAL extension pattern
- Adicionar integration tests para elevated bridge

---

## 📈 Métricas de Estabilidade

| Métrica                 | Valor    | Status      |
| ----------------------- | -------- | ----------- |
| Testes Rust             | 142      | ✅          |
| Testes Frontend         | ~15      | ⚠️ Limitado |
| Testes de Integração    | 0        | ❌          |
| Testes E2E              | 0        | ❌          |
| Blocos `unsafe`         | 98       | 🔴 Alto     |
| `unwrap()` em produção  | ~20      | 🟡 Médio    |
| `std::mem::zeroed()`    | 24       | 🔴 Crítico  |
| Clippy warnings         | 50+      | 🟡 Médio    |
| `#[allow(dead_code)]`   | 25       | 🟡 Médio    |
| Commits ahead of master | 19       | ✅          |
| Working tree            | Clean    | ✅          |
| Health checks           | 9/9 pass | ✅          |

---

## Conclusão

O miControl demonstra disciplina de engenharia excepcional para um v0.1.0 — o protocolo HMAC de elevação, o framework de panic recovery, e o cache WMI thread-local são designs de nível production-grade. No entanto, **14 findings CRITICAL** bloqueiam o release para GA:

1. **Segurança**: Race condition no bootstrap da chave HMAC pode quebrar o canal de elevação na primeira execução
2. **RAI/Privacidade**: Dark pattern no consent dialog + bypass de consent no `testConnection()` + GDPR não conformidade
3. **DevOps**: Updater completamente quebrado (sem `latest.json` + chave efêmera)
4. **Code Quality**: 98 blocos `unsafe` sem documentação + `unwrap()` em mutex crítico + `mem::zeroed()` em handles

A correção dos 9 itens do "Sprint Imediato" (esforço estimado: ~4 dias) desbloquearia o GA. Os 13 itens do "Sprint 1" (esforço: ~10 dias) elevariam a aplicação a um padrão de qualidade superior.

---

**Relatório gerado em 2026-06-24 por 7 subagentes especializados (DeepSeek V4 Flash):**

- SE: Security — 14 findings
- SE: Architect — 8 findings
- performance-optimizer — 13 findings
- SE: Responsible AI — 27 findings
- SE: DevOps/CI — 21 findings
- SE: UX Designer — 42 findings
- Explore (Code Quality) — 19+ findings
