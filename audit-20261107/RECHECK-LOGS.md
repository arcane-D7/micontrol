# RECHECK-LOGS — auditoria pós-deploy (10-Nov 20:17 → 11-Nov 15:47)

**Âmbito:** verificar, nos logs de produção e por IPC ao vivo, se cada fix da ronda de
estabilização está a funcionar; classificar erros novos; medir performance.
**Modo:** só leitura. Zero mutações de hardware, zero escritas de configuração.
**Deploy auditado:** binário 20:11, app 20:17:11Z (pid 55092, v0.2.20), bridge 20:16:47Z.
**Janela:** 19 h 30 m de runtime contínuo (uma única sessão, sem relaunch).

---

## 1. Sumário executivo

| Métrica | Valor |
|---|---|
| Linhas de log da app (pós-deploy) | 4 393 (4 259 INFO · 130 WARN · **0 ERROR**) |
| Taxa | 6,7 WARN/h · 0 ERROR/h |
| `runtime starved` | **0** (sessão anterior: 680) |
| `pipe busy` | **0** |
| Relaunches do watchdog | **0** (19 h 30 m sem uma única falha de heartbeat) |
| Crashes nativos (Event 1000) | **0** · dumps em `crashdumps`: **0** |
| Pedidos à bridge | 19 049 respondidos, **0 `ok=false`** |
| Helpers órfãos | **0** |
| Erros de bind `:4000` | **0** (um único LISTEN, pid 55092) |

**Veredicto global:** a ronda de estabilização cumpriu o objectivo de estabilidade — o
congelamento de 87 s e o `runtime starved` desapareceram por completo, e a app sobreviveu
19 h 30 m incluindo um ciclo S3 de 8 h 32 m. **Mas dois fixes estão funcionalmente mortos
em produção** (listener de tampa e persistência do limite de carga) e há um caminho de
elevação que espera 124 s apesar de um limite de 15 s.

**Contagem por severidade: P0 = 0 · P1 = 3 · P2 = 7.**

---

## 2. Veredicto por fix

| # | Fix | Veredicto | Evidência |
|---|---|---|---|
| a | Pool async (tokio oneshot+timeout, 8 threads) | **OK** | 0 `runtime starved` (era 680 em `tauri-app.log.1`); 3 WARN isolados de probe (`4273`, `4291`, `5971`) que recuperaram sem ERROR nem kill |
| b | `iot_events` bateria fora da task async (budget 3 s) | **OK** | 29 `battery poll exceeded 3000 ms (skipping tick)` — salta o tick e continua; transições AC/bateria registadas normalmente (`00:01:40`, `01:20:12`, `01:34:28`) |
| c | `boundedPoll` frontend (2 + jitter) | **OK** | Intervalo entre `wmi_ec_read`: mediana **4,00 s**, p90 4,01, p99 4,19, gap máx. 29 s. Sem tempestades; pico de 130 req/min só na janela de recuperação pós-resume (11:00–11:05) |
| d | Watchdog espera `:4000` antes de relançar | **NÃO EXERCITADO** | 0 relaunches em 19 h 30 m ⇒ o caminho nunca correu. Código presente (`micontrol_bridge.rs:2834`). Nenhum erro de bind, um só listener |
| e | BatteryGate circuit breaker WMI (30/60/120/240 s) | **OK** | 44 timeouts → 9 trips (`circuit open for 30 s after 3 consecutive`, `120 s after 5 consecutive`) e **40 recuperações**; espaçamento visível |
| f | `get_touchpad_info` 2 s · startup budget 5 s | **OK** | Listener de gestos activo 4,72 s após arranque do processo; **0** timeouts de `get_touchpad_info` (eram 5) |
| g | Modo de performance honesto (`EcActiveMode`, readback, SSDB) | **OK (leitura)** | `get_fan_info`: `ec_active=smart_adaptive`, `ec_active_raw=9`, `mode=auto`, `requested_speed_percent=50` — registo e hardware apresentados em separado. Sem escritas na janela, o caminho de escrita não foi exercitado |
| h | Listener de tampa (`[lid] CLOSED\|OPEN` + snapshot) | **REGRESSÃO — MORTO** | `4069: [power_listener] lid-switch registration failed: Win32 error 87`. **Zero** linhas `[lid]` em 19 h 30 m, apesar de um S3 real (02:23:38 → 10:55:41). `lid_open=null` ao vivo |
| i | Limite de carga WMAA 0x1000/0x02 com read-back | **PARCIAL — limite PERDIDO** | Read-back honesto (`applied=false`), mas o hardware está em **100 %/code 0** contra **requested 80**; bateria a carregar 65 % → 72 % acima do limite. Sem re-apply no arranque nem no resume |
| j | Pipe-busy 0, starved 0, tray opaca, startup manager, color WCS, firewall, edge-slide | **OK** | `pipe busy` 0 · `starved` 0 · `[tray] pre-created tray popup OK` · `get_startup_items` 509 ms · ICC `xiaomi_D65P3_base.icc` activo, HDR on, painel `EDO 0x4503` · **9** regras de firewall activas · `[gesture] pointer pinned … released after 397 ms` (ClipCursor a pinar e a libertar) |

