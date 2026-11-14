# RELEASE-READINESS — MiControl 0.2.20 (avaliação de pré-release)

**Data:** 2026-11-14 09:34–09:55 UTC · **Branch avaliado:** `f7/pre-release` @ `648e347` · **Worktree:** `C:/Projects/miPC/wt/wt/f7-pre-release`
**Build em produção:** `0.2.20` de 2026-11-14 01:44:18 (instalado 01:42:33 via auth bridge, sem tag nem publish novos)
**Âmbito:** versionamento, gate completo, estado do build/assinatura, riscos e checklist GO/NO-GO. **Nenhuma tag, release ou publish foi criada.** Nenhuma alteração a `AGENTS.md`/`SECURITY.md`.
**Método:** leitura de metadados e do histórico git, gate completo corrido nesta worktree, verificação do binário instalado e do instalador (signtool), logs de produção e Event Log, `gh` para o estado público das releases.

---

## 0. TL;DR

| Métrica                                                     | Resultado                                                                                              |
| ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Versão coerente entre ficheiros                             | **Sim** — `0.2.20` em `package.json`, `tauri.conf.json`, `Cargo.toml`, `Cargo.lock`, gate `version`    |
| Versão coerente com o que está **publicado**                | **Não** — `v0.2.20` já é tag **e** GitHub Release desde 2026-10-25; HEAD está **150 commits** à frente |
| Gate de qualidade                                           | **12/12 ✓** em 8 min 20 s (exit 0)                                                                     |
| `cargo test`                                                | **1 028 passed / 0 failed / 15 ignored**                                                               |
| `vitest`                                                    | **334/334** (40 ficheiros) · cobertura 37,5 % stmts / 73,2 % branches                                  |
| Instalador NSIS                                             | Existe: 123 750 885 B, 2026-11-14 01:42, SHA-256 `2d5cf302…62f82`                                      |
| Authenticode                                                | **NÃO assinado** (`signtool verify /pa` → _No signature found_)                                        |
| Payload do updater (`payload.zip` + `.sig` + `latest.json`) | **Não produzido** (sem `TAURI_SIGNING_PRIVATE_KEY`)                                                    |
| Shell estático (`micontrol_setup.exe`) no bundle            | **0 bytes** (placeholder) — e o instalador instalou-o assim                                            |
| Instalador testado localmente 1×                            | **Sim** — `install_update` 01:42:33 → app 01:44:18                                                     |
| Runtime desde o deploy (≈8 h)                               | **0 WARN · 0 ERROR · 0 crashes · 0 dumps · 18 944 pedidos à bridge, 0 falhas**                         |
| Docs de release actualizadas                                | **Não** (AGENTS.md e SECURITY.md protegidos; CHANGELOG incompleto; 6 docs em falta)                    |
| Rollback disponível                                         | **Não** (o setup anterior foi sobrescrito no mesmo caminho)                                            |
| **Veredicto**                                               | **NO-GO para tag/publish** · GO técnico para uso local continuado                                      |

**Bloqueadores mínimos (5):** (B1) versão reutilizada vs. release público `v0.2.20`; (B2) updater não assinável nesta máquina (password da chave indisponível) → sem payload/sig/latest.json; (B3) shell estático de 0 bytes no bundle; (B4) instalador sem Authenticode e sem certificado no store; (B5) release notes/CHANGELOG por escrever. A estes somam-se duas pendências do dono (docs protegidas) e a ausência de rollback guardado.

---

## 1. Versionamento e metadados

### 1.1 Coerência interna — OK

| Ficheiro                      | Campo                               | Valor                                             |
| ----------------------------- | ----------------------------------- | ------------------------------------------------- |
| `package.json`                | `version`                           | `0.2.20`                                          |
| `src-tauri/tauri.conf.json`   | `version`                           | `0.2.20`                                          |
| `src-tauri/tauri.conf.json`   | `productName` / `identifier`        | `MiControl` / `com.micontrol.app`                 |
| `src-tauri/tauri.conf.json`   | `bundle.targets`                    | `["nsis"]` (perMachine)                           |
| `src-tauri/tauri.conf.json`   | `createUpdaterArtifacts`            | `true`                                            |
| `src-tauri/Cargo.toml`        | `[package] version`                 | `0.2.20`                                          |
| `src-tauri/Cargo.lock`        | pacote `micontrol`                  | `0.2.20`                                          |
| `scripts/publish-release.ps1` | `$Version` default                  | `0.2.20`                                          |
| Nome do instalador            | `MiControl_<version>_x64-setup.exe` | `MiControl_0.2.20_x64-setup.exe` ✓ (contrato S50) |

`pnpm run version:check` → **`✓ All versions agree: 0.2.20`** (passo 1 do gate).

Observação (não é incoerência): as crates auxiliares têm versões próprias — `micontrol-setup` **1.0.0** (shell estático, deliberadamente hash-stable) e `micontrol-facecp` **0.1.0** (DLL do credential provider). Não são a versão do produto e o `sync-version` não as toca, por desenho.

### 1.2 Coerência com o mundo exterior — **FALHA (bloqueador B1)**

