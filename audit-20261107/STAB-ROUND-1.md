# STAB-ROUND-1 — Auditoria de logs e estabilidade pós-4 deploys

**Data da leitura:** 2026-11-09 10:00Z → 2026-11-10 18:42Z (34,7 h; snapshot às 18:42Z — o log continua a crescer)
**Build em produção:** MiControl 0.2.20, binário `C:\Program Files\MiControl\micontrol.exe` com mtime **2026-11-09 16:32:12**, deploy final às **16:36**
**Branch de trabalho:** `f4f/logs-audit` (worktree `wt/wt/f4f-logs-audit`, HEAD `b441556`)
**Regra:** apenas leitura. Nenhum `set_*`, escrita de EC, reset, health gate ou alteração de estado. As únicas escritas foram este relatório e o commit no branch.

> **Nota de datas:** os timestamps dos logs colocam os 4 deploys em **09-Nov** (10:47, 13:13, 15:34, 16:36). O relógio da máquina está em **10-Nov 18:42**. O briefing dizia "hoje (09-Nov)"; a janela real coberta é 09-Nov 10:00 → 10-Nov 18:42, e é essa que este relatório usa. A sessão atual começou a 09-Nov 23:22 e tem 19,3 h.

---

## 0. TL;DR

**Contagem por severidade: P0 = 0 · P1 = 6 · P2 = 9** (15 achados: 4 novos/reapertos nesta ronda, 11 crónicos ou esperados).

1. **O `runtime starved` NÃO está corrigido.** Reapareceu 2× a 09-Nov (16:07:46 e 16:17:30), congelou a app durante **87,7 s**, o watchdog matou o processo e o relançamento imediato **morreu ao arrancar**. A causa provável está no código: `fixed_pool::run_blocking_timeout` faz `std::sync::mpsc::recv_timeout()` **dentro de uma `async fn`**, bloqueando os workers do runtime Tokio (não é o pool dedicado que devia isolar o trabalho). O `iot_events` ainda chama a bateria síncrona a cada 5 s no task Tokio.
2. **Corrida watchdog → relançamento:** o watchdog mata e lança a app **1,2 s depois**, sem esperar pela libertação da porta MCP 4000. A nova sessão morre ao arrancar, após 5 linhas, exactamente no ponto do bind do MCP; 2 min de indisponibilidade até ao 2.º relançamento. É a mesma classe do PANIC de 08-Nov (`Failed to bind 127.0.0.1:4000`, os error 10048).
3. **Bateria WMI é o único erro crónico relevante:** 111 timeouts, 217 queries > 5 s, 76 > 10 s, máximo **67,4 s** em 34,7 h. Alimenta o ponto 1. A todos os polls normais (~1,2-3,6 s) somam-se **3.808 linhas de log** só nesta sessão (45% do ficheiro) — ruído que enterra os avisos reais.
4. **Fila de bloqueio saturada quebra comandos do utilizador:** `get_touchpad_info` 5 s ×5, `get_battery_info` 15 s ×4, comando elevado de adapter power 30,8 s ×1. Todos no mesmo cluster 15:59-16:20.
5. **App desapareceu às 23:21 sem shutdown marker, sem WER 1000/1001 e sem dump.** O watchdog relançou-a às 23:22:56. Não há evidência de crash nativo; a hipótese mais forte é kill externo (teste) ou saída silenciosa. Fica como P1 de disponibilidade, não P0.

**Fixes verificados a taxa ~0:** `All pipe instances are busy` = **0** desde 07-Nov 13:57; fallback para scheduled task = **0** desde 09-Nov; `service not available` = **0** desde 09-Nov; bridge = **27.724 pedidos / 27.725 respostas, 100% `ok=true`**; zero duplicados de processos; handles estáveis (1104 → 1100 em 45 s); firewall **7 regras** (vs 58), zero regras `micontrol_lib-*`.

---

## 1. Ficheiros, sessões e método

### 1.1 Inventário

| Fonte | Cobertura | Nota |
|---|---|---|
| `%LOCALAPPDATA%\MiControl\logs\tauri-app.log.1` | 05-Nov 08:11 → 10-Nov 11:29 | 8 MiB; contém todas as sessões da ronda até à rotação |
| `tauri-app.log` | 10-Nov 11:29 → 18:42 (live) | 8 MiB×7; sessão atual começou a 09-Nov 23:22 e atravessou a rotação |
| `tauri-app.log.2/.3/.4` | até 05-Nov 08:11 / 04-Nov / 03-Nov | só pré-ronda; nenhuma sessão de 09-Nov |
| `%PROGRAMDATA%\MiControl\logs\bridge.log.1` | 09-Nov 09:36 → 10-Nov 17:59 | 27.724 pedidos, 0 falhas |
| `bridge.log` | 10-Nov 17:59 → 18:42 | 422 pedidos, 0 falhas |
| `ecram_service.log` | 07-Nov 18:44 → 10-Nov 01:24 | sem erros; só shutdown/start |
| `watchdog.log` | 07-Nov 13:58 → 09-Nov 23:22 | 3 relançamentos automáticos na ronda |
| `tauri-app-helper.log` | 07-Nov → 10-Nov 18:37 | 274 comandos elevados; 11 `ok=false` (7 pré-ronda, 4 `0x80041002` na ronda) |
| `os_turbo_recovery.json` / `os_turbo_actions.json` | 07-Nov / 10-Nov 16:06 | **não existe `os_turbo*.log`**; estado só em JSON |
| `session.marker` | 09-Nov 23:22:56 | `state=running`, pid=38568, version=0.2.20 |
| Event Log System/Application | últimas 48 h | IDs 1000/1001/7031/7034/1026/41/6008 |
| `%LOCALAPPDATA%\CrashDumps` + `MiControl\crashdumps` + WER | — | ver §5 |

