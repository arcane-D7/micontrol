# RECHECK-2 — auditoria pós-deploy 11-Nov 19:16 → 14-Nov 01:15

**Âmbito:** reverificar em produção os fixes do último deploy (charge-hold, listener de tampa,
reconciliador, higiene de logs, timeout de elevação) e **resolver** os problemas encontrados.
**Janela:** 53 h 20 m de runtime contínuo (uma sessão, pid 57060, sem relaunch), desde o banner
`role=app` de `2026-11-11T19:16:07.578Z`.
**Deploy auditado:** binário `C:\Program Files\MiControl\micontrol.exe` mtime **11-Nov 19:12:12**,
128,24 MB; app arrancou 19:16:07; `MiControlBridge` **não** foi reiniciado (continuou a servir).
**Branch:** `f6/recheck-fix` (worktree `wt/wt/f6-recheck-fix`), base `73c8745`.
**Modo:** leitura nos logs + sondagem IPC read-only + fixes de código com testes no branch.

---

## 1. Sumário executivo

| Métrica | Valor |
|---|---|
| Linhas de log da app na janela | 9 972 (9 672 INFO · 285 WARN · **2 ERROR**) |
| Taxa | 187 linhas/h (47/h sem `ble_presence`+`hotkeys`) |
| WARN/h | 5,3 |
| `runtime starved` (ERROR) | **1** (00:53:50, auto-recuperado) — a baseline desta ronda era 0 |
| Relaunches do watchdog | **0** (log do watchdog sem uma única linha desde 19:15:46) |
| Crashes nativos (Event 1000) / dumps | **0** / **0** |
| Pedidos à bridge | 54 006, **0 `ok=false`** (bridge.log.2+.1+log) |
| Helpers órfãos | **0** |
| Handles da app | 1 195 → **1 128** em 53 h (sem leak) |
| `[lid]` | **2** linhas (OPEN + snapshot no arranque) — inscrição OK, sem erro 87 |
| `[charge_hold]` | 51 transições, **hold observado** (74-75 % durante 2 dias com o carregador ligado) |
| Event 7031/7034 da MiControl | **0** |
| Elevação | 1 716 execuções, **129 falhas 0x80041002 (7,5 %)** e **0 desde 13-Nov 00:25** (passou a usar a pipe) |

**Contagem por severidade desta ronda: P0 = 1 · P1 = 1 · P2 = 14** (16 achados: **11 corrigidos**
no branch por 10 fixes — P2-9 vai dentro do fix do P1-1 —, **1 escalado**, **3 aceites** com
justificação e **1 documentado** como não-bug).

**Veredicto:** o deploy de 19:16 cumpriu **todos** os fixes que se propôs (tampa, charge-hold,
reconciliador, timeout de 15 s, higiene) e a máquina esteve 53 h sem um único relaunch — mas a
sessão terminou com **a UI morta**: o renderer WebView2 desapareceu e o `ui_ok` continuou a 1, por
isso o watchdog nunca reiniciou o zombie. Esse é o P0, e está corrigido (P0-1).

---

## 2. Verificação fix-a-fix do último deploy