```
tag v0.2.20            → 00e8f99, 2026-10-25 21:15 (+0000)  "chore(release): v0.2.20"
HEAD (f7/pre-release)  → 648e347  ...  150 commits à frente, 260 ficheiros, +54 861 / −11 782
GitHub Release         → "MiControl v0.2.20", publicada 2026-10-25 21:40:57Z, Latest, com
                         latest.json, MiControl_0.2.20_payload.zip(+.sig), setup.exe(+.sig), MiControl_Setup.exe
```

O binário que está em produção diz `0.2.20` mas **é outro produto**: contém a campanha completa de auditoria (150 commits) que o `v0.2.20` publicado não tem. Consequências concretas:

- **Não é possível criar a tag** `v0.2.20` no HEAD (já existe noutro commit; movê-la reescreveria história pública).
- **Não é possível publicar** uma release com o mesmo nome/versão.
- **O updater não ofereceria a actualização**: o plugin compara semver e uma instalação em `0.2.20` não vê `0.2.20` como versão superior (só o deploy local via auth bridge, que não compara versões, funcionou).

**Resolução:** bump para **0.2.21** (`pnpm run release patch` ou `pnpm run version:sync 0.2.21`), rebuild, redeploy e só depois tag. O bump é decisão do dono (muda a versão instalada) — **não foi feito**.

### 1.3 CHANGELOG — existe, **não está actualizado** (bloqueador B5)

- `CHANGELOG.md` (452 linhas, Keep a Changelog) tem `## [Unreleased]` com **8 fixes** da vaga 1–2 (fan RPM/percent, `set_fan_mode`, zona ACPI, efeitos de áudio, Voice Clarity, "Set as Default", OSD do micro).
- **Falta tudo o resto da campanha**: protocolo do limite de carga (`WMAA 0x1000/0x02`, códigos de nível, supervisor de hold), listener da tampa via user32, `ui_ok` = renderer WebView2 vivo (P0-1), retry `0x80041002` (P1-1), startup manager, calibração ICC/WCS, tema hairline + i18n 4 locales, event log arquivado, gestor de navegação, overview, silence do log, etc. Verificado por `grep` dos termos no bloco `[Unreleased]`: 0 ocorrências para `WebView2 renderer`, `hold supervisor`, `watchdog`, `ui_ok`, `0x80041002`, `startup manager`, `ICC`, `hairline`.
- `## [0.2.20] - 2026-10-25` descreve o conteúdo do **build de Outubro** (handles, installer decoupling, touchpad HID). É a versão publicada — está correcto como história, mas não descreve o build de 14-Nov.

**Draft de release notes proposto em §7** (não publicado, em inglês para bater com o resto do CHANGELOG).

### 1.4 README

Badge de versão é dinâmico (`img.shields.io/github/v/release/...`) — não hardcodes nada, mostra `0.2.20` enquanto essa for a última release. Não há números de versão escritos à mão no README. **Nada a corrigir.** (O README já documenta `rotation (8 MiB × 7)`, ao contrário do AGENTS.md.)

---

## 2. Gate de qualidade (corrido AGORA nesta worktree)

Comando: `pnpm install --frozen-lockfile && node scripts/health-check.mjs` · **2026-11-14 09:34:15 → 09:42:35 UTC** (8 min 20 s) · **`GATE_EXIT=0`**
Log completo anexado: `C:/Projects/miPC/audit-20261107/health-gate-f7-pre-release.log` (3 241 linhas, 264 KB — junto aos restantes `health-gate-*.log` da campanha). Excerto literal do fecho:

```
▸ (1/12) Version consistency check [version]      ✓ All versions agree: 0.2.20
▸ (2/12) Rust formatting check [fmt]              ✓
▸ (3/12) Rust clippy (warnings denied) [clippy]   ✓
▸ (4/12) Rust tests [test-rust]                   ✓ 1 028 passed; 0 failed; 15 ignored
▸ (5/12) TypeScript type check [tsc]              ✓
▸ (6/12) ESLint [lint]                            ✓
▸ (7/12) Prettier check [format]                  ✓
▸ (8/12) Frontend tests + coverage [test-front]   ✓ 40 files / 334 tests
▸ (9/12) Frontend build [build]                   ✓
▸ (10/12) i18n completeness check [i18n]          ✓ 1 922 keys × 4 locales
▸ (11/12) Kernel-handle leak gate (S67) [handle-leaks]  ✓ 34 sites, all RAII
▸ (12/12) Toolchain consistency check [toolchain] ✓
✓ Health check passed — 12 check(s).   GATE_EXIT=0
```

