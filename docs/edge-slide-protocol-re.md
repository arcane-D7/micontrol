# Edge slide — protocolo real (engenharia reversa) e onde o nosso código se perdia

**Data:** 2026-11-08 · **Âmbito:** BLTP7853 (touchpad do Xiaomi Book Pro / TM2424), MiControl 0.2.20
**Fontes lidas (fora do repo):** `~/Documents/Projects/miPC/SvrCModule.dll_strings.json`,
`~/Documents/Projects/miPC/_re_touchpad.py`, `~/Documents/Projects/miPC/RELATORIO_ESTADO_ATUAL.md`,
`~/Documents/Projects/miPC/analysis_binaries/SYSTEM_INVESTIGATION_REPORT.md`,
`~/Documents/Projects/miPC/micontrol/sprint-planning/sprint-01-touchpad-charger-fix/sprint.md`
(S1-003 DSDT/SSDT, S1-009), `~/Documents/Projects/miPC/audit-20261107/logs-rescan.md`,
`audit-20261107/func-audit/display.md`.

---

## 1. TL;DR

1. **Não há protocolo de EC nem registo de hardware para o edge slide.** O gesto é
   reconhecido em *user space* a partir dos relatórios HID padrão (PTP) do touchpad.
   Nenhum registo EC, nenhum output report de "activar gesto".
2. A Xiaomi **divide o gesto em dois processos**: `PluginGesture.dll` reconhece e
   publica um `GesturePipeCmd` por named pipe; `MiHygieneBroker.exe` executa a acção
   (brilho/volume). `SvrCModule.dll` só guarda a flag.
3. A flag de activação é **registry-only**: valor `EdgeSlide` (o nosso
   `HKCU\Software\MiControl\Touchpad\EdgeSlide`). O `SvrCModule` regista
   `get/set_touchpad_edge_sliding` e **não** emite `SetOutputReport` para o efeito.
4. O único efeito *no dispositivo* por cada passo do gesto é um **tick háptico**:
   frame HID `0D 09 FD 01 00 03 00 00 02 32 CE` (cmd `0x02`, payload `[0x32, 0xCE]`).
5. **O nosso bug não era o reconhecimento — era o routing.** O gesto era reconhecido
   (`edge-slide captured`, 54× no log) mas a acção nunca corria: **zero** linhas
   `edge-left steps=` em todo o histórico. Duas causas, ambas no `handle_edge_slide`
   (ver §4).

---

## 2. O evento: que relatório HID, que campos

O touchpad expõe 5 colecções HID. As que interessam:

| Colecção | Usage Page | Conteúdo |
|---|---|---|
| COL01 | 0x0001 (Generic Desktop) + 0x000D (Digitizer) | **PTP**: Contact Count, X/Y, TipSwitch |
| COL04/COL05 | 0xFF00 / 0xFF01 (vendor) | diagnóstico/tuning do controlador — não usar para gestos |

Layout efectivo do relatório de input (confirmado pelo `HidP_*` em produção e pelos
testes S1-002):

| Campo | Usage Page / ID | Link Collection | Notas |
|---|---|---|---|
| Contact Count | `0x000D` / `0x0054` | 0 | nº de contactos no relatório |
| X | `0x0001` / `0x0030` | 1..5 | **um por contacto** |
| Y | `0x0001` / `0x0031` | 1..5 | idem |
| TipSwitch | `0x000D` / `0x0042` | 1..5 | dedo fisicamente em contacto |
| Confidence | `0x000D` / `0x0047` | 1..5 | presente no descriptor; não usado |

Intervalo lógico desta unidade (log de 13:26:38):
`x_max = 3282`, `y_max = 2124` (≈ 10.5 × 6.8 cm → ~312 unidades/mm em X, ~312 em Y).

Consequências de desenho que o código tem de respeitar:

- **X e Y têm de vir do mesmo link collection** — se não, misturam-se coordenadas de
  contactos diferentes. (Já estava corrigido.)
- **Os slots não usados mantêm coordenadas velhas.** Ler "o primeiro LC com x>0 && y>0"
  devolve, ao frame seguinte, o contacto errado. É a origem das linhas impossíveis do
  log: gesto do lado **esquerdo** com `x=3240/3282` (borda direita).