**Método:** parser linha-a-linha (13.614 linhas nas 7 sessões da ronda), split por banner `role=app`; agregação por nível, módulo, assinatura e hora. Bridge: 27.724 pedidos parseados. Sondagem viva: 9 comandos `invoke` read-only via MCP (127.0.0.1:4000), latência medida no WebView e na parede.

### 1.2 Sessões da ronda

| Sessão (UTC) | Origem | Duração | Linhas | WARN | ERROR | WARN/h |
|---|---|---|---:|---:|---:|---:|
| 09-Nov 10:47:38 → 13:11:55 | deploy 10:47 (tray alias) | 2,40 h | 765 | 11 | 0 | 4,6 |
| 09-Nov 13:13:21 → 15:32:54 | deploy 13:13 (startup manager) | 2,33 h | 792 | 17 | 0 | 7,3 |
| 09-Nov 15:34:23 → 16:17:32 | deploy 15:34 (color-online) | 0,72 h | 365 | 22 | **2** | **30,6** |
| 09-Nov 16:17:34 → 16:17:34 | relançamento watchdog | ~0 h | 5 | 1 | 0 | — |
| 09-Nov 16:19:45 → 16:35:07 | 2.º relançamento | 0,26 h | 223 | 12 | 0 | 46,9 |
| 09-Nov 16:36:45 → 23:20:53 | deploy 16:36 (cm-api/WCS) | 6,74 h | 2.930 | 49 | 0 | 7,3 |
| 09-Nov 23:22:56 → 10-Nov 18:42 | relançamento watchdog | 19,33 h | 8.534 | 77 | 0 | 4,0 |
| **Total** | | **34,7 h** | **13.614** | **189** | **2** | **5,4** |

A sessão 15:34 é a única com ERROR e tem a maior taxa de WARN (30,6/h); a sessão 16:19 (arranque falhado + recuperação) tem 46,9/h mas dura 16 min. A sessão atual (19,3 h) é a mais calma em taxa (4,0/h) e tem 0 ERROR.

---

## 2. Achados por severidade

### 2.1 Resumo

| ID | Sev | Módulo | Achado | Ocorr. | Estado |
|---|---|---|---:|---|
| F-01 | **P1** | `hw::crash_recovery`, `util::fixed_pool`, `hw::iot_events` | Runtime starved: 2 ERROR, congelamento 87,7 s, kill do watchdog | 2 ERROR + 8 WARN | **REABERTO** (baseline 0) |
| F-02 | **P1** | `micontrol_bridge` (watchdog) / MCP | Relançamento imediato após kill → app morre no bind da porta 4000 | 1 | **NOVO** (mesma classe de 08-Nov) |
| F-03 | **P1** | `cmd::system` | `get_touchpad_info` timeout 5 s | 5 | CRÓNICO (nova ocorrência) |
| F-04 | **P1** | `cmd::system` | `get_battery_info` timeout 15 s | 4 | CRÓNICO (nova ocorrência) |
| F-05 | **P1** | `elev_bridge` | Comando elevado falha após 30,8 s por `cleanup_stale_elev_files` | 1 | **NOVO** |
| F-06 | **P1** | app / `applog` | Processo desaparece às 23:21 sem marker, WER ou dump | 1 | **NOVO** |
| F-07 | P2 | `hw::battery` | WMI stalls: 111 timeouts, 217 >5 s, 76 >10 s, máx. 67,4 s | 111 WARN | CRÓNICO |
| F-08 | P2 | `hw::battery` | "local AC adapter-power probe skipped for 300s" a cada 5 min | 372 | CRÓNICO/por design |
| F-09 | P2 | `iot_watchdog` / `ecram_service` | IoT pipe indisponível → restart do serviço (~10 s) | 3 | CRÓNICO |
| F-10 | P2 | `hw::charging`, `hw::fn_key`, `hw::iotservice` | EC read failures (0xA4, Fn-lock, wifi_count) | 12 | CRÓNICO |
| F-11 | P2 | vários (arranque) | 6 assinaturas de WARN em cada arranque (BLE, DSDT, IGCL, hotkey, FrameServer, abnormal) | 6×6 | CRÓNICO |
| F-12 | P2 | `elev_bridge` + `tauri_plugin_mcp` | 0x80041002 transitório; `[TAURI_MCP] Error: [object Object]` | 4 + 4 | CRÓNICO |
| F-13 | P2 | `hw::display` | `adaptive_brightness: blocking(config) failed` (sintoma do F-01) | 2 | CRÓNICO |
| F-14 | P2 | `hw::ecram` | ERAM/SMA2 `DriverBlocked` logado a WARN (esperado) | 2 | ESPERADO (nível errado) |
| F-15 | P2 | `commands` / UI | Performance: `get_iot_device_info` 3,4 s; `get_startup_items` 1,15 s | 9/9 probes | WATCH |

### 2.2 F-01 — Runtime starved (P1, REABERTO)

**Evidência (16:07-16:17):**
```
2026-11-09T16:07:41.475Z [WARN ] cmd::system: get_battery_info: failed after 15007 ms: Timeout: blocking hardware operation exceeded 15 seconds
2026-11-09T16:07:42.032Z [WARN ] elev_bridge: Elevated command 'wmi_ec_read_adapter_power' failed after 30796 ms ... cleanup_stale_elev_files task failed: Timeout: blocking hardware operation exceeded 10 seconds
2026-11-09T16:07:46.766Z [ERROR] hw::crash_recovery: async runtime probe failed 2 consecutive times — runtime starved
```
```
2026-11-09T16:07:55.556Z [INFO ] hw::battery: battery WMI query took 44427 ms (slow threshold 1000 ms, ok)
2026-11-09T16:17:30.504Z [ERROR] hw::crash_recovery: async runtime probe failed 2 consecutive times — runtime starved
2026-11-09T16:17:32.670Z (watchdog) heartbeat stale (stale 87733ms) — killing micontrol.exe (pid 37728) and relaunching
```