---

## 3. Erros novos (não vistos na STAB-ROUND-1)

### P1

**P1-1 · Listener de tampa nunca regista — `Win32 error 87`**
```
tauri-app.log:4069
2026-11-10T20:17:15.993Z [WARN ] micontrol_lib::hw::power_listener:
  [power_listener] lid-switch registration failed: Win32 error 87
```
Causa-raiz provada por sonda isolada (processo limpo, janela válida, mesmo GUID):
`powrprof!PowerSettingRegisterNotification` devolve **87 (ERROR_INVALID_PARAMETER)** tanto
com janela *message-only* como *top-level*; `user32!RegisterPowerSettingNotification`
devolve **handle válido** no mesmo host. O `LidWatch.ps1` que validou a telemetria em
2026-11-10 usa exactamente a variante **user32**. O código da app (`power_listener.rs:159`)
usa a variante **powrprof** — é a chamada errada, não o ambiente.
Consequência: a funcionalidade que motivou o fix (correlacionar fan com tampa fechada)
não existe em produção; `lid_open` fica `null` para sempre.

**P1-2 · Limite de carga perdido, sem re-apply**
```
get_charge_limit_state (ao vivo, 15:47Z):
  requested_threshold=80  hardware_threshold=100  hardware_code=0
  applied=false  source=firmware
get_battery_info: level 65 % → 72 % em 7 min, is_charging=true, charge_rate 23 W
```
Às 20:17 o deploy foi verificado com `hardware 80 / code 1 / applied true`. Entre 00:15 e
agora o EC voltou a 100 %. Suspeitos, por ordem: o ciclo S3 (02:23:38 → 10:55:41) e os dois
restarts do `ecram_service` (10:55:58, 11:11:50). `on_resume()` (`power_listener.rs:284`)
só faz `request_sensor_reset` + `reset_elevated_thermal_circuit` + `wmi_cache::invalidate`
— **não reaplica o limite**. Não há reconciliação periódica. O read-back está correcto
(é honesto dizer `applied=false`), mas o dono pediu 80 % e a máquina vai a 100 %.

**P1-3 · Comando elevado espera 124 s apesar do limite de 15 s**
```
tauri-app.log:7847
2026-11-11T13:23:43.321Z [WARN ] elev_bridge: Elevated command 'wmi_ec_read' failed after
  124278 ms (…): Elevated command 'wmi_ec_read' timed out after 15077 ms (limit 15 s) …
```
O timeout dispara aos 15 s mas o chamador só é libertado 124 s depois — 8× o orçamento.
Ocupa um dos 8 threads do pool durante dois minutos. Mais 4 ocorrências de 15 s
(`22:18:59`, `22:30:35`, `23:06:04`, `23:59:13`). A tarefa `MiControlElevated` está sã
(último run 15:50:21, resultado 0), portanto é espera no join, não tarefa partida.

### P2

| ID | Achado | Linha / contagem | Nota |
|---|---|---|---|
| P2-1 | `async runtime probe failed once (workers may be starved)` | `4273`, `4291`, `5971` (3×) | Residual do F-01; nunca 2 consecutivos ⇒ sem ERROR nem kill. Correlaciona com stalls WMI de 5 s (20:41:30, 20:43:23) |
| P2-2 | `Too many concurrent pipe clients — dropping this connection` | `ecram_service.log:167` (1×, 10:56:13) | Novo, mas único; rajada de reconexão pós-resume |
| P2-3 | `elev_bridge` 0x80041002 | 40× (~2/h) | Crónico (F-12); sempre recuperado no poll seguinte |
| P2-4 | `battery WMI query took N ms (slow threshold 1000 ms, ok)` | **1 438 linhas** = 33 % do log | Crónico (F-07). Stalls >10 s: 5; máximo **50 826 ms** |
| P2-5 | `Region ERAM unavailable (DriverBlocked)` a WARN | `8467` (+1 pré-deploy) | F-14: condição esperada, nível errado |
| P2-6 | `[TAURI_MCP] Timeout waiting for JS execution` | 5× às 15:46 | Não é cold-start: coincide com um stall de bateria (15:45:43–15:46:18). Retries a 28–509 ms |
| P2-7 | `MiControlFace` 7031 ×2 | Event Log 20:16:00, 20:16:58 | Artefacto do deploy (installer mata o serviço duas vezes) |

**Confirmados como NÃO novos** (presentes em `tauri-app.log.1`): clamp de brilho +40/+41 pp
(2→2), `os_turbo drift re-apply failed` (2→1), BLE `Access is denied`, fallback DSDT,
`igcl_session ctlInit 0x40000009`, `ERROR_HOTKEY_ALREADY_REGISTERED`, `AC adapter-power
probe skipped` (372→87).

---

## 4. Estado live (IPC `127.0.0.1:4000`, 15:46–15:48Z)

