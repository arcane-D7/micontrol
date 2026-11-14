# RECHECK-3 — auditoria pós-deploy 14-Nov 01:44 (8 h)

**Âmbito:** provar em produção que os 11 fixes do RECHECK-2 estão a funcionar, contar e classificar
tudo o que apareceu na janela e **corrigir** o que se encontrou.
**Janela:** `2026-11-14T01:44:18.225Z` → `2026-11-14T10:02Z` (8 h 18 m de runtime contínuo, uma
sessão, **pid 18804**, sem relaunch).
**Deploy auditado:** `C:\Program Files\MiControl\micontrol.exe` mtime **14-Nov 01:39:12Z**,
134 454 272 B, versão **0.2.20**; marker `started_at=2026-11-14T01:44:18.257Z`; bridge/ecram
reiniciados pelo installer às 01:43.
**Branch:** `f7/logs-final` (worktree `wt/wt/f7-logs-final`), base `648e347`.
**Modo:** leitura de logs + Event Log + processos + sondagem IPC read-only + fixes com testes no
branch. Health gate completo não foi corrido (é do orchestrator).

---

## 1. Sumário executivo

| Métrica | RECHECK-2 (53 h) | **RECHECK-3 (8,3 h)** | Leitura |
|---|---|---|---|
| Linhas da app | 9 972 (187/h) | **629 (76/h)** | −59 % de volume |
| INFO / WARN / **ERROR** | 9 672 / 285 / **2** | 493 / 136 / **0** | **zero ERROR** |
| WARN/h | 5,3 | 16,4 | sobe — 82 % é a bateria (§3, P1-1) |
| `runtime starved` | 1 | **0** | |
| Relaunches do watchdog | 0 | **0** | caminho continua sem cobertura |
| Crashes (Event 1000/1002) / dumps | 0 / 0 | **0 / 0** | |
| Pedidos à bridge | 54 006, 0 `ok=false` | **19 425, 0 `ok=false`** | 2 340/h, p50 11-24 ms |
| Despachos elevados (helper) | 1 715, **129 `ok=false`** | **48, 1 `ok=false`** | P1-1 do RECHECK-2 provado |
| Handles da app | 1 195 → 1 128 | **1 099** após 8,3 h | sem crescimento (baseline anterior 1 128-1 195) |
| `[lid]` | 2 (só arranque) | 2 (só arranque) | tampa não foi fechada |
| `[charge_hold]` | 55 (1 holding) | **19 (1 holding, 9 unknown→holding)** | plateau mantido |
| Event 7031/7034 da MiControl | 0 | **1 × 7031 (MiControlFace, 01:44:02)** | restart do installer |
| Probes da bridge logadas | 5 429 (102/h) | **49 (5,9/h)** + 941 suprimidas | −95 % de ruído |

**Contagem desta ronda: P0 = 0 · P1 = 1 (escalado, causa fora da app) · P2 = 10** — **6 corrigidos**
(2 commits), **4 aceites/documentados** com justificação. **Nenhum dos 11 fixes do RECHECK-2
regrediu**; todos os que eram observáveis na janela estão comprovados (§2).

**Veredicto:** o deploy de 01:44 fechou o P0 (UI zombie) e o P1 (elevação dupla) do RECHECK-2 e
matou os geiseres de log (−90 % a −99 % por família). O que resta é **uma** condição de hardware —
o provider WMI da bateria — que piorou 4,8× nesta janela e é a origem de 82 % dos WARN, dos
`phase=unknown` do charge-hold e de dois comandos a passar dos 5 s. Está escalada (§7); a app levou
duas mitigações (leitura paciente do registo de carga, orçamento de elevação honesto).

---

## 2. Prova fix-a-fix dos 11 fixes do RECHECK-2