| # | Fix | Veredicto | Evidência |
|---|---|---|---|
| a | Supervisor de carga (`[charge_hold]`, 45 s tick, verify 5 min) | **OK** | 51 transições: `holding`, `above limit`, `idle`, `charging`, `on battery`, `unknown→after verify`. Uma reaplicação: `[charge_limit] re-applied 80% on hold (was Some(100)%, read-back Some(80)%)` (12-Nov 09:40:35) — imediatamente a seguir ao resume. |
| b | `reapply` em resume / AC-plug | **OK (por via do supervisor)** | Resume às 09:40:34.563 (após 8 h 51 m de S3); o re-apply saiu 1,2 s depois com trigger `hold` (o caminho `on_resume` foi coalescido pelo `ReconcileGate`). A partir daí o pack ficou **parado em 74-75 %** com `plugged=true, charging=false` durante 39 h — o primeiro plateau observado desde que o limite foi introduzido. |
| c | Banda 78-80 | **OK (leitura)** mas o plateau real é 74-75 % | `80 % × 93,3 % de saúde = 74,6 %` — o EC limita **80 % da capacidade de projecto**, o Windows reporta % da capacidade actual. Não é bug; é a mesma grandeza em bases diferentes. Registado abaixo (P2-13). |
| d | Listener de tampa via user32 | **OK (inscrição), não exercitado no fecho** | Nenhum `registration failed: Win32 error 87`; `[lid] OPEN (previous unknown)` + snapshot no arranque (19:16:09). **Zero `[lid] CLOSED`** porque o dono não fechou a tampa na janela. |
| e | Reconciliador de carga (5 min + 60 s em drift/unknown) | **OK** | `get_charge_limit_state` continua a ser chamado (último às 01:19:26, 29 ms) e responde `requested=80 hardware=80 code=1 applied=true source=firmware` (medido ao vivo pela pipe, §5). |
| f | Higiene: slow-query a DEBUG, ERAM 1×, sonda bridge só em mudança | **PARCIAL** | ERAM `DriverBlocked` = **0** linhas (subiu a INFO, ok). Slow-query caiu de 1 438/19,5 h para **811/53 h**, mas a regra "1 por episódio" não funcionou (ver P2-1). A sonda da bridge **piorou**: 5 429 linhas (P2-5). |
| g | `elev` timeout (deadline de 15 s) | **OK** | 3 abortos, todos **exactamente** no limite: `exceeded its total budget of 15 s (stuck in 'result poll' / 'startup' / 'scheduled-task dispatch')`. O caso de 124 s desapareceu. |
| h | Watchdog / relaunch race | **Não exercitado** | 0 relaunches em 53 h ⇒ o caminho do `:4000` continua sem cobertura (herdado). |

---

## 3. Achados

| ID | Sev | Módulo | Achado | Ocorr. | Estado |
|---|---|---|---|---|---|
| P0-1 | **P0** | `hw::crash_recovery` / `tauri_runtime_wry` | WebView2 morto sem detecção: UI zombie, `ui_ok=1`, watchdog nunca reinicia | 1 (detectado 14-Nov 01:02) | **CORRIGIDO** `f6/recheck-fix` |
| P1-1 | **P1** | `elev_bridge` | Uma falha **da própria comando** (0x80041002) é tratada como falha de transporte ⇒ segunda elevação completa (processo + falha duplicada) | 129 no helper (7,5 %) | **CORRIGIDO** |
| P2-1 | P2 | `hw::battery` | `battery WMI query took` a INFO: 811 linhas (a regra "1 por episódio" é anulada por queries rápidas intercaladas) | 811 | **CORRIGIDO** |
| P2-2 | P2 | `hw::ble_presence` | `[ble_presence] sample` a INFO a cada 30 s = **66 % do log** | 6 410 | **CORRIGIDO** |
| P2-3 | P2 | `hw::hotkeys` | `[hotkeys] WMI key:` a INFO em cada evento cru (o driver repete a tecla premida) | 1 063 | **CORRIGIDO** |
| P2-4 | P2 | `hw::battery` | `local AC adapter-power probe skipped` a INFO a cada 5 min | 487 | **CORRIGIDO** |
| P2-5 | P2 | `micontrol_bridge` | Supressão da sonda de saúde não funciona: 1 linha a cada 30 s | 5 429 | **CORRIGIDO** |
| P2-6 | P2 | `util::auth` / `os_turbo` | Contenção de lock no `elev_key.bin` falha o comando (`os error 33`) | 4 | **CORRIGIDO** |
| P2-7 | P2 | `elev_bridge` | Resposta autêntica rejeitada por "stale" quando a bridge demora >30 s | 1 | **CORRIGIDO** |
| P2-8 | P2 | `ecram_service` | Rajada de resume: 14 conexões dropadas e o serviço reiniciado (9 s) | 14 + 1 restart | **CORRIGIDO** |
| P2-9 | P2 | `hw::wmi_ec` | 0x80041002 do provider: 7,5 % das leituras WMAA elevadas | 129 | **CORRIGIDO** |
| P2-10 | P2 | `micontrol_bridge` | `accept step failed: win32 error 232` (ERROR_NO_DATA) | 6 | **ACEITE** (já tratado: destrói a instância e volta a ouvir; 0 impacto no cliente) |
| P2-11 | P2 | `hw::crash_recovery` | `runtime starved` voltou **1×** (00:53:50) durante um stall de bateria de 129 s; auto-recuperou | 1 ERROR + 4 WARN | **ESCALADO** (§6) |
| P2-12 | P2 | `cmd::system` | `get_battery_info` 40 238 ms e `get_touchpad_info` 2 015 ms (espera de fila do pool) | 2 + 2 | **ACEITE** (residual do F-01; 2 em 53 h vs 9 em 19,5 h) |
| P2-13 | P2 | `hw::charging` | Limite 80 % produz plateau a 74-75 % (base projecto vs capacidade actual) | — | **DOCUMENTADO** (não é bug) |
| P2-14 | P2 | `hw::iot_events` | `battery poll exceeded 3000 ms` a WARN | 82 | **ACEITE** (já limitado a 1/5 min; é a condição real da bateria) |