| #   | Passo                                                          | Resultado | Evidência                                                                        |
| --- | -------------------------------------------------------------- | --------- | -------------------------------------------------------------------------------- |
| 1   | `version` — Version consistency                                | ✓         | `All versions agree: 0.2.20`                                                     |
| 2   | `fmt` — `cargo fmt --check`                                    | ✓         | —                                                                                |
| 3   | `clippy` — `--all-targets --features face -- -D warnings`      | ✓         | sem warnings                                                                     |
| 4   | `test-rust` — `cargo test --features face -- --test-threads=1` | ✓         | **1 028 passed · 0 failed · 15 ignored**                                         |
| 5   | `tsc` — `tsc --noEmit`                                         | ✓         | 0 erros                                                                          |
| 6   | `lint` — ESLint                                                | ✓         | —                                                                                |
| 7   | `format` — Prettier check                                      | ✓         | —                                                                                |
| 8   | `test-front` — `vitest run --coverage`                         | ✓         | **40 ficheiros / 334 testes** · 37,5 % stmts · 73,18 % branches                  |
| 9   | `build` — Vite build                                           | ✓         | `dist/` gerado (chunk principal 733,94 kB / 244,96 kB gzip)                      |
| 10  | `i18n` — completude                                            | ✓         | **1 922 chaves × 4 locales** (en/pt/es/fr), sem vazios nem drift de placeholders |
| 11  | `handle-leaks` — S67                                           | ✓         | **34 acquisition sites, todos com RAII**                                         |
| 12  | `toolchain`                                                    | ✓         | Node 24.15.0 = `.nvmrc`                                                          |

Detalhe do passo 4 (alvos): `micontrol_lib` 958 · `ecram_service` 16 · `micontrol` 0 · `micontrol_bridge` 25 · `micontrol_face_svc` 0 · `tests/elevated_bridge` 23 · `tests/face_pipe` 4 · `tests/logging_e2e` 1 · `tests/pipe_dacl_fix` 1 · `tests/test_perf_mode` 0 (1 ignored). Os 15 `#[ignore]` são os testes live/hardware conhecidos (BLE, ICC real, pipe IoT instalado, event log, Phone Link, startup HKLM, STA WMI).

Nota: o gate não constrói o instalador NSIS nem assina nada — valida código, testes e frontend. O build de release é um passo separado (§3).

---

## 3. Estado do build, instalador e assinatura

### 3.1 Instalador

| Propriedade  | Valor                                                                                                 |
| ------------ | ----------------------------------------------------------------------------------------------------- |
| Caminho      | `C:/Projects/miPC/wt/integration/src-tauri/target/release/bundle/nsis/MiControl_0.2.20_x64-setup.exe` |
| Tamanho      | 123 750 885 B (118,02 MiB)                                                                            |
| mtime        | 2026-11-14 01:42                                                                                      |
| SHA-256      | `2d5cf30273748ffdb8cba40f1820475ded5fa431238d938372272d21c7362f82`                                    |
| Fonte        | worktree `wt/integration` @ **`648e347`** — exactamente o HEAD de `f7/pre-release` ✓                  |
| Log de build | `audit-20261107/build-nsis-recheck2.log` (termina em `BUILD_EXIT=1`)                                  |

`BUILD_EXIT=1` **é esperado e não é falha de compilação**: o Tauri produziu o bundle NSIS e só falhou no passo de assinatura do updater (`A public key has been found, but no private key. Make sure to set TAURI_SIGNING_PRIVATE_KEY`). O instalador está completo.

### 3.2 Authenticode — **NÃO assinado** (bloqueador B4)

```
signtool verify /pa /v MiControl_0.2.20_x64-setup.exe
→ SignTool Error: No signature found.   (0 ficheiros verificados, 1 erro)
```

- `signtool` disponível: `C:/Program Files (x86)/Windows Kits/10/bin/10.0.26100.0/x64/signtool.exe`.
- O `tauri.conf.json` declara `certificateThumbprint 4678FB4266F446C72CF4A181D6A348B9EE9C9661` e `timestampUrl http://timestamp.digicert.com`, mas **esse certificado não está no store** (`Cert:\CurrentUser\My` + `Cert:\LocalMachine\My` → não encontrado; **0** certificados de code signing em qualquer store).
- Efeito no utilizador: SmartScreen "Unknown publisher", bloqueio em ambientes geridos, e nenhuma integridade verificável do instalador.

**Caminho para resolver:** obter/instalar o certificado (OV ou EV) com o thumbprint configurado (ou actualizar o thumbprint), garantir a chave privada no store do utilizador que faz o build, e correr `signtool sign /sha1 <thumbprint> /fd sha256 /tr http://timestamp.digicert.com /td sha256` — o `tauri build` fá-lo automaticamente quando o thumbprint resolve. Para o updater (separado do Authenticode): `TAURI_SIGNING_PRIVATE_KEY` + `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

### 3.3 Payload do updater — **ausente** (bloqueador B2)

- `src-tauri/target/release/` **não tem** `*.nsis.zip`, `*.sig` nem `latest.json` (procura em toda a árvore `C:/Projects/miPC` sem resultados para o build de Novembro).
- Motivo: `createUpdaterArtifacts: true` mas a chave privada não foi carregada no build.
- **A chave existe nesta máquina**: `~/.tauri/micontrol-new.key` (privada, 348 B) + `micontrol-new.key.pub`, cuja pubkey é **exactamente** a configurada em `tauri.conf.json` (`minisign key id 828882725C62229A`).
- **Mas está protegida por password e a password não está disponível** — testado: `tauri signer sign --private-key-path ~/.tauri/micontrol-new.key --password ""` → `incorrect updater private key password`. (Há também `micontrol-key.pub` e `micontrol.key.pub` de gerações antigas, que **não** correspondem à pubkey configurada — não usar.)
- Enquanto isto não for resolvido, **o caminho de update público não existe**: o `latest.json` no GitHub aponta para o build de 2026-10-25.

### 3.4 Shell estático de 0 bytes no bundle (bloqueador B3)

O log de build diz explicitamente:

```
warning: micontrol@0.2.20: setup shell not found at ...\target\shell-release\micontrol_setup.exe
         — temporary placeholder created for resource validation