| # | Fix (RECHECK-2) | Veredicto | Evidência nesta janela |
|---|---|---|---|
| 1 | `ui_ok` exige renderer WebView2 vivo (P0-1) | **OK** | `%ProgramData%\MiControl\heartbeat` tem o formato novo de 3 linhas: `ts · 18804 · **1**` (fresco, 23 s). `Get-CimInstance` confirma **1** `msedgewebview2.exe` filho do pid 18804. **Zero** `0x8007139F`, zero `UI dead`, zero relaunch. |
| 2 | Retry `0x80041002` dentro de `wmi_read` (P1-1/P2-9) | **OK** | Helper: **129 `ok=false` → 1** em 48 despachos (7,5 % → 2,1 %). O único que passou: `08:38:03 Hardware error: 0x80041002` num stall que excedeu os 600 ms de retry. Segunda elevação desapareceu (helper 1 715 → 48 despachos). Nota: a linha do retry é `DEBUG` (`WMAA read hit a degraded WMI provider … retry n/2`), por isso a prova em produção é a queda dos despachos/falhas, não uma linha de retry no log INFO. |
| 3 | `battery` slow-query a DEBUG, INFO 1/10 min (P2-1) | **OK** | `battery WMI query took`: 811 (15,2/h) → **27 (3,3/h)**, com o novo limiar de 2 500 ms. |
| 4 | `ble_presence` amostra em regime a DEBUG (P2-2) | **OK** | 6 477 (121,5/h) → **101 (12,2/h)** = exactamente o heartbeat de 5 min. Transições continuam a sair (1 `rssi=Some(-74)` às 02:22). |
| 5 | `hotkeys` evento cru a DEBUG (P2-3) | **OK** | 1 056 (19,8/h) → **1 (0,1/h)**; a acção despachada continua INFO (`Consumer usage 0x1747`, `debounced`). |
| 6 | `adapter skip` 1×/motivo (P2-4) | **OK** | 494 (9,3/h) → **1** (arranque, `0x80041003`, com a explicação de que a bridge cobre o caminho). |
| 7 | Sonda da bridge em janela de 10 min (P2-5) | **OK** | 5 429 (102/h) → **49 linhas (5,9/h)**; **990 probes** reais na janela, **941 suprimidas** e contadas (`19/20 identical probe(s) suppressed`). O contador fecha: 49 + 941 = 990. |
| 8 | Contenção de lock 32/33 no `elev_key.bin` (P2-6) | **OK (não exercitado)** | 0 `os error 33` na app e no helper (eram 2). Sem contenção na janela, o caminho não foi preciso. |
| 9 | `verify_response` sem janela de frescura (P2-7) | **OK (não exercitado)** | 0 respostas rejeitadas por "stale" (era 1). |
| 10 | `MAX_CONCURRENT_CLIENTS` 16→64 (P2-8) | **OK (não exercitado)** | `ecram_service`: 0 `Too many concurrent pipe clients`, 0 reinícios desde 01:43 (nenhum resume na janela, logo a rajada não ocorreu). |
| 11 | `elev` deadline de 15 s (fix anterior, g) | **PARCIAL → corrigido nesta ronda** | 4 abortos, mas **dois passaram do limite**: 09:14 `29 607 ms` na fase 'startup' (total **34 278 ms** para 15 s) e 09:35 `16 128 ms` na fase 'stale-file sweep'. A mensagem dizia "aborted at the declared limit" — falso. Ver P2-2/P2-3. |

**Extras verificados:** `P2-10` (`accept step failed 232`) = **0** na janela (era 6);
`P2-12` (`get_battery_info` 40 s) = **0** WARN de `cmd::system`; `P2-14` (`battery poll exceeded`)
mantém-se mas a subir (§3).

---

## 3. Achados novos