**Hipótese (alta confiança, confirmada no código):** o probe do runtime não está a mentir. `src-tauri/src/util/fixed_pool.rs:130-163` é uma `async fn` que faz `rx.recv_timeout(timeout)` **síncrono na linha 148**; o mesmo padrão está em `util::blocking` (wrapper, linhas 40-55). Cada comando de hardware que espera pelo pool bloqueia um worker Tokio até 5/15 s. A isto soma-se `src-tauri/src/hw/iot_events.rs:240-247`: o loop é `tauri::async_runtime::spawn(async { ... tick(); })` e `tick()` chama `hw::battery::get_battery_info()` **síncrono** (linha 263), que por sua vez faz `result_rx.recv_timeout(5 s)` em `battery.rs:217` sempre que há query real (a cada 15 s). Com o `slowPoll` do frontend a disparar 5 invokes em paralelo a cada 15 s (`src/hooks/useHardware.ts:246-265`), o runtime fica sem workers exactamente quando o WMI da bateria engasga (query real de 44 s). O `run_blocking` do pool dedicado foi desenhado para isolar o trabalho, mas a **espera** continua no worker async.

**Ficheiro/código sugerido:** trocar o `std::sync::mpsc` por `tokio::sync::oneshot` + `tokio::time::timeout(...).await` em `fixed_pool.rs:141-148`; ou envolver o `recv_timeout` em `tokio::task::spawn_blocking`; e tornar `iot_events::tick()` async com `run_blocking` para a leitura de bateria (ou usar um `peek_cached()` não bloqueante). Reduzir o `slowPoll` a uma chamada serializada (ou `Promise.all` com limite de 2).

### 2.3 F-02 — Relançamento do watchdog morre no bind do MCP (P1, NOVO)

**Evidência:**
```
2026-11-09T16:17:34.676Z [INFO ] tauri_plugin_mcp: [TAURI_MCP] Socket server will start automatically
(gap de 131 s; o processo já não existe)
2026-11-09T16:19:45.544Z [WARN ] applog: previous session ended without shutdown marker — possible crash/kill (session started 2026-11-09T16:17:34.643Z)
```
A sessão das 16:17:34 tem **5 linhas** e pára exactamente antes de `Setting up plugin` / `Initializing TCP socket server`. Não há PANIC no log desta vez, mas o precedente de 08-Nov é explícito:
```
2026-11-08T02:12:24.470Z [ERROR] applog: PANIC: at src\lib.rs:1166:10: error while building MiControl: PluginInitialization("mcp", "IO error: Failed to bind to 127.0.0.1:4000 ... os error 10048")
```

**Hipótese:** `src-tauri/src/bin/micontrol_bridge.rs:2649-2652` faz `kill_app()` e chama `launch_app()` **sem esperar** pela libertação da porta 4000; a app nova tenta o bind no arranque, falha e sai antes de inicializar o crash_recovery (daí não haver PANIC nem dump). A janela de 1,2 s é curta demais para o socket em `TIME_WAIT`.

**Ficheiro/código sugerido:** em `micontrol_bridge.rs` (WatchdogAction::Restart), esperar 3-5 s ou fazer poll activo de `bind(127.0.0.1:4000)` antes de `launch_app`; no lado da app, retry do bind com backoff 250 ms/1 s/2 s (como já existe para o WebView) em vez de panic.

### 2.4 F-03 — `get_touchpad_info` 5 s timeout (P1)

**Evidência:**
```
2026-11-09T15:59:32.187Z [WARN ] cmd::system: get_touchpad_info: failed after 6423 ms: Timeout: blocking hardware operation exceeded 5 seconds
2026-11-09T16:07:34.155Z [WARN ] cmd::system: get_touchpad_info: failed after 5010 ms: Timeout: blocking hardware operation exceeded 5 seconds
2026-11-09T16:16:40.917Z [WARN ] cmd::system: get_touchpad_info: failed after 5008 ms: Timeout: blocking hardware operation exceeded 5 seconds
```
Ocorrências: 13:01:51, 15:59:32, 16:07:34, 16:16:40, 23:01:56 (5). O comando é registry-only (`commands/system.rs:254-267`), pelo que o timeout prova saturação do pool, não lentidão da leitura.

**Hipótese:** o pool fixo de 4 threads está ocupado por jobs de bateria/elevados que esperam 5-30 s; o `run_blocking_timeout(5 s)` do touchpad fica sem slot. **Ficheiro:** `commands/system.rs:263-266` + `util/fixed_pool.rs` (pool 4 / espera bloqueante).

### 2.5 F-04 — `get_battery_info` 15 s timeout (P1)

**Evidência:**
```
2026-11-09T16:07:41.475Z [WARN ] cmd::system: get_battery_info: failed after 15007 ms: Timeout: blocking hardware operation exceeded 15 seconds
2026-11-09T16:19:58.986Z [WARN ] cmd::system: get_battery_info: failed after 5495 ms: Timeout: battery WMI query exceeded 5043 ms ... (no cached snapshot yet)
2026-11-09T16:20:28.383Z [WARN ] cmd::system: get_battery_info: failed after 5039 ms: Timeout: battery WMI query exceeded 5039 ms ... (no cached snapshot yet)
```
4 ocorrências. No arranque das 16:19 não havia snapshot em cache, por isso a UI ficou sem bateria durante os primeiros ~40 s. **Ficheiro:** `commands/system.rs:44-49`; `hw/battery.rs:217-243`.

### 2.6 F-05 — Comando elevado falha após 30,8 s (P1, NOVO)