```

E a máquina tem `C:\Program Files\MiControl\setup\micontrol_setup.exe` com **0 bytes** (2026-11-07 12:07). Consequências: a actualização por payload (S65) não tem shell local; só funciona pelo _bootstrap_ que descarrega o shell dos assets da release (dependente de rede).

**Causa provada (ordem de operações), não um mistério:**

1. `scripts/publish-release.ps1` corre `pnpm exec tauri build` (linha ~57) **antes** de `cargo build -p micontrol-setup --profile shell-release` (linha ~74). O `build.rs` procura o shell em `target/shell-release/` no momento em que o bundle é montado → não existe → placeholder de 0 bytes.
2. O mesmo erro de ordem existe no `release.yml`: `pnpm run tauri build` (job step anterior) e só depois "Build static setup shell".
3. Bónus: o passo CI verifica `src-tauri/target/release/shell-release/micontrol_setup.exe`, mas o cargo escreve em **`src-tauri/target/shell-release/`** — provado localmente (`cargo build -p micontrol-setup --profile shell-release` → `target/shell-release/micontrol_setup.exe`, 2 672 640 B). O passo CI falharia com "shell binary not found".

**Prova do fix (feita nesta worktree, sem rebuild do bundle):** depois de construir o shell, um simples `cargo check` copiou-o para `src-tauri/bin/micontrol_setup.exe` (**2 672 640 B**, antes 0 B). Ou seja, basta **construir o shell antes do `tauri build`** para o bundle o incluir. Não refiz o bundle (não é necessário para o veredicto e evita reescrever o artefacto documentado).

### 3.5 Pipeline de release — nunca completou

`gh run list --workflow=release.yml`:

| Tag            | Conclusão                                                             | Duração   |
| -------------- | --------------------------------------------------------------------- | --------- |
| `v0.2.20`      | **cancelled** (Health checks cancelado; Build & publish com 0 passos) | 45 s      |
| `v0.2.19-beta` | **failure**                                                           | 14 m 39 s |
| `v0.2.18-beta` | cancelled                                                             | 1 m 25 s  |
| `v0.2.17-beta` | cancelled                                                             | 2 m 39 s  |

A release `v0.2.20` publicada foi portanto produzida **localmente** (`publish-release.ps1`), não pelo CI. Isto agrava o critério "build reproduzível": o caminho documentado em `docs/release.md` não tem uma execução verde recente.

### 3.6 Deploy e estado em produção (8 h)

| Facto             | Evidência                                                                                               |
| ----------------- | ------------------------------------------------------------------------------------------------------- |
| Instalação        | `install_update` despachado 2026-11-14T01:42:33 (`tauri-app-helper.log`)                                |
| Binário instalado | `micontrol.exe` 134 454 272 B, mtime 01:39:12, `FileVersion/ProductVersion 0.2.20`                      |
| Arranque          | `role=app` 2026-11-14T01:44:18.225Z (pid 18804)                                                         |
| Contém o fix P0-1 | `grep -a "WebView2 renderer is gone"` no binário instalado → **1** ✓                                    |
| Log da sessão     | 524 linhas até 09:36, **0 WARN, 0 ERROR**                                                               |
| Bridge            | **18 944 pedidos, 0 `ok=false`** desde 01:44 (`bridge.log.1` a partir de 01:44 + `bridge.log`)          |
| Crashes/dumps     | Event 1000/1001/7031/7034 da MiControl: **0** · `CrashDumps`: **0** · 7036 de serviços MiControl: **0** |
| Serviços          | `MiControlBridge`, `MiControlFace`, `IoTSvc` = Running/Automatic                                        |
| Tarefa elevada    | `MiControlElevated` = Ready, último resultado **0** (09:23:30)                                          |
| Firewall          | **7** regras (6 allow + 1 block public), todas activas                                                  |
| Watchdog          | 1 "heartbeat stale — confirmation 1/2" no arranque (01:44:21), sem relaunch                             |

---

## 4. Inventário de riscos e pendências

### 4.1 Bloqueadores de release (detalhe em §1–§3)

| ID  | Item                                                                                 | Sev            | Evidência                                                                              | Dono da acção            |
| --- | ------------------------------------------------------------------------------------ | -------------- | -------------------------------------------------------------------------------------- | ------------------------ |
| B1  | Versão `0.2.20` reutilizada (tag + release públicos desde 25-Out; HEAD +150 commits) | **P0 release** | `git log v0.2.20..HEAD` = 150; `gh release view v0.2.20` = Latest                      | Dono (decisão) + bump    |
| B2  | Updater sem payload/sig/latest.json; password da chave privada indisponível          | **P0 release** | `BUILD_EXIT=1`; `tauri signer sign` → wrong password                                   | Dono (segredo)           |
| B3  | Shell estático 0 B no bundle e em `Program Files`                                    | **P1 release** | warning do build; `ls setup/micontrol_setup.exe` = 0 B; ordem do `publish-release.ps1` | Engenharia (reordenar)   |
| B4  | Instalador sem Authenticode; certificado ausente do store                            | **P1 release** | `signtool verify` = No signature; 0 certs                                              | Dono (certificado)       |
| B5  | CHANGELOG/release notes não descrevem o build                                        | **P1 release** | §1.3                                                                                   | Engenharia (draft em §7) |
| B6  | Docs protegidas desactualizadas                                                      | **P2 release** | ver 4.4                                                                                | **Dono** (protegidas)    |
| B7  | Sem rollback: setup anterior sobrescrito                                             | **P1 release** | só existe `dist_b12/MiControl_0.1.18` (Ago) e o actual                                 | Engenharia               |

### 4.2 Riscos técnicos aceites / escalados (não bloqueiam uso local)

| #   | Item                                                                                                 | Estado                           | Nota para a release                                                                                                         |
| --- | ---------------------------------------------------------------------------------------------------- | -------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| R1  | `runtime starved` **1×/53 h** sob stall do provider WMI da bateria (129 s)                           | **ESCALADO** (provider, não app) | 680 → 1 ERROR por sessão; recupera sozinho; resolver exige outra fonte de bateria                                           |
| R2  | Relaunch do watchdog sem cobertura de teste (espera pelo `:4000`)                                    | Aberto (herdado)                 | 0 relaunches em 53 h + 8 h ⇒ caminho não exercitado; teste de bancada                                                       |
| R3  | Tampa: `[lid] CLOSED` **nunca observado** em campo                                                   | Parcialmente provado             | Inscrição user32 OK (sem erro 87), `[lid] OPEN` no arranque; falta fechar a tampa uma vez                                   |
| R4  | Limite de carga: plateau **74–75 %** com limite 80 %                                                 | **Explicado, não é bug**         | 80 % da capacidade de **projecto** ≈ 74,6 % da capacidade **actual** (saúde 93,3 %); `applied=true`, plateau observado 39 h |
| R5  | `performance::set_performance_mode` ainda escreve `WMAA 5..8` (no-op: SMMT/SMMD)                     | Dívida D2                        | A UI já não mente (`EcModeReadout` + `FanModeResult.warning`), mas o ruído no EC e o caminho optimista continuam            |
| R6  | `scenario_rules::apply_lid_action()` **sem chamadores** — a opção "ação da tampa" da UI não faz nada | Dívida D1                        | Ligar muda comportamento de sistema (sleep): decisão do dono                                                                |
| R7  | `chargePhase` com `limitApplied:null` mostra "holding"                                               | Dívida D6                        | Janela de 45 s até o helper responder                                                                                       |
| R8  | Componentes órfãos: `ChargingThreshold.tsx`, `SystemInfoCard.tsx`, `SetupTab`                        | Dívida D3                        | Caminho de UI morto, sem impacto funcional                                                                                  |
| R9  | `get_fan_info` (2 s) + `get_ec_active_mode` (3 s) = ~20 chamadas/min à bridge com a UI aberta        | Aceitável (38 ms)                | Carga nova do fix de perf-mode                                                                                              |

### 4.3 Limitações honestas a manter nas notas (não são defeitos)

- **RPM da ventoinha**: não há tacómetro legível neste host (`Win32_Fan.DesiredSpeed` vazio). A UI mostra `—`, nunca 0 RPM.
- **TOPS da NPU**: o Windows expõe o dispositivo (`ComputeAccelerator`) mas não os TOPS. Mostra `—`.
- **Temperatura do SSD**: `StorageReliabilityCounter` exige elevação; indisponível fora do contexto elevado.
- **Descarga da bateria para o limite**: **não existe canal de descarga** neste firmware (provado: um único caso `FUN3=0x02` na árvore de escrita WMAA; `IoTDriver.sys` com 2 IOCTLs; `IoTService.exe` original sem comandos de carga). O limite impede carregar mais; não descarrega um pack acima do limite.
- **Modo activo da EC vs. modo pedido**: o registry pode dizer `Balance` com a EC em `QFAN=9` (SmartAdaptive); a UI mostra ambos.
- **Curva de ventoinha com a tampa fechada**: é o firmware (tabela clamshell), não a app; potência/temperatura são idênticas com a tampa aberta.

### 4.4 Documentação pendente

| Ficheiro                              | Problema                                                                                                                                                                                                                                                             | Acção                                                             |
| ------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| `AGENTS.md` (protegido)               | "8 MiB per file, **3** rotated files kept" → o código usa **7** (confirmado pelo banner `rotation: 8 MiB x 7 files`); "**18** lazy-loaded tabs" → são **24** (`React.lazy` em `src/pages/MainWindow.tsx`); falta a linha de `tauri-app-helper.log` na tabela de logs | **Dono**                                                          |
| `SECURITY.md` (protegido)             | "Supported: **1.0.x**", "< 1.0 não suportado" — a versão actual é 0.2.20                                                                                                                                                                                             | **Dono**                                                          |
| `docs/HARDWARE_INVESTIGATION.md`      | auto-intitula-se "✅ SINGLE SOURCE OF TRUTH — Last updated 2026-06-30", em conflito com `architecture.md`, `RE_ANALYSIS_REPORT.md` e `LOGGING.md`                                                                                                                    | Dono/engenharia: rebaixar para "consolidated findings (snapshot)" |
| `CHANGELOG.md`                        | `[Unreleased]` cobre 8 fixes de 1 de 6 vagas; `[0.2.20]` descreve o build de Outubro                                                                                                                                                                                 | Engenharia (draft §7)                                             |
| `docs/release.md`                     | descreve um pipeline CI que nunca completou; não menciona que o shell tem de ser construído **antes** do bundle                                                                                                                                                      | Engenharia                                                        |
| Docs em falta (DOCS-AUDIT 2026-11-11) | `startup-manager.md`, `color-calibration.md`, `os-turbo.md`, `face-unlock.md` (operacional), `cross-device.md`, `elevated-bridge.md`                                                                                                                                 | Engenharia                                                        |
| Links quebrados históricos            | 11, todos em `docs/deprecated/` e `sprint-planning/` (não corrigidos de propósito)                                                                                                                                                                                   | Aceite                                                            |

### 4.5 Segurança / privacidade

- `consent_audit.log` selado por HMAC (chave com rotação), `elev_key.bin` 32 B user-readable, protocolo de comando assinado v2 (ficheiros) / v3 (pipe) com nonce e frescura ±30 s — verificado em código e em produção (18 944 pedidos sem falha).
- Socket MCP (`127.0.0.1:4000`) existe em produção; é opt-in (`MCPIntegrationEnabled`, off por omissão) — decisão do dono se fica.
- Sentry: CSP restringe `connect-src` a `sentry.io`/GitHub; consent-gated.

---

## 5. Checklist GO/NO-GO

| #   | Critério                                        | Veredicto                   | Evidência                                                                                         |
| --- | ----------------------------------------------- | --------------------------- | ------------------------------------------------------------------------------------------------- |
| 1   | Versão coerente em todos os ficheiros           | **✓ PASSA**                 | `version:check` OK; 5 ficheiros a `0.2.20`                                                        |
| 1b  | Versão utilizável para uma nova tag/release     | **✗ FALHA**                 | `v0.2.20` já é tag + release pública; +150 commits                                                |
| 2   | Gate 12/12 verde                                | **✓ PASSA**                 | 12/12, exit 0, 8 min 20 s, log anexado                                                            |
| 3   | Build reproduzível a partir do HEAD             | **⚠ PARCIAL**               | instalador de `648e347` (= HEAD) ✓, mas `BUILD_EXIT=1` no signing e CI de release nunca completou |
| 4   | Instalador testado localmente 1×                | **✓ PASSA**                 | `install_update` 01:42:33 → app 01:44:18 → 8 h limpas                                             |
| 5   | Updater assinável (payload + sig + latest.json) | **✗ FALHA**                 | sem artefactos; password da chave indisponível                                                    |
| 6   | Docs actualizadas para a release                | **✗ FALHA**                 | AGENTS.md, SECURITY.md, HARDWARE_INVESTIGATION, CHANGELOG, 6 docs em falta                        |
| 7   | Sem P0/P1 abertos                               | **✓ PASSA (com ressalvas)** | P0-1 e P1-1 corrigidos **e** no binário instalado; R1 escalado (P2), R2 herdado                   |
| 8   | Rollback possível (setup anterior guardado)     | **✗ FALHA**                 | anterior sobrescrito; só 0.1.18 de Agosto                                                         |
| 9   | Firewall/UAC limpos                             | **✓ PASSA**                 | 7 regras activas; tarefa elevada Ready/resultado 0; deploy sem UAC                                |
| 10  | i18n 4 locales completos                        | **✓ PASSA**                 | 1 922 chaves × en/pt/es/fr; gate i18n OK                                                          |
| 11  | Instalador assinado (Authenticode)              | **✗ FALHA**                 | _No signature found_; 0 certs no store                                                            |

**Veredicto global: NO-GO para tag/publish.** Cinco critérios falham (1b, 5, 6, 8, 11) e um é parcial (3). O produto em si está saudável — 12/12 no gate, 8 h sem um único WARN em produção, P0/P1 fechados e verificados no binário — mas **o pacote de release não está fechável**: a versão colide com uma release pública, o updater não é assinável nesta máquina, o instalador não está assinado, o shell vai a 0 bytes e não há rollback guardado.

**Para uso local continuado (o que está instalado): GO.** A build de 01:44 é a melhor que a campanha produziu.

---

## 6. Lista mínima de bloqueadores para chegar a "PRONTO PARA TAG LOCAL"

Por ordem de execução (cada passo desbloqueia o seguinte):

1. **B1 — Bump de versão para 0.2.21** (decisão do dono). `pnpm run release patch` (ou `pnpm run version:sync 0.2.21`), confirmar `pnpm run version:check`, e actualizar o `## [Unreleased]` do CHANGELOG para `## [0.2.21] - 2026-11-14` com as notas de §7.
2. **B3 — Construir o shell antes do bundle.** `cargo build -p micontrol-setup --profile shell-release --manifest-path src-tauri/Cargo.toml` **antes** de `pnpm run tauri build -f face`; corrigir a ordem no `publish-release.ps1` e no `release.yml`, e o path do check CI para `src-tauri/target/shell-release/micontrol_setup.exe`.
3. **B2 — Assinar o updater.** Exportar `TAURI_SIGNING_PRIVATE_KEY` (conteúdo de `~/.tauri/micontrol-new.key`) e `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (só o dono a tem), correr o build e confirmar `MiControl_0.2.21_payload.zip` + `.sig` + `latest.json`. Validar que a pubkey do `latest.json` corresponde à configurada.
4. **B4 — Authenticode.** Instalar o certificado com o thumbprint `4678FB…9661` (ou actualizar o thumbprint no `tauri.conf.json`) e assinar o setup; confirmar com `signtool verify /pa`.
5. **B7 — Guardar o rollback.** Copiar o installer actual para `dist_b12/` (ou pasta de releases locais) **antes** do próximo deploy, com o SHA-256 registado.
6. **B5/B6 — Notas e docs.** Publicar as notas de §7 no CHANGELOG; o dono corrige `AGENTS.md` (3→7, 18→24, linha do helper log) e `SECURITY.md` (1.0.x→0.2.21).

Só depois: `git tag v0.2.21` + release (decisão do dono, fora do âmbito desta avaliação).

---

## 7. Draft de release notes (NÃO publicado)

Proposta para `CHANGELOG.md` / corpo da release. Inglês para bater com o resto do ficheiro.

```markdown
## [0.2.21] - 2026-11-14