- **O relatório pode trazer vários `dwCount`** no mesmo `WM_INPUT`; processar só o
  primeiro descarta amostras e transforma movimento normal em "saltos".
- Não há eixo de "zona"; a noção de bordo é **nossa** (era `x < x_max/8` para
  iniciar e `x < x_max/3` para continuar — valores **substituídos** em §8 pelo que
  o `PluginGesture.dll` realmente usa: banda de 4 % + pré-rolo de 250 unidades).

## 3. O que a Xiaomi faz (cadeia real)

```
BLTP7853 (I2C5) ──HID input report──► PluginGesture.dll
                                        │  reconhece o swipe vertical no bordo
                                        │  (mesma matemática: zona + delta de Y)
                                        ├─► frame HID 0x02 (tick háptico)  ──► touchpad
                                        └─► GesturePipeCmd
                                              \\.\pipe\...  (mi_ipc / HygienePipeClient)
                                                   │
                                                   ▼
                                            MiHygieneBroker.exe
                                                   ├─► brilho (WMI/IGCL)
                                                   └─► volume (Core Audio)
```

Evidência (strings de `SvrCModule.dll`):

- `hygiene::HygienePipeClient::PostGestureCmd(const class GesturePipeCmd &)`
- `gesture.GestureCmd`, `gesture.SetTouchpadEdgeSlidingRequest`,
  `gesture.GetTouchpadEdgeSlidingRequest`, `gesture.SetTouchpadGesturesScreenshotRequest`
- `sc::TouchSettingModule::Get/SetTouchpadEdgeSliding` → "Send get/set touchpad edge
  sliding request" (JSON, sem output report)
- `EdgeSlide` (nome do valor de registry)

O `SvrCModule` **não** escreve no EC para isto, e o gesto **não** passa pelo
`IoTService`/`IoTDriver`. Confirmado também do lado ACPI (S1-003): o bit EC `TPEN`
(ERAM `0x1A`) está definido no DSDT mas **nenhum método o referencia**, e o `_PS0` do
I2C5 está vazio. Não há caminho firmware para "edge slide".

### 3.1 O tick háptico por passo (formato do frame)

```
0D 09 FD 01 00 03 00 00 02 32 CE
│  │  │  │  │  │  │  │  │  └── payload[1]
│  │  │  │  │  │  │  │  └───── payload[0] = força
│  │  │  │  │  │  │  └──────── cmd = 0x02 (click único)
│  │  │  │  │  │  └─────────── reservado
│  │  │  │  │  └────────────── payload_len + 1
│  │  │  │  └───────────────── fase: 0x01 = commit (0x00 = primeiro frame)
│  │  │  └──────────────────── checksum = XOR(bytes[3..31]) + 1
│  │  └─────────────────────── len = payload_len + 7
│  └────────────────────────── magic
```

Este frame é o mesmo que o `build_commit_frame`/`build_touchpad_report` do repo
produzem, e está coberto pelos testes `issue106_single_haptic_click_frame_checksum`.
**Não é necessário para a função** — o brilho/volume funcionam sem tick — mas é o que
dá a sensação táctil por passo e é a prova de que cada passo deve gerar exactamente
um evento.

### 3.2 Registo de configuração

| Onde | Valor | Tipo | Significado |
|---|---|---|---|
| Registry | `EdgeSlide` | DWORD | 0/1 — liga o gesto |
| Registry | `GesturesScreenshot` | DWORD | idem, para os 5 dedos |

O nosso código usa `HKCU\Software\MiControl\Touchpad\EdgeSlide`
(`TP_REG_EDGE_SLIDE` em `hw/touchpad.rs`), lido no arranque para
`EDGE_SLIDE_ENABLED`. Como é só registry, ligar/desligar em sessão funciona de
imediato (não há handshake com o dispositivo).

---

## 4. Porque é que "disparava mas não fazia nada"

Cadeia real no nosso binário:

```
WM_INPUT → process_raw_input → [parse HID] → handle_edge_slide → (brightness, volume)
                                                                      │
                                        simulate_brightness_* / simulate_volume_*
```