**Evidência:**
```
2026-11-09T16:07:42.032Z [WARN ] elev_bridge: Elevated command 'wmi_ec_read_adapter_power' failed after 30796 ms (uac_fallback=disabled by design for background commands (MIOT-38)): cleanup_stale_elev_files task failed: Timeout: blocking hardware operation exceeded 10 seconds
```
**Hipótese:** `elev_bridge.rs:361-368` corre `cleanup_stale_elev_files` no pool de bloqueio antes de despachar o comando; com o pool saturado, o cleanup esgota os 10 s e o comando inteiro é abortado — perde-se a leitura de potência do adaptador nesse ciclo. **Ficheiro:** `elev_bridge.rs:355-370`.

### 2.7 F-06 — Saída sem shutdown marker às 23:21 (P1, NOVO)

**Evidência:**
```
2026-11-09T23:20:53.125Z [INFO ] hw::battery: battery WMI query took 5817 ms (slow threshold 1000 ms, ok)
2026-11-09T23:21:26.135Z (watchdog) micontrol.exe NOT running (miss #1)
2026-11-09T23:22:56.580Z [WARN ] applog: previous session ended without shutdown marker — possible crash/kill (session started 2026-11-09T16:36:45.099Z)
```
O processo estava vivo e a escrever logs 33 s antes; o watchdog detectou "NOT running" (não heartbeat stale, não kill do próprio watchdog). **Não há** Event 1000/1001 para `micontrol.exe`, **não há** dump em `%LOCALAPPDATA%\MiControl\crashdumps`, **não há** Event 41/6008. A hipótese mais forte é `TerminateProcess` externo (teste de watchdog/kill manual) ou saída silenciosa sem excepção. **Confirmar com o orchestrator** se houve kill às 23:21; se não, tratar como P0 de crash silencioso. **Ficheiro:** caminho de shutdown em `src-tauri/src/lib.rs` (`RunEvent::Exit`) / `applog` marker.

### 2.8 F-07 — Stalls WMI da bateria (P2, CRÓNICO)

**Evidência:**
```
2026-11-09T22:48:19.033Z [INFO ] hw::battery: battery WMI query took 67376 ms (slow threshold 1000 ms, ok)
2026-11-09T15:59:21.787Z [INFO ] hw::battery: battery WMI query took 23973 ms (slow threshold 1000 ms, ok)
2026-11-10T18:26:04.133Z [WARN ] hw::battery: battery WMI query exceeded 5013 ms (timeout 5000 ms, previous query took 1719 ms) — returning last known-good snapshot
```
Em 34,7 h: **111 WARN de timeout**, **217 queries > 5 s**, **76 > 10 s**, máximo 67,4 s. O stall é transversal a todos os builds (10:47 tem máx. 46 s; 13:13 tem 59,8 s), pelo que **não é regressão desta ronda**. **Ficheiro:** `hw/battery.rs` `get_battery_info_inner` / `wmi_cache`; a query WMI do provider de bateria bloqueia o worker dedicado e os chamadores ficam à espera.

### 2.9 F-08 — Ruído do adapter power local (P2)

```
2026-11-09T16:05:06.416Z [INFO ] hw::battery: local AC adapter-power probe skipped for 300s (Hardware error: Impersonation failed - Access denied (0x80041003)); the elevated bridge path is unaffected and the local probe retries afterwards
```
372 ocorrências; 79 só na sessão 16:36, 224 na sessão atual. É o design actual (probe local sem privilégios falha, backoff de 300 s), mas o log repete a mesma linha completa a cada 5 min para sempre. **Sugestão:** logar 1× por sessão ou a DEBUG; o caminho elevado já fornece o valor.

### 2.10 F-09 — IoT pipe indisponível → restart do ecram_service (P2)

```
2026-11-09T16:39:16.864Z [WARN ] micontrol_lib::hw::iot_watchdog: [iot_watchdog] IoT pipe unavailable — attempting service recovery
2026-11-09T16:39:17.545Z (ecram_service) pipe server shutting down
2026-11-09T16:39:26.750Z (ecram_service) ecram_service starting (v0.2.20, pid 25708)
```
3 ocorrências (16:39, 19:23, 01:24), cada uma com ~10 s de restart. O `ensure_ecram_service` no bridge tem p95 10,4 s (10 chamadas). O self-heal funciona; o custo é a interrupção de 10 s. **Ficheiro:** `hw/iot_watchdog.rs:88-128`; `bridge` `ensure_ecram_service`.

### 2.11 F-10 — EC read failures (P2)

12 ocorrências de:
```
2026-11-09T23:07:24.097Z [WARN ] hw::charging: EC read of 0xA4 failed: Hardware error: Open IoT driver device, falling back to registry
2026-11-09T23:07:27.389Z [WARN ] micontrol_lib::hw::iotservice: [iot] EC command read_wifi_count failed: EC pipe command 'wifi_count' failed: RET timeout for cmd 0x08
2026-11-09T14:47:41.678Z [WARN ] hw::fn_key: EC read failed for Fn-lock: Hardware error: Open IoT driver device, falling back to registry
```
Concentradas em 12:36, 14:46-14:48, 23:07-23:31 e 18:32 (a última foi provocada pela minha sonda `get_ecram_map`). Têm fallback para registry/IoT e não quebram a UI. **Ficheiro:** `hw/charging.rs`, `hw/fn_key.rs`, `hw/iotservice.rs`.

### 2.12 F-11 — Ruído de arranque (P2, CRÓNICO)