### P0-1 — UI zombie: WebView2 morto, `ui_ok` a 1, watchdog cego

```
tauri-app.log:19692
2026-11-14T01:02:49.848Z [ERROR] tauri_runtime_wry: WebView2 error: WindowsError(Error {
  code: HRESULT(0x8007139F), message: "The group or resource is not in the correct state to
  perform the requested operation." })
```
Medições no momento (14-Nov 01:03–01:05):
- `Get-CimInstance Win32_Process` → **zero** processos `msedgewebview2.exe` com `ParentProcessId=57060`
  (há 40 na máquina, de outras apps);
- `micontrol.exe` pid 57060 vivo, `MainWindowHandle=6563456`, `Responding=True`, 1 128 handles;
- os 7 comandos do MCP (`execute_js`) devolveram **`Timeout waiting for JS execution`** em 5 s;
- `watchdog.log`: **nenhuma** linha desde `2026-11-11T19:15:46` (nem "UI dead", nem relaunch).

**Causa:** `crash_recovery::ui_is_alive()` respondia `true` com
`app.get_webview_window("main").is_some()` + o probe do runtime Tokio. O objecto de janela do Tauri
é uma entrada de registo do lado Rust; o renderer é uma **árvore de processos à parte** e a sua
ausência é o sinal. Com `ui_ok=1` no heartbeat, o watchdog (que já tem um caminho `ui_ok=0` com
60 s de graça) nunca actuou: a app ficou com uma janela morta sem qualquer recuperação automática.

**Fix:** `ui_ok=1` passa a exigir **≥1 `msedgewebview2.exe` filho deste processo**. A contagem usa
`CreateToolhelp32Snapshot` embrulhado no `OwnedHandle` (o gate de handles continua verde) e é
injectada numa função pura testada (`renderer_state_is_alive`): `None` (snapshot falhou) ⇒ vivo,
`Some(0)` dentro de 90 s de arranque ⇒ vivo, `Some(0)` depois ⇒ **ui_ok=0** + `ERROR` único. O
watchdog reinicia então o zombie pelo caminho que já existia.

### P1-1 — Uma falha da própria comando provoca uma segunda elevação completa

O helper registou **129** `ok=false` com `Hardware error: 0x80041002` na janela e a app apenas 62
WARN — e o **primeiro** par coincide ao segundo (`19:30:16`). A cadeia:
`run_via_service_pipe` devolve `Err` tanto para falha de transporte como para
`{"ok":false,"error":...}` do serviço; `run_elevated_inner` só reaproveita `Ok`, logo trata o erro
**da comando** como erro de transporte e despacha uma **nova elevação** via `schtasks` (processo
novo) que repete a mesma leitura condenada. Custo: 2 processos e 2 falhas por hiccup de 5 ms, mais
o `phase=unknown` do charge-hold quando a vítima é o read-back do limite (20× na janela).