A última caixa está bem: `simulate_volume_*` injecta `VK_VOLUME_UP/DOWN` por
`SendInput`; `simulate_brightness_*` usa `display::set_brightness` + OSD. O problema
estava **antes**, em duas linhas do `handle_edge_slide`:

### Causa A — o acumulador nunca enchia (deadband ≥ passo)

```rust
// antes
let deadband = charger_aware_y_deadband(state.y_max).min(y_step / 2);
```

Com `y_max = 2124`:

| Grandeza | Valor | Efeito |
|---|---|---|
| `y_step` | `2124/24 = 88` | precisa de 88 unidades para 1 passo |
| deadband (sem carregador) | `2% × 2124 = 42`, cap `88/2 = 44` | **42** |
| movimento real por frame a 250 Hz | 15–30 unidades | **descartado como ruído** |

Ou seja: **todo** o movimento real de um swipe normal era rejeitado pelo deadband.
O `captured` acontecia (usa deslocamento total, não por frame) e a partir daí nenhuma
amostra entrava no `accum`. Prova nos logs: 54 `edge-slide captured`, **0**
`edge-left steps=` / `edge-right steps=` em 11 h de sessão.

Correcção: `min(y_step / 6)` (= 14 nesta unidade). O deadband continua a existir para
ruído EMI, mas já não engole movimento legítimo.

### Causa B — re-armar de uma sessão morta (o dilúvio de 3 167 `ended`)

No ramo `state.edge == None`, o código usava `edge_contact_start_side` — o lado onde o
**contacto começou** — sem verificar onde o dedo estava **agora**. Depois do primeiro
`ended`, `edge_contact_start_side` continuava `Some(Left)`, e cada frame seguinte:

1. re-armava com `start_x = x` (já no meio do pad);
2. no frame seguinte, `x > x_max/3` → `ended`;
3. repetia.

Resultado: 3 167 `ended` para 54 `captured`, com `start_x` absurdos (daí os
`total_dx=-3200` nos `captured`). O gesto morria antes de o acumulador chegar a 88.

Correcção: re-armar **só** se o dedo ainda estiver na zona de iniciação
(`edge_side_of(x) == session_side`), e limpar `edge_contact_start_side` quando o gesto
termina por sair da zona.

### Causa C (contribuinte) — slots trocados

O `first_lc` escolhia o primeiro slot com `x>0 && y>0`, sem `TipSwitch` nem
continuidade. Um frame lia o outro contacto e o gesto terminava (ou capturava com
`total_dy=403, total_dx=-3200`). Correcção: `select_contact()` prefere slots com
`TipSwitch` e, entre esses, o mais próximo da posição anterior; coordenadas fora do
intervalo lógico são rejeitadas.

### Correcções aplicadas (resumo)

| # | Mudança | Teste |
|---|---|---|
| A | deadband `min(y_step/6)` | `edge_slide_real_geometry_emits_steps_at_250hz` (falha com `/2`: "got 0") |
| B | re-armar só na zona de iniciação + limpar sessão | `edge_slide_does_not_rearm_after_leaving_the_zone` |
| C | `select_contact` com TipSwitch + continuidade | `edge_slide_survives_a_contact_slot_shuffle` |
| D | validar coordenadas contra `x_max/y_max` | `edge_slide_ignores_out_of_range_coordinates` |
| E | debounce de 3 frames antes de terminar | idem B/C |
| F | `ended` só em INFO se `captured`; `armed` em DEBUG | ruído de log (348/10 min → ~0) |
| G | decaimento do acumulador 25% → 12.5%/frame | A |

---

## 5. A dica do GitHub (issue #106, Oksion)

`git log --all --grep=issue` → commit **`02fab88` "fix(#106): touchpad HID protocol —
checksum, phase flag, commit frame"**. O Oksion (mantenedor do XiControl) verificou
seis correcções **no hardware**, lendo os valores de volta:

1. checksum = `XOR(bytes[3..31]) + 1` (não só o payload) — com o XOR antigo o
   firmware **ignorava** o frame em silêncio;
2. fase: primeiro frame `0x00`, commit `0x01` (estava invertido);
3. frame de commit obrigatório (`cmd 0x01`, payload `[cmd, 0x100-cmd]`) — sem ele o
   firmware nunca aplica;