Em cada um dos 6 arranques, exactamente as mesmas assinaturas:
```
[ble_advertise] publisher.Start failed: Access is denied. (0x80070005)
DSDT-based ERAM discovery failed, using hardcoded fallback
[igcl_session] ctlInit warning rc=0x40000021 (severity bit clear) — treating as non-fatal
RegisterHotKey Win+Shift+F23 id=104 unavailable: ERROR_HOTKEY_ALREADY_REGISTERED (0x80070581)
[face.camera] FrameServer unavailable — capture may fail
Abnormal restart detected — previous session did not exit cleanly
```
A mais relevante é a **BLE advertise negada**: se a funcionalidade de presença BLE depende de o PC anunciar, ela está morta à nascença nesta instalação. As restantes são fallbacks esperados ou consequência de kills/updates. **Sugestão:** BLE — verificar requisitos/capability do `BluetoothLEAdvertisementPublisher` (0x80070005 é Access Denied, não rádio off); as outras — INFO/1× por sessão, não WARN.

### 2.13 F-12 — 0x80041002 e erro MCP não serializado (P2)

```
2026-11-10T18:36:01.126Z [WARN ] elev_bridge: Elevated command 'wmi_ec_read' failed after 214 ms (uac_fallback=allowed): Hardware error: 0x80041002
2026-11-09T23:42:12.225Z [INFO ] tauri_plugin_mcp::tools: [TAURI_MCP] Error: [object Object]
```
4 falhas elevadas 0x80041002 (WMI provider stall; 2 delas às 18:36, provavelmente de sondas paralelas) e 4 linhas `[object Object]` do plugin MCP, que não serializa o erro. **Sugestão:** no `tauri-plugin-mcp`, `format!("{e:?}")`/`serde_json::to_string` em vez de `Display` de um objeto.

### 2.14 F-13 — Adaptive brightness (P2, sintoma)

```
2026-11-09T15:59:25.450Z [WARN ] micontrol_lib::hw::display: adaptive_brightness: blocking(config) failed
2026-11-09T16:17:26.071Z [WARN ] micontrol_lib::hw::display: adaptive_brightness: blocking(config) failed
```
2 ocorrências, ambas dentro do cluster de starvation; não há evidência de falha independente. **Ficheiro:** `hw/display.rs:1977`.

### 2.15 F-14 — ERAM bloqueada logada a WARN (P2, esperado)

```
2026-11-09T23:31:01.763Z [WARN ] hw::ecram: Region ERAM unavailable (DriverBlocked): ecram_service: DeviceIoControl ECRAM_READ: Access is denied. (0x80070005) | direct IOCTL: Hardware error: Open IoT driver device
```
2 ocorrências (23:31 e 18:32 — a última da minha sonda). É o bloqueio de driver conhecido, classificado `driver_blocked`; a regra de honestidade manda mostrar cartão informativo, não WARN. **Sugestão:** baixar para INFO quando `blocked_kind=driver_blocked`.

### 2.16 F-15 — Performance de comandos (P2, watch)

Sondagem viva: `get_iot_device_info` **3.396 ms** (pipe IoT), `get_startup_items` **1.149 ms** (36 entradas, 13 admin), `get_color_status` 233 ms, `get_fan_info` 38 ms, `get_display_info` 43 ms, `get_battery_info` 11 ms, `get_overview_snapshot` 7 ms. Nenhum falhou. O `get_iot_device_info` é chamado 1×/60 s pelo Overview; se o pool estiver ocupado, contribui para F-01. **Ficheiro:** `commands/hardware.rs:408-411`, `hw/iotservice.rs:1413`.

---

## 3. Regressões por deploy

| Deploy | Commits incluídos | Sessão seguinte | ERROR | Novos sinais | Veredicto |
|---|---|---|---|---:|---|
| **10:47** | `d90c4c8` tray alias cycle | 10:47:38-13:11:55 | 0 | 2 battery timeouts, 1 touchpad, 2 EC read, 1 probe-once (13:07) | **Sem regressão.** Só ruído crónico. |
| **13:13** | `48f5b90` startup manager, `74402fc` ICC path, `c51d280` merge | 13:13:21-15:32:54 | 0 | 7 battery timeouts, 3 EC read, 1 probe-once (15:28), `get_startup_items` 1,15 s | **Sem regressão.** Novo comando pesado (1,15 s) entra no pool; watch. |
| **15:34** | `ae38502` color-online, `4806709` EDID/catalogue, `9cd01f1`, `9ea35e2`, `1fe0d6e` | 15:34:23-16:17:32 | **2** | 4 probe-once, 3 touchpad, 2 battery_info, 1 elev 30,8 s, 2 adaptive brightness; termina com kill do watchdog | **Pior sessão da ronda (P1).** O defeito de blocking é anterior (S68), mas foi aqui que o stall WMI de 44 s o expôs. Não atribuir ao color-online sem prova. |
| **16:36** | `7d91b0c` WCS/mscms, `b441556` merge cm-api | 16:36:45-23:20:53 | 0 | 36 battery timeouts, 2 IoT recovery, 1 touchpad, 2 EC read; saída sem marker às 23:21 | **Estabiliza o starved (0 ERROR em 6,7 h)**, mas o defeito estrutural continua e a saída das 23:21 não está explicada. |
| (relançamento) | mesmo build 16:36 | 23:22:56-18:42 | 0 | 57 battery timeouts, 1 IoT recovery, 3 elev 0x80041002, 5 EC read | **19,3 h estável**, 0 ERROR. |

**Conclusão:** nenhum dos 4 deploys introduziu uma assinatura de erro *nova* no código; o que mudou foi a **exposição** de um defeito pré-existente (bloqueio do runtime) sob um stall WMI. O build 16:36 é o mais estável da ronda (0 ERROR em 26,1 h combinadas), mas a taxa de timeouts de bateria mantém-se.

---

## 4. Assinaturas e taxas vs baseline

### 4.1 Contagem por assinatura e sessão