Stability and honesty release: five audit waves over the 0.2.20 build, 87 fixes verified
one by one, and a new hairline UI. Nothing here promises hardware behaviour the machine
cannot prove.

### Added

- **Windows startup manager** — Run keys, Startup folders and logon scheduled tasks in one
  place. Enable state follows Task Manager semantics (`StartupApproved` blobs, never a
  delete), machine-scope writes go through the elevated whitelist, and auto-start services
  stay out of the list by design.
- **Colour calibration** — panel detected by EDID, validated ICC catalogue, staged install.
  Uses the WCS association API because `mscms.dll` on Windows 11 26200 no longer exports
  `SetICMProfileW`/`GetICMProfileW`.
- **Overview** — full machine summary: specifications, NPU, temperatures, battery.
- **Navigation** — five sidebar groups, user glossary, accessibility P0 pass on the task
  manager.
- **Edge Slide guide** — accordion with the touch zone diagram, numbered steps, what does
  NOT trigger it and a troubleshooting line, inside the feature card.
- **i18n** — Face Unlock and IoT surfaces translated; 1 922 keys complete in en/pt/es/fr.
- **Theme** — hairline token set, 11 primitives, SVG icon system (zero emojis in the UI).

### Changed

- **Charging limit is applied through ACPI `WMAA` (`0x1000/0x02`)** with the OEM level
  codes (not a percentage), and re-asserted at startup, on resume and on AC plug — the EC
  clears the register on every power-source transition. The card now reports the real
  phase (`holding`/`above_limit`/`charging`/`drifted`) and **never promises a discharge**:
  this firmware has no discharge channel.