| ID | Sev | Módulo | Achado | Evidência | Estado |
|---|---|---|---|---|---|
| P1-1 | **P1** | `hw::battery` / provider WMI | Degradação do provider: 76 timeouts de 5 s (**9,2/h** vs 1,9/h) + 35 polls saltados (4,2/h vs 1,7/h); picos de **52,8 s** e **60,7 s**; knock-on em comandos e no charge-hold | `tauri-app.log` 09:13-10:02; §5 | **ESCALADO** (§7) |
| P2-1 | P2 | `hw::crash_recovery` | `Abnormal restart detected` em cada instalação, **uma linha depois** de a app dizer que o shutdown foi do installer | `tauri-app.log:19931` (01:44:18.260 "terminated by the installer … intentional, not a crash") → `:19946` (01:44:20.599 "Abnormal restart detected") | **CORRIGIDO** `ebe5179` |
| P2-2 | P2 | `elev_bridge` | Espera do lock de serialização sem limite: 29 607 ms na fase 'startup', **34 278 ms** de wall clock para um orçamento de 15 s, e a mensagem a afirmar que abortou "no limite declarado" | `:20382` (09:14:11.288), `:20383` (09:14:16.668) | **CORRIGIDO** `ebe5179` |
| P2-3 | P2 | `elev_bridge` | O *stale-file sweep* corre no caminho crítico do dispatch e comeu o orçamento inteiro: 16 128 ms, comando morto antes de escrever o pedido | `:20427`/`:20428` (09:35:25) | **CORRIGIDO** `ebe5179` |
| P2-4 | P2 | `hw::display` | `Sensor reset requested (power resume event)` escrito pelo *health supervisor* quando o sensor ambiente está degradado — **10 falsos resumes** numa janela sem um único suspend | `:20207` (04:54), `:20384` (09:14) … e o log não tem `[power_listener] Resume` | **CORRIGIDO** `ebe5179` |
| P2-5 | P2 | `hw::charging` / `hw::wmi_ec` | O read-back do limite de carga tem 600 ms de retry, mas os stalls deste provider duram segundos: **9 ciclos `unknown→holding`** desde 06:02 e, ao vivo, `hardware_threshold: null` / `Hardware error: 0x80041002` em 1 de 6 leituras (17 %) | `:20263`, `:20270`, `:20283` …; §5 | **CORRIGIDO** `ebe5179` |
| P2-6 | P2 | `hw::hotkeys` | Registo de log multi-linha: o stdout do script elevado era escrito tal e qual, deixando **4 linhas por arranque sem timestamp nem nível** (`HKCU TurnOffWindowsCopilot set to 1`, …) e a partir o parsing linha-a-linha | `tauri-app.log:19980-19983` | **CORRIGIDO** `7766581` |
| P2-7 | P2 | `bin/ecram_service` | Dois arranques a 2 s de distância no arranque do installer: pid 41052 (01:43:47) não chegou a registar `pipe server starting`; pid 26876 (01:43:49) é o que serve | `ecram_service.log` tail; serviço `IoTSvc` → pid 26876 | **ACEITE** (padrão de restart do installer; 1 pipe vivo, 0 clientes perdidos) |
| P2-8 | P2 | `hw::os_turbo` | `sweep` a INFO a cada ~60 s em séries (72 linhas, 8,7/h) porque `inspected` oscila 5↔6 e derrota o "unchanged → DEBUG" | `[os_turbo] sweep: inspected=5/6 applied=5/6` | **ACEITE** (abaixo do orçamento de ruído; candidato a janela se crescer) |
| P2-9 | P2 | `tauri_plugin_mcp` | `execute_js` tem tecto fixo de 5 s e é serializado no renderer: um comando lento faz os seguintes falharem por arrastamento (3 timeouts seguidos às 09:35) | `[TAURI_MCP] Error: Timeout waiting for JS execution` ×3 | **DOCUMENTADO** (para sondar hardware usar a pipe; §5) |
| P2-10 | P2 | `applog` | A linha de resumo do rate-limiter está em português (`×N repetições suprimidas`) num log todo em inglês (27 linhas) | `debug_log.rs:391` | **ACEITE** (cosmético; testes pinam a string) |

### Detalhe dos dois achados de elevação (P2-2/P2-3)

```
tauri-app.log:20382  09:14:11.288 [WARN] elev_bridge: Elevated command 'wmi_ec_read' exceeded its
  total budget of 15 s (stuck in 'startup' after 29607 ms) — aborted at the declared limit …
tauri-app.log:20383  09:14:16.668 [WARN] elev_bridge: Elevated command 'wmi_ec_read' failed after
  34278 ms …
tauri-app.log:20427  09:35:25.987 [WARN] elev_bridge: … (stuck in 'stale-file sweep' after 16128 ms)
```
**Causa:** (a) `ELEV_REQUEST_LOCK.lock().await` sem prazo — o chamador fica na fase 'startup' atrás
de outro comando que já tinha rebentado o orçamento; o `tokio::time::timeout` exterior só dispara
quando o runtime volta a correr, daí 34 s; (b) o sweep de ficheiros (`read_dir` em
`%LOCALAPPDATA%\MiControl`) corria **em cada** dispatch, com tecto de 10 s — num filesystem lento
consome o orçamento todo e o comando morre antes de ser escrito.

### Detalhe do charge-hold (P2-5)

19 transições em 8,3 h: 1 `holding` (01:44, 79 %), depois **9 pares** `unknown (78 %) firmware=None%`
→ `holding after verify (firmware=Some(80)%)` entre 06:02 e 09:54. O `unknown` é honesto (cache de
15 min expirada porque o read-back falhou); o custo é ~2 min de cegueira por ciclo. Ao vivo, 1 em 6
leituras de `get_charge_limit_state` devolveu `hardware_threshold: null` com
`Hardware error: 0x80041002` — **o mesmo stall do provider**, não um bug do supervisor.

---

## 4. Fixes aplicados (2 commits, cada um com testes)

### `ebe5179` — honestidade de reinício/timeout/fase + esperas limitadas