| Assinatura | 10:47 | 13:13 | 15:34 | 16:17 | 16:19 | 16:36 | 23:22 | Total |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| ERROR runtime starved | 0 | 0 | **2** | 0 | 0 | 0 | 0 | **2** |
| WARN probe failed once | 1 | 1 | 4 | 0 | 0 | 2 | 0 | 8 |
| WARN battery timeout | 2 | 7 | 4 | 0 | 5 | 36 | 57 | 111 |
| WARN touchpad 5 s | 1 | 0 | 3 | 0 | 0 | 1 | 0 | 5 |
| WARN battery_info 15 s | 0 | 0 | 2 | 0 | 2 | 0 | 0 | 4 |
| WARN elev_bridge failed | 0 | 0 | 1 | 0 | 0 | 0 | 3 | 4 |
| WARN adaptive brightness | 0 | 0 | 2 | 0 | 0 | 0 | 0 | 2 |
| WARN IoT pipe recovery | 0 | 0 | 0 | 0 | 0 | 2 | 1 | 3 |
| WARN EC read fail | 2 | 3 | 0 | 0 | 0 | 2 | 5 | 12 |
| WARN BLE advertise denied | 1 | 1 | 1 | 0 | 1 | 1 | 1 | 6 |
| WARN DSDT fallback | 1 | 1 | 1 | 0 | 1 | 1 | 1 | 6 |
| WARN IGCL ctlInit | 1 | 1 | 1 | 0 | 1 | 1 | 1 | 6 |
| WARN FrameServer | 0 | 1 | 1 | 0 | 1 | 1 | 0 | 4 |
| WARN hotkey already reg | 1 | 1 | 1 | 0 | 1 | 1 | 1 | 6 |
| WARN abnormal restart | 1 | 1 | 1 | 0 | 1 | 1 | 1 | 6 |
| WARN no shutdown marker | 0 | 0 | 0 | 1 | 1 | 0 | 1 | 3 |
| INFO adapter probe skip | 29 | 28 | 9 | 0 | 3 | 79 | 224 | 372 |
| INFO battery query >5 s | 2 | 8 | 5 | 0 | 7 | 66 | 129 | 217 |
| INFO battery query >10 s | 2 | 5 | 3 | 0 | 3 | 18 | 45 | 76 |
| INFO MCP `[object Object]` | 0 | 0 | 0 | 0 | 0 | 0 | 4 | 4 |

### 4.2 Taxas vs baseline pré-fix

| Métrica | Baseline (07-Nov, 11 h / 5.000 linhas) | Ronda atual (34,7 h) | Veredicto |
|---|---:|---:|---|
| `All pipe instances are busy` | 1.030 (~1,57/min) | **0** desde 07-Nov 13:57 | **Verificado (~0)** |
| Helper por leitura (fallback task) | 513 (~0,78/min) | **0** desde 09-Nov | **Verificado (~0)** |
| `service not available` loop | 515 | **0** desde 09-Nov | **Verificado (~0)** |
| `runtime starved` | 0 (pós-fix) / 666 (episódio 06-Nov) | **2 ERROR + 8 WARN** (0 desde 16:36) | **NÃO verificado** |
| Battery slow/timeout | 312/5.000 linhas | 111 timeouts + 217 >5 s / 13.614 linhas | Persiste; sem ~0 |
| Bridge | morto desde 02-Nov | 27.724/27.725 `ok=true`, 0 falhas, 0 gaps >5 min | **Verificado** |
| Handles da app | 727 → 930 (1.º min) | 1.104 → 1.100 em 45 s (sem crescimento) | Sem leak na amostra; watch |
| Watchdog | desarmado | armado; 3 relançamentos, 1 kill de freeze | Funciona, mas F-02 |

**Leitura:** os fixes de pipe/helper/serviço estão genuinamente a ~0. O `runtime starved` não está; a taxa é 0 nas últimas 25 h apenas porque não houve um stall WMI suficientemente longo com carga. O critério "taxa ~0" não se aplica à bateria.

---

## 5. Event Log, crash dumps e serviços

### 5.1 Event Log (janela 48 h)

| ID | Ocorrências MiControl | Correlação | Leitura |
|---|---:|---|---|
| 7034 (Bridge) | 4 | 10:47:05, 13:12:47, 15:33:49, 16:36:12 | Sempre 1-2 min antes do banner da app = **installer a matar o serviço**, não crash espontâneo |
| 7031 (Face) | 8 | 10:47:10/27, 13:12:52/13:13:10, 15:33:54/15:34:12, 16:36:17/34 | 2 por deploy: kill do installer + 2.ª terminação após restart; depois estável 26 h |
| 7034 (IoT) | 1 | 09-Nov 00:53:36 | Deploy anterior (00:54), não esta ronda |
| 1000 (Application Error) | **0** | — | O único 1000 é OneDrive.Sync.Service às 12:42 de 10-Nov |
| 1001 (WER) | **0** | — | Os 1001 são `RADAR_PRE_LEAK_64` do comet.exe (10-Nov 15:09) e python.exe (09-Nov 10:13) + APPCRASH OneDrive (10-Nov 12:42) |
| 1026 (.NET) | **0** | — | — |
| 41 / 6008 | **0** | — | Sem shutdown inesperado do sistema |
| 7036 (serviço) | — | — | Não há 7036 para MiControl; os 2097/2099 do firewall mostram as 5 regras recriadas em cada deploy |

**Application log:** Circular, 20 MB, 1.135 registos, 1,1 MB usados, última escrita 18:35 — **não está cheio**; os eventos recentes estão presentes. System log: 43.548 registos, 20,97 MB (no limite, circular).

### 5.2 Crash dumps

| Local | Conteúdo | MiControl? |
|---|---|---|
| `%LOCALAPPDATA%\CrashDumps` | OneDrive 10-Nov 12:42 (18,9 MB), powershell 07-Nov (2×), FileCoAuth, svchost, SnippingTool, SearchHost | **Não** |
| `%LOCALAPPDATA%\MiControl\crashdumps` | vazio | — |
| WER ReportArchive/Queue | nenhum relatório MiControl | — |