- **Performance mode tells the truth** — the EC's active mode (`QFAN`) is read back and
  shown next to the requested mode; a write that does not latch is no longer reported as
  applied.
- **Fan card** — RPM is `null` when there is no tachometer (and rendered as `—`), the
  requested percentage travels separately, and `set_fan_mode` reports what it actually did.
- **Lid** — state comes from `GUID_LIDSWITCH_STATE_CHANGE` via `user32`, logs every
  transition with a thermal snapshot, and fails loudly if registration fails.

### Fixed

- **UI could die silently and the watchdog never noticed (P0).** `ui_ok` stayed 1 after the
  WebView2 renderer was gone; it now requires a live `msedgewebview2.exe` child, so the
  watchdog restarts a zombie in under two minutes.
- **A transient WMI provider stall (`0x80041002`) triggered a second full elevation.** The
  retry now lives inside `wmi_read` (3 attempts, 200/400 ms, provider-stall only).
- **Charge-hold supervisor** re-asserts the limit on AC/resume instead of waiting for the
  5-minute timer, and the plateau is read in the right base (80 % of design capacity).
- **Kernel-handle leak** — all 34 handle acquisition sites own their handles through RAII;
  handle growth is 0/min.
- **Log volume** — battery slow-query, BLE samples, raw hotkey events, adapter probes and
  the bridge health probe are windowed (1/10 min) and report suppressed counts.