| Fix | Ficheiro | O que muda | Teste |
|---|---|---|---|
| P2-1 | `util/session_marker.rs`, `hw/crash_recovery.rs` | O veredicto do `begin_session` passa a ficar publicado (`previous_session_verdict()` / `previous_session_ended_at_installer()`); `init_crash_recovery` ignora a flag HKCU quando o installer anunciou o shutdown e loga INFO com o motivo (`abnormal_restart_from_flag`, pura) | `an_installer_shutdown_never_counts_as_an_abnormal_restart`, `the_installer_verdict_is_published_for_later_startup_phases` |
| P2-2 | `elev_bridge.rs` | Espera do lock limitada pelo orçamento restante → erro claro "another elevated command held the serialization lock" dentro do prazo; a mensagem de orçamento passou a distinguir **honrado no limite** de **rebentado** (mostra o tempo real e diz que o runtime estava bloqueado) | (lógica de timeout; coberta pelo comportamento; 19 testes do módulo verdes) |
| P2-3 | `elev_bridge.rs` | Sweep de ficheiros: **1×/5 min** (`claim_sweep_slot` + `sweep_is_due`) e tecto de **2 s** (`SWEEP_TIMEOUT`); fora da janela não corre | `the_stale_file_sweep_is_windowed_not_per_dispatch` |
| P2-4 | `hw/display.rs` (+ `health_supervisor.rs`, `power_listener.rs`) | `request_sensor_reset(reason)`: DEBUG com o motivo real; o resume continua a ter o INFO do power listener; o supervisor passa "ambient sensor degraded" | — (só nível/mensagem; 3 call sites actualizados) |
| P2-5 | `hw/wmi_ec.rs`, `hw/charging.rs` | `wmi_read_patient` (5 tentativas, backoff 250 ms linear = 2,5 s) usado **só** pelo read-back do limite de carga; `with_transient_retry_config` parametrizado, caminho genérico intacto | `the_patient_budget_covers_the_slow_stalls_that_blank_the_charge_read` |

### `7766581` — um registo por resultado da política Copilot

| Fix | Ficheiro | O que muda | Teste |
|---|---|---|---|
| P2-6 | `hw/hotkeys/mod.rs` | O stdout do script elevado (uma linha por resultado) é juntado com `"; "` num **único** registo com timestamp/nível | — (formatação; 27 testes do módulo verdes) |

**Verificação:** `cargo check --lib --bins --features face` OK · testes dos módulos tocados
(`util::session_marker` **14**, `hw::crash_recovery` **6**, `elev_bridge` **19**, `hw::wmi_ec` **11**,
`hw::charging` **20**, `hw::hotkeys` **27** — todos verdes) · `cargo clippy --all-targets
--features face -- -D warnings` **EXIT=0** · `cargo fmt --check` OK ·
`node scripts/check-handle-leaks.mjs` **34 sites, todos com RAII** · nenhum ficheiro TS tocado
(tsc/eslint/vitest não se aplicam) · `src-tauri/gen/schemas` sem alterações.

---

## 5. Estado live (10:02-10:05, read-only)

| Comando (MCP `execute_js`, pid 18804) | Latência | Resultado |
|---|---|---|
| `get_charge_hold_state` | **48 ms** | `phase=holding · level 79 · limit 80 · band 78-80 · firmware 80 · applied true · charging false · plugged true` |
| `get_charge_limit_state` | **1 265 ms** | `requested 80 · hardware **null** · applied false · source registry · detail "Hardware error: 0x80041002"` — em 6 leituras: 1 null (17 %), as outras 14-27 ms com `hardware 80` |
| `get_fan_info` | **49 ms** | `cpu 73 °C · gpu 62 °C · ambient 27,85 °C · tdp 24,6 W · source esif · ec_active smart_adaptive (raw 9) · lid_open true` |
| `get_overview_snapshot` | **5 ms** (cache) | XIAOMI Xiaomi Book Pro 14, Ultra X7 358H, Arc B390, NPU presente |
| `get_startup_items` | **1 016 ms** | 37 entradas, `tasks_available=true`, sem erro de task-scan (1,8 s a frio; 9 ms em cache) |
| `get_color_status` | **197 ms** | `xiaomi_D65P3_base.icc` activo, HDR ligado, painel `EDO 0x4503` |

> Às 09:35 três comandos (`get_fan_info`, `get_overview_snapshot`, `get_startup_items`) devolveram
> `Timeout waiting for JS execution` a 5 s, durante o pico do stall da bateria; repetidos 10 min
> depois deram 49/5/1 016 ms. É contenção, não UI zombie: o renderer está vivo e o heartbeat diz
> `ui_ok=1`.