Não há **nenhum** minidump de `micontrol.exe` em todo o período. O `hw::crash_recovery` regista os dumps em `MiControl\crashdumps`, mas a pasta está vazia — coerente com a ausência de excepção nativa. O evento das 23:21 é, portanto, uma saída/kill, não um crash WER.

### 5.3 Serviços e watchdog

| Serviço | Estado | PID | Conta |
|---|---|---|---|
| MiControlBridge | Running / Auto | 26184 | LocalSystem |
| MiControlFace | Running / Auto | 56640 | LocalSystem |
| IoTSvc | Running / Auto | 55340 | LocalSystem |

- `MiControlElevated`: **Ready**, último resultado **0** (10-Nov 18:37:58).
- `watchdog.log`: armado em todos os deploys; **3 relançamentos** na ronda: 16:17:32 (kill de freeze), 16:19:45 (após morte ao arrancar), 23:22:56 (processo desaparecido). Nenhum "give up".
- Sem duplicados: 1 `micontrol.exe`, 1 `micontrol_bridge.exe`, 1 `micontrol_face_svc.exe`, 1 `IoTService.exe`. `micontrol.exe` é filho do bridge (PID 26184), não órfão.
- Handles: app 1.104 → 1.100 em 45 s (threads 64 → 62, WS 48,8 MB); bridge 231 → 229; IoT 124. Sem crescimento na amostra.

---

## 6. Sondagem viva (MCP, read-only)

| Comando | Parede | Interno | Resultado |
|---|---:|---:|---|
| `get_overview_snapshot` | 35 ms | 7 ms | OK — XIAOMI Xiaomi Book Pro 14, Ultra X7 358H, Arc B390, NPU Intel, 32 GB, 1,14 GB livres em C: |
| `get_battery_info` | 35 ms | 11 ms | OK — 95%, plugged, **not charging**, 100 W AC, 17,595 mV, health 93,3% (fallback de fábrica) |
| `get_fan_info` | 58 ms | 38 ms | OK — CPU **95 °C**, GPU 67 °C, ambient 27,85 °C, 51,3 W, mode auto, RPM null (sem sensor) |
| `get_color_status` | 239 ms | 233 ms | OK — 3 displays, EDO 0x4503 / GSM 0x7770 / GSM 0x774A, catálogos 3/2/2 |
| `get_startup_items` | 1.165 ms | 1.149 ms | OK — 36 itens: 28 on / 8 off / 13 admin / 0 missing / 3 bloqueados (entradas de pasta, não atalhos) |
| `get_ecram_map` | 50 ms | 25 ms | OK mas `available=false`, `ERAM`+`SMA2` **driver_blocked** (esperado) |
| `get_os_turbo` | 25 ms | 1 ms | OK — enabled, 14 throttled, Best Performance, failures 0, worker active |
| `get_display_info` | 56 ms | 43 ms | OK — brightness 10, HDR on, 120 Hz, 3 displays |
| `get_iot_device_info` | **3.399 ms** | **3.396 ms** | OK — pipe available, `xiaomi.laptop.p52`, fw 1.0.3_0010, online, wifi_status 3, SSID null |

Nenhum comando devolveu erro. Observações: CPU a 95 °C / 51 W no momento da sonda (sem erro de hardware, mas no limite); `get_iot_device_info` é o comando mais lento (3,4 s) e contribui para a ocupação do pool.

### 6.1 Firewall

- **7 regras** mencionam `micontrol` (contagem à data): 5 do instalador (`MCP TCP 9223`, `LocalSend TCP/UDP 53317`, `KDE Connect UDP 1716`, `block public`) + **2 stale** de um caminho debug `C:\users\mafsc\...\target\debug\micontrol.exe`.
- **Zero** regras `micontrol_lib-<hash>.exe` — as 58 antigas (56 dev + 2 instaladas) foram limpas. **Fix verificado.**
- O Event Log do firewall mostra as 5 regras recriadas em cada deploy (2097 às 10:47:28, 13:13:11, 15:34:13, 16:36:35). Durante a ronda, builds dev criaram regras `micontrol_lib-*` (11:19, 13:01, 14:25, 15:23, 16:26), mas já não existem. Se voltar a haver builds dev, as regras reaparecem.

---

## 7. O que está verificado (taxa ~0) e o que não está

**Verificado:**
- Pipe busy: 0 desde 07-Nov 13:57 (baseline 1.030).
- Fallback para scheduled task: 0 desde 09-Nov (baseline 513).
- `service not available`: 0 desde 09-Nov (baseline 515).
- Bridge: 27.724 pedidos / 27.725 respostas, 0 `ok=false`, 0 gaps >5 min.
- Firewall stale: 0 regras `micontrol_lib-*` (7 no total, 2 debug).
- Processos: sem duplicados, sem helpers órfãos; handles estáveis.
- Sondagem viva: 9/9 comandos OK.

**Não verificado:**
- `runtime starved` a ~0: 2 ERROR + 8 WARN; a condição estrutural (bloqueio do runtime) continua no código. 0 nas últimas 26 h é sorte de carga, não prova.
- Bateria sem timeouts: 111 WARN + 217 queries >5 s; a taxa não é ~0.
- Saída limpa: a sessão 16:36 terminou sem marker (23:21); não há prova de crash, mas também não há prova de quit limpo.
- `load_icc_profile` / color: não foi objecto desta ronda; a sonda `get_color_status` só lê estado.

---

## 8. Acções recomendadas (por prioridade)