- **Bridge** — `elev_key.bin` lock contention no longer fails a command; a late but
  authentic response is no longer rejected as stale; 64 concurrent eCRAM clients.
- **Audio effects** were written into the SYSTEM hive and always read back "off"; setters
  now run in-process, and legacy stranded values migrate once.
- **Fan/ACPI honesty** — the ambient zone is no longer presented as the CPU temperature;
  voice-clarity detection probes the OS build + NPU instead of a key that does not exist;
  "Set as Default" is wired; the mic OSD reads the endpoint's real mute state.

### Known limitations

- Fan RPM, NPU TOPS and SSD temperature are not readable on this hardware and render as `—`.
- There is no battery discharge channel: a pack above the limit stays there.
- The WMI battery provider can stall for up to ~2 minutes; one `runtime starved` warning per
  50 h is expected and self-recovers.
- The lid-closed fan ramp is the EC's clamshell fan table, not application behaviour.
```

---

## 8. Anexos e evidência

| Artefacto                             | Caminho                                                                                                                                                                                                                   |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Log do gate (12 passos, 3 241 linhas) | `C:/Projects/miPC/audit-20261107/health-gate-f7-pre-release.log` (run de 09:34:15–09:42:35)                                                                                                                               |
| Log do build NSIS                     | `audit-20261107/build-nsis-recheck2.log`                                                                                                                                                                                  |
| Relatórios da campanha                | `audit-20261107/{RECHECK-2,RECHECK-LOGS,FIX-REVIEW,STAB-ROUND-1}.md` (no repo) e `C:/Projects/miPC/audit-20261107/{AUDIT,POST-FIX,FUNC-FIX,WAVE4,PERF-MODE-DEBUG,LID-FAN-DEBUG,BATTERY-LIMIT-DEBUG,CHARGE-HOLD-DEBUG}.md` |
| Auditoria de docs                     | `docs/DOCS-AUDIT-2026-11-11.md`                                                                                                                                                                                           |
| Instalador                            | `wt/integration/src-tauri/target/release/bundle/nsis/MiControl_0.2.20_x64-setup.exe` (SHA-256 em §3.1)                                                                                                                    |

Comandos de reprodução:

```bash
# gate completo (worktree f7/pre-release)
pnpm install --frozen-lockfile && node scripts/health-check.mjs