**Processos:** `micontrol` **18804** (26,3 MB WS, **1 099 handles**, 72 threads, arranque
01:44:18) · `micontrol_bridge` 22960 · `micontrol_face_svc` 11768 · `IoTService` 26876.
**Serviços** todos `Running/Auto`; tarefa `MiControlElevated` `Ready`; **dumps 0**.

**Watchdog:** última linha `01:44:21 heartbeat stale — confirmation 1/2` (arranque); desde aí
**nenhuma** linha — sem misses, sem `UI dead`, sem relaunch. **Helpers órfãos:** 0.

---

## 6. Event Log (01:44 → 10:03)

| ID | Ocorrências | Leitura |
|---|---|---|
| 1000 / 1002 | **0 / 0** | sem APPCRASH/AppHang da app |
| 1001 | 1 (05:38:04) | `RADAR_PRE_LEAK_64`, não é nosso |
| 7031 MiControl | 1 (01:44:02) | `MiControl Face Auth Service` — restart do installer (padrão já documentado) |
| 7034 | 1 (03:11:07) | *Energy Server Service queencreek* (Intel), não é nosso |
| 41 / 6008 | 0 | sem shutdown inesperado |

---

## 7. Escalado (não corrigível na app)

**P1-1 — provider WMI da bateria.** Números desta janela vs as 53 h anteriores:

| Métrica | 53 h (RECHECK-2) | 8,3 h (agora) | Factor |
|---|---|---|---|
| `battery WMI query exceeded 5000 ms` | 101 (1,9/h) | **76 (9,2/h)** | **4,8×** |
| `battery poll exceeded 3000 ms` | 88 (1,7/h) | **35 (4,2/h)** | 2,5× |
| Pico de query observado | 129 580 ms | **60 689 ms** (e 52 852 ms às 09:14) | — |
| `battery WMI recovered` | — | 71 | recupera sempre |

Distribuição horária: 02h 17, 03h 11, 04h 12, 05h 8, 06h 4, 07h 3, 09h 6 — degradação contínua,
não um pico isolado. Consequências medidas: comandos acima dos 5 s do MCP (3 no mesmo minuto), 9 ciclos
`unknown→holding` no charge-hold (via `WMAA 0x1000/0x02`), 1 falha de elevação `0x80041002` e os
dois estouros de orçamento da §3. **Causa fora da app** (o provider serve `Win32_Battery` e o
`root\WMI` no mesmo host; a app já tem worker dedicado + circuit breaker + cache). Precisa de
**outra fonte de bateria** (ou wrapper de provider) — decisão de arquitectura, não de log.
Mitigações já aplicadas nesta ronda: leitura paciente do registo de carga (P2-5) e orçamento de
elevação honesto/limitado (P2-2/P2-3).

**Herdado, continua sem cobertura:** o caminho de relaunch do watchdog (0 relaunches em 61 h
acumuladas). Teste de bancada: matar a app com o `:4000` ocupado.

---

## 8. Observações para o dono

- **O limite de carga está a valer e agora no sítio certo:** `holding` a **79 %** com
  `plugged=true, charging=false`, `firmware 80`, `applied=true`. Ao contrário das 53 h anteriores
  (plateau 74-75 %), o pack estabilizou em **78-80 %** — a mesma ordem de grandeza da banda
  `[78..80]`, sem nenhuma reaplicação forçada na janela (só o verify de 5 min).
  **Sem transição AC/DC** em 8,3 h (todas as 19 linhas `[charge_hold]` com `plugged=Some(true)`),
  logo o caminho `force` de resume/AC-plug não foi exercitado; o que se observou foi o EC a
  **reabrir a carga** ao chegar aos 80 % (`[iot_events] charging transition: charging=true` às
  09:28:53, `false` às 09:32:43) e a fechar outra vez no limite — comportamento correcto.
- **A UI já não morre em silêncio:** heartbeat com `ui_ok=1` + 1 renderer vivo + 0 `0x8007139F` em
  8,3 h. O caminho negativo (renderer morto ⇒ `ui_ok=0` ⇒ relaunch) continua provado só por teste
  unitário; para o exercitar era preciso matar o `msedgewebview2` à mão.
- **A tampa continua por fechar** (2 `[lid]`, ambos no arranque): o listener está registado por
  user32, mas a transição `CLOSED` nunca foi observada.
- **O log da app está 2,5× mais silencioso** (76 vs 187 linhas/h) e sem ERROR. O que sobra é 82 %
  bateria — se o provider for resolvido, o log fica praticamente limpo.
- **Os 6 fixes desta ronda não estão em produção** (branch `f7/logs-final`); o binário em produção
  continua o de 01:39. Entram no próximo deploy via auth bridge.