1. **P1 — corrigir o blocking do runtime (F-01).** `util/fixed_pool.rs:141-148`: substituir `std::sync::mpsc::recv_timeout` por `tokio::sync::oneshot` + `tokio::time::timeout(...).await`; ou `spawn_blocking` para a espera. Fazer o mesmo em `util::blocking`. Sem isto, o starved volta no próximo stall WMI.
2. **P1 — tirar a bateria do task Tokio (F-01).** `hw/iot_events.rs:240-247/263`: tornar `tick()` async e usar `run_blocking`, ou ler um snapshot cacheado não bloqueante; o listener de 5 s não pode chamar `get_battery_info()` síncrono.
3. **P1 — eliminar a corrida de relançamento (F-02).** `bin/micontrol_bridge.rs:2649-2652`: após `kill_app()`, esperar pela libertação de `127.0.0.1:4000` (poll bind ou 3-5 s) antes de `launch_app`; na app, retry do bind do MCP com backoff em vez de panic.
4. **P1 — aliviar o pool e o slowPoll (F-03/F-04/F-05).** `useHardware.ts:246-265`: serializar/limitar o `Promise.all` de 5 invokes; `fixed_pool.rs`: POOL_SIZE 4 → 8 ou pool separado para leituras registry-only; `elev_bridge.rs:355-370`: não condicionar o comando ao cleanup (ou cleanup fora do caminho crítico).
5. **P2 — cortar ruído (F-07/F-08/F-14).** `battery.rs:126`: subir `BATTERY_SLOW_QUERY_LOG_MS` para 2.500-3.000 ms (a latência normal é 1,2-1,6 s) e logar timeouts com rate-limit; `adapter_skip` 1×/sessão; ERAM `DriverBlocked` a INFO.
6. **P2 — investigar a saída das 23:21 (F-06).** Confirmar com o orchestrator se houve kill externo; se não, instrumentar `RunEvent::Exit`/`process::abort` e garantir dump WER antes de sair.
7. **P2 — BLE advertise (F-11).** `ble_advertise.rs:189`: 0x80070005 em todos os arranques; verificar capability/permissões do `BluetoothLEAdvertisementPublisher` (não é "rádio off").
8. **P2 — observabilidade.** Serializar erros do MCP (`[object Object]`), criar `os_turbo*.log` (hoje só há JSON), e alinhar o log de `read_wifi_count`/EC read com retry.

---

## 9. Anexos

- App logs: `%LOCALAPPDATA%\MiControl\logs\tauri-app.log.1` (sessões 10:47-23:22) e `tauri-app.log` (23:22 → live).
- Bridge: `%PROGRAMDATA%\MiControl\logs\bridge.log.1` + `bridge.log`.
- Serviço/watchdog: `ecram_service.log`, `watchdog.log`, `tauri-app-helper.log`, `session.marker`.
- Baseline: `audit-20261107/baseline-pre-fix.json` (`runtime_starved: 0`, `pipe_busy: 1030`, `helper_spawns: 513`, `battery_slow: 312`, `svc_not_available: 515`).
- Pós-fix: `audit-20261107/POST-FIX-REPORT.md` (0 starved na sessão de 07-Nov).
- Re-scan anterior: `audit-20261107/logs-rescan.md` (já registava stall de 44,8 s em 08-Nov e o PANIC de bind).
- Sondas vivas: `%TMPDIR%\live_probes.json` (9 comandos, latências e resultados).

---

# ADIÇÃO PÓS-FIX (deploy 10-Nov 20:17) — o que foi corrigido e verificado

## Modos de performance (f9388fe)
- **Causa**: WMAA FUN2=0x0800 só escreve QFAN (EC RAM 0x68) para 2/3/4/9/0x0A; os valores 5..8 escrevem SMMT/SMMD e nunca tocam no QFAN; SGER=0x8000 incondicional = "success" falso. QFAN=9 = SmartAdaptive, não Ultra.
- **Fix**: EcActiveMode + escrita verificada (before/after, só applied se latchou) + fallback SSDB→QFAN + leitura honesta na UI ("EC active mode: smart_adaptive raw 9").

## Ventoinha com tampa fechada (fcd4535)
- **Causa**: a EC troca para tabela clamshell ao fechar a tampa — firmware; nenhum software desta máquina escreve fan duty.
- **Fix**: listener GUID_LIDSWITCH_STATE_CHANGE (log [lid] CLOSED/OPEN + snapshot térmico), lid_open na FanInfo, aviso honesto na UI.
- **Live**: lid_open=null (ainda sem notificação — honesto); a confirmar pelo dono ao fechar a tampa.

## Proteção de carga (d668f49)
- **Causa tripla**: ECRAM_WRITE 0xFE0B03A4/0xA7 negada (0x80070005 mesmo a SYSTEM) com fallback silencioso; protocolo real = WMAA 0x1000/0x02 com CÓDIGO de nível (1=80%); e wmi_ec_set_brightness_data escrevia o MESMO par (brilho 5..8 reescrevia o limite 70/60/50/40%).
- **Fix**: escrita WMAA + read-back obrigatório; get_charge_limit_state; allowlist (0x1000,0x02); brilho removido; UI "Paused" só com applied=true.
- **Live pós-deploy**: requested=80 → hardware=80, code=1, applied=true, source=firmware. Bateria 94%, charging=false (EC segura; retoma abaixo de 80).

## Estabilidade (6 commits, 9bc780f..59a928b)
- Pool async (tokio oneshot+timeout, 8 threads), bateria fora da task async (budget 3 s), slowPoll limitado (2+jitter+cancelamento), watchdog espera o :4000 (probe de bind, backoff, budget 10 s), BatteryGate circuit breaker (30/60/120/240 s), touchpad 2 s, startup budget 5 s. hardware.ts JSDoc corrigido (20 erros tsc no merge).
- 917 testes lib + 24 bridge; gate 12/12.