4. `0x5D` High = `(104, 128)`, não `(104, 104)`;
5. `0x59` payload = um LE short (`01 00` / `02 00`), não um byte;
6. 120 ms entre frames (a ~10 ms a leitura de volta dá zeros).

A lição transferível para o edge slide é a **metodologia**, não os bytes: neste
touchpad, uma escrita que é ignorada **não devolve erro**. Foi exactamente o que
aconteceu aqui — `handle_edge_slide` devolvia `(0,0)` sem erro, sem log e sem acção.
Por isso a correcção inclui telemetria obrigatória (`armed`/`captured`/`steps` com
`y_step` e `deadband` na mesma linha) em vez de confiar em "não deu erro".

## 6. O que falta validar no hardware

1. Swipe no bordo esquerdo → esperar `edge-slide armed` (debug) → `captured` →
   `edge-left steps=1` a cada ~1 cm.
2. Se aparecer `contact switch (dx=...)` com frequência, o `select_contact` ainda
   está a escolher o slot errado: subir `RUST_LOG=hw::touchpad=trace` e ver
   `contact pick: lc=... candidates=...`.
3. Se `captured` acontecer mas `steps` não, o `deadband`/`y_step` impressos na linha
   de `captured` dizem qual dos dois está a travar.
4. Confirmar que o gesto **não** dispara com dois dedos (o `contact_count != 1`
   continua a bloquear) nem a partir do centro.

---

## 7. Verificação ao vivo do lado BLE (mesma sessão)

`cargo test --lib ble_presence_passive_scan -- --ignored --nocapture` (uma única
passagem de scan, sem lock):

```
[ble_presence.test] looking for D8803CD01CA4 (D8-80-3C-D0-1C-A4)
[ble_presence.test] 12 peripheral(s) seen:
  D8:80:3C:D0:1C:A4    name=Some("Amazfit Balance 2")
  FC:43:45:81:4B:9A    name=None
  ...
[ble_presence.test] matched D8:80:3C:D0:1C:A4 (Some("Amazfit Balance 2"))
```

Duas conclusões:

1. O dispositivo vinculado é **o relógio**, não o telemóvel — o MAC configurado
   (`D8-80-3C-D0-1C-A4`) e o nome (`Amazfit Balance 2`) são do relógio. O lock
   dependia, portanto, do RSSI de um dispositivo *wearable* que passa a maior parte do
   tempo ligado por BLE ao telemóvel e deixa de anunciar.
2. `FC:43:45:81:4B:9A` — o MAC antigo que o audit interpretou como "rotação de
   endereço" — **continua a anunciar** e é o telemóvel (sem `local_name`). Não houve
   rotação: houve uma re-vinculação do telemóvel para o relógio. Isto também explica
   porque o fallback por nome nunca teria salvado o caso antigo (o telemóvel não
   anuncia nome).

---

## 8. O reconhecedor original, desmontado (2026-11-08)

Até aqui o RE tinha sido feito sobre *strings* do `SvrCModule.dll`. Esta secção vem
do binário que **faz** o gesto: `C:\MI_Backup\XiaomiPCManager\5.8.0.57\Plugins\PluginGesture.dll`
(244 952 bytes, PDB `...\src\Plugins\PluginGesture\plugin_gesture.cpp`) e
`MiTouchGesture.dll` (50 392 bytes), desmontados com capstone. Todos os endereços
abaixo são RVA + image base `0x180000000`, build 5.8.0.57.

### 8.1 Não existe "zona de gesto" no controlador — 0x5D é o motor háptico

A hipótese de trabalho era: *0x5D configuraria a banda de gesto no controlador, e
por isso o rato não anda no original*. **É falsa, e o binário mostra porquê.**

| Report | Quem usa | Payload | Significado |
|---|---|---|---|
| `0x02` | `PluginGesture!0x180018750` | `[0x32, 0xCE]`, fase `01`, **um só frame** | tick háptico por passo |
| `0x58` | `PluginGesture!0x18001972e` | `[0x05]` | `WriteMotorSildeGears` (força 80) |
| `0x5B` | `PluginGesture!0x18001968b` | `01 00 02 00` | `WriteForceGears` (sensibilidade) |
| `0x5D` | `SvrCModule` (e nós) | 2 shorts LE, ex. `(80,104)` | **`WriteMotorGears`** — força do motor esquerdo/direito |
| `0x59` | `SvrCModule` | LE short `01 00`/`02 00` | `WriteEnableHeavyPress` |