**Fix:** o retry passou para **dentro** de `hw::wmi_ec::wmi_read` (3 tentativas, 200/400 ms, só para
`0x80041002`; `0x80041010` e `0x80070005` falham de imediato). Um stall transitório passa a
devolver `Ok` e a segunda elevação deixa de acontecer; a semântica do fallback fica intacta.
Testes: `only_the_provider_stall_is_retried`, `a_stall_recovered_on_the_second_call_never_surfaces`,
`a_persistent_stall_gives_up_after_the_declared_attempts`, `a_non_transient_error_is_not_retried`.

---

## 4. Fixes aplicados no branch (cada um com teste)

| Fix | Ficheiro | O que muda | Teste |
|---|---|---|---|
| 1 (P1-1/P2-9) | `hw/wmi_ec.rs` | `wmi_read` repete o stall `0x80041002` (3×, 200/400 ms) | 4 testes em `hw::wmi_ec::tests` |
| 2 (P2-1) | `hw/battery.rs` | `BATTERY_SLOW_QUERY_LOG_MS` 1 000→2 500; INFO limitado a 1/10 min | `slow_query_info_is_windowed_not_episode_based` |
| 3 (P2-2) | `hw/ble_presence.rs` | amostra em regime → DEBUG; INFO só em transição + heartbeat 5 min | `steady_state_samples_stop_flooding_info`, `the_state_is_remembered_so_a_transition_is_never_missed` |
| 4 (P2-3) | `hw/hotkeys/mod.rs` | evento cru WMI → DEBUG (a acção despachada continua INFO) | — (só nível de log) |
| 5 (P2-4) | `hw/battery.rs` | `adapter skip` a INFO 1× por motivo; renovações a DEBUG | — |
| 6 (P2-5) | `bin/micontrol_bridge.rs` | linha de sonda limitada a 1/10 min e conta as suprimidas | `the_health_probe_line_is_windowed` |
| 7 (P2-6) | `util/auth.rs` | `ERROR_LOCK_VIOLATION`/sharing (32/33) contam como contenção; `read_key` repete 10×50 ms | `lock_violation_is_contention_not_a_hard_failure` |
| 8 (P2-7) | `util/auth.rs` + `elev_bridge.rs` | `verify_response` (HMAC sem janela de frescura) para respostas | `a_late_but_authentic_response_is_not_rejected_as_stale` |
| 9 (P2-8) | `bin/ecram_service.rs` | `MAX_CONCURRENT_CLIENTS` 16→64 (rajada de resume) | — (constante) |
| 10 (P0-1) | `hw/crash_recovery.rs` | `ui_ok=1` exige renderer WebView2 vivo | `a_missing_renderer_is_reported_as_a_dead_ui` |

**Verificação:** `cargo check --lib --bins --features face` OK · `cargo test` dos módulos tocados
(`hw::wmi_ec` 10, `hw::ble_presence` 22, `hw::battery` 16, `util::auth` 21, `hw::crash_recovery` 5,
`bin micontrol_bridge` 25 — todos verdes) · `cargo clippy --all-targets --features face -D warnings`
OK · `cargo fmt` aplicado · `node scripts/check-handle-leaks.mjs` **34 sites, todos com RAII** ·
sem alterações em `src-tauri/gen/schemas` · nenhum ficheiro TS tocado (tsc/eslint/vitest não se
aplicam a esta ronda).

---

## 5. Estado live (14-Nov 01:19, via pipe `micontrol_bridge`, read-only)

A UI está morta (P0-1), por isso o MCP `execute_js` não responde; a **bridge responde na mesma**
(serviço autónomo) e foi ela a fonte dos valores abaixo.

| Comando (pipe v3) | Latência | Resultado |
|---|---|---|
| `get_charge_limit_state` | 98 ms | **requested 80 · hardware 80 · code 1 · applied true · source firmware** |
| `get_battery_care` | 86 ms | `true` |
| `read_thermal_readings` | 185 ms | ESIF 15 participantes **82 °C**, ACPI `TZ00_0` 27,85 °C |
| `wmi_ec_read(0x0800,0)` | 30 ms | `SGER=0x8000`, `FRD0=9` → QFAN=SmartAdaptive |
| `wmi_ec_read_adapter_power` | 75 ms | 100 W (`supported=true`) |