# coerência de versão e i18n
pnpm run version:check && pnpm run i18n:check

# assinatura do instalador
"/c/Program Files (x86)/Windows Kits/10/bin/10.0.26100.0/x64/signtool.exe" verify /pa /v \
  "C:/Projects/miPC/wt/integration/src-tauri/target/release/bundle/nsis/MiControl_0.2.20_x64-setup.exe"

# shell estático (tem de existir ANTES do tauri build)
cargo build -p micontrol-setup --profile shell-release --manifest-path src-tauri/Cargo.toml
```


---

# ACTUALIZAÇÃO PÓS-AVALIAÇÃO (14-Nov 11:06)

## B3 — RESOLVIDO E PROVADO
- **Build-order**: `scripts/publish-release.ps1` e `.github/workflows/release.yml` passam a construir o shell estático **antes** do `tauri build`; o path do CI foi corrigido (`target/shell-release`, não `target/release/shell-release`).
- **Causa raiz do setup a 0 bytes desde 07-Nov**: o par `.new`+`Rename` do POSTINSTALL falhava em silêncio em todos os deploys. Substituído por **delete-then-copy com 3 retries + log de cada passo** em `$INSTDIR\setup\install-hook.log`.
- **Auto-cura sem rede**: o bootstrap da app copia a cópia raiz do instalador (`$INSTDIR\micontrol_setup.exe`) quando a shell instalada é inválida, antes de tentar o download do GitHub.
- **Prova no deploy de 14-Nov 11:06**: `install-hook.log` = `root size: 2672640 · attempt 1 · final size: 2672640 · RESULT: deployed`; `setup\micontrol_setup.exe` = 2 672 640 B (era 0 desde 07-Nov). O caminho de updates desacoplado está finalmente funcional.

## Estado actualizado dos bloqueadores
| # | Estado |
|---|---|
| B1 bump 0.2.21 | **aberto — decisão do dono** |
| B2 senha da chave do updater | **aberto — dono** (chave existe em ~/.tauri/micontrol-new.key; password indisponível) |
| B3 shell 0 bytes | **FECHADO** (deploy 11:06, provado) |
| B4 Authenticode | **aberto — dono** (0 certificados de code-signing em qualquer store) |
| B5 release notes | **aberto** (draft em §7; actualizar com as vagas 4b–7) |
| B6 AGENTS.md/SECURITY.md | **aberto — protegido, só o dono** |
| B7 rollback | **aberto** (guardar o setup actual como `MiControl_0.2.20_prev_x64-setup.exe` antes do próximo deploy) |