- `0x5D` `High = (104, 128)` é o **par de gears dos dois motores** (issue #106), não
  uma largura de banda. `(104, 104)` era ignorado porque o firmware descarta o
  segundo short quando é igual ao primeiro — daí a correcção do Oksion. Não há
  nenhum campo de posição/largura em 0x5D.
- **Não existe `HidD_SetFeature` em lado nenhum.** As importações de
  `PluginGesture.dll` são `HidD_SetOutputReport` e `HidD_GetInputReport` (mais
  `HidP_GetUsages`/`HidP_GetUsageValue`/`HidD_GetPreparsedData`). O
  "Feature report 0x5D" do issue #106 é uma imprecisão de nomenclatura: 0x5D é um
  **Output report**, tal como o nosso `send_touchpad_command` já o envia.
- `WriteMotorSildeGears` (0x58) é o único sítio onde aparece o `80` de
  *"Ajust slide gears to 80"*: o original lê o report com `HidD_GetInputReport`,
  compara `byte[12] == 0x50` e, se não bater, escreve 0x58 `[0x05]`. É afinação da
  força do tick, não geometria.

**Consequência para o código:** configurar uma "zona" via 0x5D seria escrever
gears de háptica (104/128 = intensidade High) e estragar a preferência do
utilizador. Não foi implementado — e o comentário em `motor_gears()` já estava
certo.

### 8.2 A geometria real (constantes do binário)

`handle_edge_slide` original ≈ `0x1800190d3`–`0x1800193d8`:

| Constante | Onde | Efeito |
|---|---|---|
| `24/25` = 96 % (`0x180018ae3`) | `xmm12` | bordo **direito**: `x > 0.96·x_max` |
| `1/25` = 4 % (`0x180018ada` → `x_max/25`) | `xmm11` | bordo **esquerdo**: `x < 0.04·x_max` |
| `-100.0` (`0x180018aec`) | `xmm10` | converte Δy em **percentagem da altura** |
| `cmp eax, 0xFA` (250) | `0x1800191a3` | **pré-rolo**: só captura depois de 250 unidades (8 mm) de Δy |
| `cmp r8d, 2` (volume) | `0x1800191c0` | cada passo = 2 % da altura (≈1,4 mm) |
| `cmp r8d, 4` (brilho) | `0x180019390` | cada passo = 4 % da altura (≈2,7 mm) |
| `0xAF`/`0xAE` | `0x1800191e2` | `SendInput(VK_VOLUME_UP/DOWN)` |
| `0x6F`/`0x70` | `0x180018182` | `SendInput(VK_BRIGHTNESS_DOWN/UP)` |

Notas de comportamento que só o binário revela:

1. **Âncora obrigatória na banda**: o primeiro ponto do traço *e* o ponto actual têm
   de estar dentro dos 4 %. Não há histerese — se o dedo derivar para dentro, o
   gesto morre e a âncora é reposta a `-1.0f` (`0xbf800000` em `0x180019242`).
2. **Pré-rolo de 250 unidades** antes de qualquer acção. É este — e não a largura da
   banda — o mecanismo que impede o gesto de disparar num clique/scroll normal. Com
   o pré-rolo satisfeito, o original emite **uma** acção nesse mesmo frame e
   re-ancora (o `pct > 2` já está garantido pelos 250 unidades).
3. O `g_timer.GetMs() > 500` que aparece nas strings **é só do log**: o frame háptico
   é enviado na mesma; o gate de 500 ms protege a linha de log.
4. **O original lê `TipSwitch` (0x42) *e* `Confidence` (0x47)** —
   `HidP_GetUsages` sobre as duas usage arrays em `0x180018dde`. O nosso
   `select_contact` só usa TipSwitch; `Confidence` continua por explorar como
   métrica de qualidade de contacto.
5. **Não há supressão de cursor nenhuma**: `PluginGesture.dll` não importa
   `ClipCursor`, `SetCursorPos` nem `ShowCursor`. No original o ponteiro **também
   se move**; o que o torna invisível é a banda de 4 % manter o cursor encostado ao
   bordo do ecrã. Quem quiser "rato parado" tem de o fazer explicitamente (§8.3).

### 8.3 O que mudou neste branch (`f4b/edge-zone`)

| Área | Antes | Agora | Porquê |
|---|---|---|---|
| Cursor | `WH_MOUSE_LL` desligado por constante (`EDGE_POINTER_SUPPRESSION = false`), o rato andava | `ClipCursor` para um pixel no frame de captura, libertado em todos os caminhos terminais + watchdog de 4 s | O hook LL corria na thread do gesto (que faz WMI/OSD) e o movimento engolido era re-aplicado como salto ao largar. `ClipCursor` prende de facto (verificado nesta máquina: 638,805 mantém-se após `SetCursorPos(+300,+200)`) e não salta ao libertar |
| Banda | `x_max/8` (12,5 %) / `x_max/3` (33 %) | **16 % / 28 %**, configurável (`ZoneStartPct`, `ZoneContinuePct`) | 4 % do original é ~4 mm — não se acerta com o dedo. 16 % ≈ 17 mm é uma área utilizável; a histerese de 28 % deixa derivar sem matar o gesto |
| Captura | `y_max/80` = 0,85 mm | `y_max·118/1000` = **250 unidades = 8 mm** (o `0xFA` do original) | É o que impede o gesto de "interferir no uso normal": um toque normal no bordo não percorre 8 mm |
| Acção no 1.º frame | nenhuma (só depois de mais `y_step`) | **uma**, exactamente como o original | Feedback imediato no momento em que o gesto compromete |
| Acumulador | decaimento de 1/8 em **todos** os frames, deadband `y_step/6` | decaimento só nos frames de ruído, deadband `y_step/10` | A 10 unidades/frame o equilíbrio era `8×10 = 80 < y_step = 88` — o gesto capturava e **nunca** dava passo, a mesma classe de bug de §4 |
| Tick háptico | frame construído e testado, **nunca enviado** | `0D 09 FD 01 00 03 00 00 02 32 CE` por passo, limitado a ~25/s | É o único efeito no dispositivo que o original produz por passo |
| Log de escrita | INFO em todas | INFO nas configurações (0x5D/0x59/0x5B), DEBUG no tick | Os bytes exactos continuam registados para reversão sem inundar o log a 25 Hz |

O `WH_MOUSE_LL` foi **removido** (não apenas desligado): era código morto que
prometia uma supressão que nunca aconteceu.

### 8.4 Plano de teste de campo

1. Ligar EdgeSlide; no log deve aparecer
   `[gesture] edge-slide zone: start<16% continue<28% of x_max`.
2. Deslizar devagar no bordo esquerdo: **nada** durante os primeiros ~8 mm
   (`armed`, sem `captured`), depois
   `edge-slide captured … activation=250` seguido de `pointer pinned at (x, y)`.
   O cursor tem de ficar imóvel durante todo o gesto.
3. Ao levantar o dedo: `pointer released at (x, y) after N ms (gesture ended)` e o
   cursor move-se outra vez. Se o `released` não aparecer em 4 s, o watchdog
   dispara (`reason=watchdog`).
4. Repetir no bordo direito (volume) e confirmar 1 tick por passo (o LED/motor
   vibra; com `RUST_LOG=hw::touchpad=debug` vê-se o frame).
5. **Não interferência**: clicar e arrastar uma scrollbar no bordo, fazer scroll de
   dois dedos e um clique simples no canto — nenhum deve produzir `steps=`.
6. Afinar a área sem rebuild:
   `reg add HKCU\Software\MiControl\Touchpad /v ZoneStartPct /t REG_DWORD /d 10`
   (e `ZoneContinuePct`), desligar/ligar o toggle, repetir o passo 2.
7. Se o cursor "saltar" ao libertar, o culpado é um `SetCursorPos` de outro
   programa, não o `ClipCursor` — recolher `RUST_LOG=hw::touchpad=info` e a posição
   antes/depois.