**Processos:** `micontrol` 57060 (19,2 MB WS, 1 128 handles, 74 threads) · `micontrol_bridge` 39360 ·
`micontrol_face_svc` 17888 · `IoTService` 42540. **Serviços** todos `Running/Auto`.
**Tarefa `MiControlElevated`:** `Ready`, último resultado **0** (13-Nov 00:25) — deixou de ser usada
porque a pipe passou a servir tudo (bom sinal: 0 spawns em 24 h). **Firewall:** 7 regras.
**Helpers órfãos:** 0. **Dumps:** 0 (`crashdumps` vazio, WER sem relatórios MiControl).

### 5.1 Event Log (11-Nov 19:16 → 14-Nov 01:15)

| ID | MiControl | Leitura |
|---|---|---|
| 1000 | **0** | sem APPCRASH da app |
| 1001 | **0** | os 1001 do período são `AnycubicSlicerNext` (AppHangTransient), `RADAR_PRE_LEAK_64`, `crashpad_log`, PowerShell |
| 7031/7034 | **0** | nenhum serviço MiControl terminou (o 7034 do período é o *Energy Server Service queencreek*, Intel) |
| 7036 | 8 | filtros de rede a *pause/detach* no sleep (00:49:41) e *attach* no resume (09:40:34) — não é nosso |
| 41/6008 | **0** | sem shutdown inesperado |

---

## 6. Escalado (não corrigido nesta ronda)

1. **P2-11 — `runtime starved` sob stall de 129 s da bateria.** Às 00:53:31/00:53:50 o probe falhou
   duas vezes seguidas enquanto uma query de bateria levava **129 580 ms** (`circuit open for 300 s
   after 7 consecutive timeouts`). A app recuperou sozinha e o watchdog (correctamente) não matou
   nada. O defeito estrutural é o **provider WMI da bateria**: 4 180 queries >1 s no ficheiro,
   máximo 129 s. Resolver exige outra fonte de bateria (ou um wrapper de provider), não cabe num
   fix de app. Taxa actual: 1 ERROR em 53 h vs 680 na sessão pré-fix — melhoria de 3 ordens de
   grandeza, razão pela qual fica P2 e não P1.
2. **Cobrir o caminho de relaunch do watchdog** (fix `h` do deploy anterior): 0 relaunches em 53 h
   significa que a espera pelo `:4000` continua por exercitar. Teste controlado (matar a app com a
   porta ocupada) é trabalho de bancada, não de log.

## 7. Observações para o dono

- **O limite de carga está a valer.** Pela primeira vez há um plateau observado: 74-75 % durante
  dois dias com o carregador ligado (`plugged=true, charging=false`), e o read-back diz
  `applied=true`. A diferença para os 80 % pedidos é a base de cálculo: o EC limita 80 % da
  **capacidade de projecto** e o Windows reporta % da **capacidade actual** (saúde 93,3 % ⇒ 74,6 %).
- **A tampa não foi fechada na janela** — o listener está registado (arranque sem erro 87), mas a
  transição `[lid] CLOSED` continua por observar. Fechar a tampa uma vez confirma o fix.
- **A UI estava morta e ninguém sabia.** É o único achado que afecta o uso normal da app; o fix
  faz o watchdog reiniciar o zombie em ≤2 min (60 s de graça + 30 s de poll).
- O binário em produção (19:12) continua a ser o desta auditoria; os 10 fixes só entram no próximo
  deploy.

---

*Fontes: `%LOCALAPPDATA%\MiControl\logs\tauri-app.log` (linhas 9791-19699), `tauri-app-helper.log`,
`%PROGRAMDATA%\MiControl\logs\{bridge.log,bridge.log.1,bridge.log.2,watchdog.log,ecram_service.log}`,
Event Log Application/System desde 11-Nov 19:16, `Get-CimInstance`/`Get-Process` para processos e
handles, e 5 comandos read-only pela pipe `\\.\pipe\micontrol_bridge` (protocolo v3).*