| Comando | Latência | Resultado |
|---|---|---|
| `get_overview_snapshot` | 2 167 ms (frio) | Ultra X7 358H · Arc B390 · NPU ok · 32 GB 9600 · 3120×2080@120 · build 26200 |
| `get_fan_info` | 92 ms (quente) | `mode=auto` · `ec_active=smart_adaptive` raw 9 · CPU 82 °C · GPU 66 °C · ambiente 27,85 °C · 35,9 W · **`lid_open=null`** |
| `get_charge_limit_state` | 28 ms | **requested 80 / hardware 100 / code 0 / applied false / firmware** |
| `get_battery_info` | 3 491 ms | 65 %→72 %, a carregar, 23 W, saúde 93,3 %, 56 min p/ cheio |
| `get_ecram_map` | 31 ms | `available=false`, `driver_blocked` (ERAM, SMA2) — esperado |
| `get_color_status` | 159 ms | `xiaomi_D65P3_base.icc` activo, HDR on, painel `EDO 0x4503` |
| `get_startup_items` | 509 ms | ok (MiControl `enabled=true`, Discord `false`) |
| `get_touchpad_info` | 2 260 ms | ok (very_high, haptics high, edge_slide on) |
| `get_os_turbo` | 105 ms | enabled, 13 processos aplicados, 0 falhas, worker activo |

**Processo:** pid 55092 · 42 MB WS / 207,7 MB privado · **1 195 handles** · 77 threads ·
CPU 552,8 s em 19,5 h (**0,79 %** médio).
**Serviços:** bridge 28684, face 27724, IoT 63640 — todos vivos, sem duplicados.
**Firewall:** 9 regras `MiControl*` (≥7 esperadas), todas activas.
**Helpers órfãos:** nenhum (`micontrol.exe --elevated` não fica pendurado).

---

## 5. Performance

- **Log/h:** ~350–380 linhas/h em regime; pico 818 às 11h (recuperação pós-resume).
  Total 4 393 / 19,5 h = 226/h.
- **Bateria WMI:** 44 timeouts (5 s) em 19,5 h = 2,3/h — **vs 111 na STAB-ROUND-1**.
  O breaker espaça correctamente: 30 s após 3 consecutivos, 120 s após 5. 40 recuperações.
  Continua a haver stalls reais de 11 s, 15 s, 18 s e um de **50,8 s** — o thread do pool
  fica ocupado esse tempo todo (é o que produz os 3 probes falhados e os 5 timeouts MCP).
- **Bridge:** `wmi_ec_read` p50 8 ms / p95 12 / p99 29 / máx 729 ms. 0 falhas em 19 049.
- **Ruído:** 2 334 linhas `read N bytes (client closed before sending)` = 2/min exactos
  (sonda de saúde que liga e fecha sem enviar; padrão pré-existente — 3 867 no log anterior).

---

## 6. Acções sugeridas

1. **P1-1 (tampa):** trocar `PowerSettingRegisterNotification` (powrprof) por
   `RegisterPowerSettingNotification` (user32) em `power_listener.rs:159`, mantendo
   `DEVICE_NOTIFY_WINDOW_HANDLE`; adicionar auto-teste no arranque que sobe o nível para
   ERROR se a inscrição falhar (um WARN silencioso escondeu a funcionalidade morta 19 h).
   Validar com uma tampa fechada real e confirmar `[lid] CLOSED` + `lid_open=true/false`.
2. **P1-2 (limite de carga):** reaplicar `requested_threshold` (HKLM) no arranque **e** em
   `on_resume()` quando `hardware != requested`, sempre com read-back; mais uma
   reconciliação periódica (5 min) porque o EC perde o valor em reset. A UI já sabe dizer
   "not applied" — falta o lado que corrige.
3. **P1-3 (elevação):** o `join` do comando elevado não respeita o limite de 15 s; cortar a
   espera no chamador e logar qual tarefa a ocupava (o `wmi_ec_read` de 124 s é o pior caso
   observado e ocupa 1/8 do pool).
4. **P2-4:** depois de N timeouts consecutivos, descer as linhas `slow threshold` a DEBUG
   (1 438 linhas = 1/3 do log) e manter só as transições de estado a INFO.
5. **P2-5:** `Region ERAM unavailable (DriverBlocked)` é esperado — baixar a INFO.
6. **P2-2/P2-6:** suprimir a sonda de 2/min que liga e fecha sem enviar; e dar ao
   `execute_js` do MCP um orçamento > 5 s (ou devolver "busy" em vez de timeout) para não
   confundir stall de bateria com falha de comando.
7. **Fix (d) sem cobertura:** 19 h 30 m sem relaunch significa que a espera pelo `:4000`
   nunca correu. Fazer um teste controlado (matar a app com a porta ocupada) antes de
   considerar o fix verificado.

---

*Relatório gerado por auditoria de logs + IPC, só leitura. Fontes: `%LOCALAPPDATA%\MiControl\logs\tauri-app.log` (sessão a partir da linha 4030), `tauri-app-helper.log`, `%PROGRAMDATA%\MiControl\logs\{bridge,watchdog,ecram_service}.log`, Event Log Application/System desde 10-Nov 20:00, e sondas MCP a `127.0.0.1:4000`.*
