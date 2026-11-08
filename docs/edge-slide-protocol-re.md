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
- Não há eixo de "zona"; a noção de bordo é **nossa** (`x < x_max/8` para iniciar,
  `x < x_max/3` para continuar).

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