# BATTERY-LIMIT-DEBUG — porque o limite de 80% nunca travou a carga

**Data:** 2026-11-10 · **Branch:** `f4f/battery-limit` (worktree `miPC/wt/wt/f4f-battery-limit`)
**Âmbito:** limite de carga / Battery Care. Não toca em modos de performance, tampa, ventoinha.
**Sintoma do dono:** proteção de carga activa há muito tempo com limite 80%, bateria a **94%**, UI a dizer _"Paused at 94% — the limit is 80%"_, carregador de 100 W ligado.

---

## 1. TL;DR

1. **A escrita do limite nunca chegou ao hardware.** O caminho `iot_set_charging_threshold` → `ecram_service` → `ECRAM_WRITE` em `0xFE0B03A4`/`0xFE0B03A7` devolve **`Access is denied. (0x80070005)`** — o `IoTDriver.sys` não permite a região `0xFE0B0300` a _nenhum_ IOCTL, nem ao serviço SYSTEM. O erro era engolido e o registry respondia por ele.
2. **A UI inventava a pausa.** `ChargingProtection.tsx` calculava `paused = care && level > limite` — 94% > 80% ⇒ "Paused at 94%". Ninguém perguntou ao hardware. O EC nunca recebeu os 80%.
3. **O caminho real existe e é o ACPI `WMAA`.** `MICommonInterface.MiInterface` (`FUN2=0x1000`, `FUN3=0x02`) escreve `LONL/HBDA` no EC. Descodificado do `ssdt24.dsl` e confirmado ao vivo: `wmi_ec_write(0x1000, 0x02, 1)` devolve **`SGER=0x8000`** (aceite pelo firmware).
4. **O payload do `0x1003` é um código de nível, não uma percentagem.** OEM enum `gp`: 1=80%, 4=90%, 5=70%, 6=60%, 7=50%, 8=40%, 0=sem limite. O nosso código enviava `80` (percentagem) — valor que o firmware não conhece.
5. **O 94% não é pausa nenhuma.** Com 100 W ligados, `Charging=false`, `ChargeRate=0`, `17.59 V` (≈4.40 V/célula em 4S): a carga terminou por tensão/química, não por limite. A app interpretou isso como "o limite está a segurar".

---

## 2. Cadeia actual (antes do fix), passo a passo

```
UI (ChargingProtection.tsx)
  └─ invoke('set_charging_threshold', {threshold: 80})
       └─ commands/hardware.rs::set_charging_threshold
            └─ elev_bridge::run_elevated("set_charging_threshold")
                 └─ elevated.rs::dispatch → hw::charging::set_charging_threshold(80)
                      ├─ ecram pipe: {"op":"iot_set_charging_threshold","threshold":80}
                      │    └─ ecram_service::ec_command::set_charging_threshold
                      │         ├─ write_ecram(0xFE0B03A4, [0x01])  → 0x80070005
                      │         └─ devolve {"ok":false,"error":"...Access is denied..."}
                      ├─ log::warn!("... falling back to registry")   ← silêncio
                      └─ persist_threshold_registry(80) → Ok(ChargingResult{success:true, method:"registry"})
  └─ toast "Limit of 80% applied — stops charging at 80%"             ← MENTIRA
```

Leitura (`get_charging_threshold`): tenta o pipe; o serviço responde
`{"ok":true,"cmd":"get_charging_threshold","battery_care_enabled":false,"threshold":80}`
— ou seja, **o próprio serviço diz `care=false`** (porque a leitura de `0xA4` falhou) e devolve
`threshold=80` do registry. A app mostra 80 e a UI pinta "proteção ligada".

---

## 3. Provas (timestamps reais, 2026-11-10)

### 3.1 Pipe `\\.\pipe\ecram_service` (serviço `IoTSvc` RUNNING, `LocalSystem`)

```
2026-11-10 19:27:10  {"op":"ping"}                              -> {"ok":true,"pong":true}   (1 ms)
2026-11-10 19:27:11  {"op":"iot_get_charging_threshold"}        -> {"ok":true,"cmd":"get_charging_threshold","battery_care_enabled":false,"threshold":80}  (6 ms)
2026-11-10 19:27:11  {"op":"read","addr":"0xFE0B03A4"}          -> {"ok":false,"error":"DeviceIoControl ECRAM_READ: Access is denied. (0x80070005)"}  (3 ms)
2026-11-10 19:27:11  {"op":"read","addr":"0xFE0B03A7"}          -> {"ok":false,"error":"DeviceIoControl ECRAM_READ: Access is denied. (0x80070005)"}  (3 ms)
2026-11-10 19:27:11  {"op":"iot_set_charging_threshold","threshold":80}  -> {"ok":false,"error":"DeviceIoControl ECRAM_WRITE: Access is denied. (0x80070005)"}  (3 ms)
2026-11-10 19:27:11  {"op":"iot_set_charging_threshold","threshold":100} -> {"ok":false,"error":"DeviceIoControl ECRAM_WRITE: Access is denied. (0x80070005)"}  (1 ms)
2026-11-10 19:27:12  {"op":"iot_set_charging_threshold","threshold":80}  -> {"ok":false,"error":"DeviceIoControl ECRAM_WRITE: Access is denied. (0x80070005)"}  (2 ms)
2026-11-10 19:27:12  {"op":"read_region","region":"IOT_STATUS"}  -> {"ok":true,"addr":"0xFE0B0F00","size":8,"data":"0008010200000000"}  (1 ms)
```

- A op **existe** no binário deployado (`IoTService.exe` = o nosso `ecram_service`, FileVersion **0.2.20**, 21 731 840 bytes — igual a `C:\Program Files\MiControl\ecram_service.exe`). Não é "unknown op": é recusa do driver.
- Regiões permitidas pelo driver (confirmado por leitura): `0xFE0B0F00+0x80`, `0xFE0B0AB8+8`, `0xFE0B0E00+0x100` — todas lêem. `0xFE0B0300+0x100` (ERAM: `0xA4`, `0xA7`) não.
- O bloco permitido `0xFE0B0E00+0x100` está **todo a zeros** (buffer de sensores), logo não há cópia daí.

### 3.2 Estado real do sistema no momento do diagnóstico

```
{"level":94,"is_charging":false,"is_plugged":true,"charge_rate_mw":0,"voltage_mv":17590,
 "ac_input_power_mw":100000,"full_capacity_mwh":63666,"designed_capacity_mwh":68224}
```

`root\WMI BatteryStatus`: `Charging=false, Discharging=false, ChargeRate=0, RemainingCapacity=60081 mWh, Voltage=17.587 V, PowerOnline=true`.

`60129/63666 = 94.4%` → Windows mostra 94%. `17590 mV / 4 células = 4397 mV/célula`.

### 3.3 A escrita que funciona (ACPI WMAA), ao vivo

```
2026-11-10 19:2x  wmi_ec_write(fun2=0x1000, fun3=0x02, fun4=0)  -> {"ok":true,"value":{"sger":32768,...}}   # 32768 = 0x8000
2026-11-10 19:29  wmi_ec_write(fun2=0x1000, fun3=0x02, fun4=1)  -> {"ok":true,"value":{"sger":32768,...}}   # 80%
```

`SGER=0x8000` é o valor de sucesso definido pelo próprio método AML (`SGER = 0x8000` antes do `ECWT`).
Bateria depois da escrita: `Charging=false, ChargeRate=0` (esperado — 94% > 80%).
**Limitação:** o read-back (`FRD1`) exige admin; um processo não elevado recebe
`0x80041003 Access denied` ao enumerar `MICommonInterface`. O read-back só é verificável
na build deployada (comando `get_charge_limit_state`, que corre no helper elevado). Ver §8.

---

## 4. Protocolo `0x1003` decodificado

### 4.1 Mensagem MCPI (o que o nosso código enviava)

Header de 16 bytes (`#[repr(C)]`, documentado a partir de `Worker_IPC.cpp`):

| offset | campo       | valor                   |
| ------ | ----------- | ----------------------- |
| 0      | magic u32   | `0x4950434D` ("MCPI")   |
| 4      | src_id u16  | 1 (MiControl)           |
| 6      | dst_id u16  | 2 (IoTDriver)           |
| 8      | type_lo u16 | **0x1003** (set limit)  |
| 10     | routing i16 | 0                       |
| 12     | field u16   | 0                       |
| 14     | payload_len | 17 (header 16 + 1 byte) |

Payload de **1 byte** = **código de nível** (não percentagem).

**Erro #2:** `charging.rs::send_via_pipe` escrevia `&[threshold]`, isto é `80 (0x50)`.
O firmware só conhece `0,1,4,5,6,7,8`; `0x50` não corresponde a caso nenhum do `Switch (FUN4)`.
Além disso o pipe MCPI já não existe (o nosso `ecram_service` substituiu o `IoTService.exe` original),
portanto este caminho estava duplamente morto.

### 4.2 O que o `0x1003` faz no serviço original

`IoTService.exe` (original, 747 112 bytes, PDB `Worker_WMI.cpp`) trata `0x1003` como
_SetChargingLimit_ e encaminha para WMI. O consumidor final é o método ACPI `WMAA`
(`\_SB.PC00.WMID`, `_HID PNP0C14`, `_UID "MIFS"`), descodificado de `ssdt24.dsl`:

```
Method (WMAA, 3)                  # Arg1 = MethodId, Arg2 = buffer InData (32 B)
  FUN1 = word@0   0xFA00 (read) | 0xFB00 (write)
  FUN2 = word@2   0x1000 = charging-protection group
  FUN3 = word@4   0x02
  FUN4 = dword@6  nível

Switch (Arg1) { Case (1) { Switch (FUN1) {
  Case (0xFA00) { Switch (FUN2) { Case (0x1000) { Switch (FUN3) {
      Case (0x02):                              # LER
        se (ECRD(LONL) & 1) == 1:
            HBDA==0x50 -> FRD1=1 ; 0x5A -> 4 ; 0x46 -> 5 ; 0x3C -> 6 ; 0x32 -> 7 ; 0x28 -> 8
        senão FRD1 = 0
        SGER = 0x8000                       # atenção: SGER é posto ANTES, não é prova do EC
  }}}
  Case (0xFB00) { Switch (FUN2) { Case (0x1000) { Switch (FUN3) {
      Case (0x02): Switch (FUN4) {          # ESCREVER
        0: ECWT(LONL & ~1); ECWT(HBDA, 0x64)   # 100% / proteção OFF
        1: ECWT(LONL | 1);  ECWT(HBDA, 0x50)   # 80%
        4: ECWT(LONL | 1);  ECWT(HBDA, 0x5A)   # 90%
        5: ECWT(LONL | 1);  ECWT(HBDA, 0x46)   # 70%
        6: ECWT(LONL | 1);  ECWT(HBDA, 0x3C)   # 60%
        7: ECWT(LONL | 1);  ECWT(HBDA, 0x32)   # 50%
        8: ECWT(LONL | 1);  ECWT(HBDA, 0x28)   # 40%
      }  SGER = 0x8000
  }}} }}}
```

Cross-check com o software OEM (`XiaomiPCManager 5.8.0.57`, `dist/static/js/main.js`):

```js
enum gp { One=1, Four=4, Five=5, Six=6, Seven=7, Eight=8 }
Map wp = { 8:"40%", 7:"50%", 6:"60%", 5:"70%", 1:"80% (Recommended)" }
L("set_charging_threshold", { mode: e })   // e = código, não percentagem
```

Os códigos do UI OEM são **exactamente** os casos do `Switch (FUN4)` do ACPI. Isto fecha a cadeia
UI OEM ↔ SvrCModule ↔ WMAA ↔ EC (`LONL` bit0 + `HBDA`).

Mapa dos registos ERAM (do DSDT, `OperationRegion (ERAM, SystemMemory, 0xFE0B0300, 0x100)`):

| offset | nome   | uso verificado                                                                                       |
| ------ | ------ | ---------------------------------------------------------------------------------------------------- |
| 0xA4   | `LONL` | bit0 = proteção ligada (`ECWT(LONL\|1)`)                                                             |
| 0xA7   | `HBDA` | limite em percentagem (`0x50`=80, `0x5A`=90, `0x46`=70, `0x3C`=60, `0x32`=50, `0x28`=40, `0x64`=100) |

### 4.3 Bónus: o `set_brightness_data` era um tiro no pé

`hw::wmi_ec::set_brightness_data()` escrevia `wmi_write(0x1000, 0x02, level)` como se fosse
"hotkey brightness data (HBDA)". É o **mesmo** par do limite de carga. Um brilho 5/6/7/8 teria
escrito 70/60/50/40% de limite, e um brilho 1 ligava a proteção a 80%. Não tinha chamadores na UI
(só estava exportado em `useHardware`), mas ficava à espera de ser ligado. **Removido** (comando,
hook, mock e entrada do dispatcher), com o motivo escrito no código.

---

## 5. Porque está a 94%?

O que está provado:

- O limite de 80% **nunca** foi escrito no EC (todas as escritas ERAM devolvem `0x80070005`).
- A carga **está** parada: `Charging=false`, `ChargeRate=0`, `PowerOnline=true`, 100 W na entrada.
- A tensão é `17.59 V` num pack 4S ⇒ **4.397 V/célula**, ou seja topo de carga real da célula.
- `RemainingCapacity/FullChargeCapacity = 94.4%`; `FullChargeCapacity` (63 666 mWh) é 93% da
  capacidade de projecto (68 224 mWh) — o gauge conta sobre uma FCC já degradada.

Interpretação (honesta): a carga terminou por **tensão/química** ao nível do charger IC, não por um
limite de 80% que nunca existiu. O 94% é "cheio" para este pack; a UI é que o rotulou de "pausa ao
limite de 80%". Não há evidência de um "battery care default" de firmware a 94/95% — para isso
seria preciso ler `LONL/HBDA` (bloqueado) ou observar um ciclo completo com o limite aplicado.
**R3 (pós-deploy):** com `get_charge_limit_state` a responder, observar a descida 100→80 para
confirmar que a carga pára exactamente no limite configurado.

---

## 6. Causa raiz

1. **Caminho de escrita morto por design do driver.** `IoTDriver.sys` só expõe
   `0xFE0B0F00+0x80`, `0xFE0B0AB8+8`, `0xFE0B0E00+0x100`. Escrever `0xA4/0xA7` via IOCTL é
   impossível — e o código tentava-o na mesma.
2. **Fallback silencioso.** `charging.rs` convertia a falha em `ChargingResult{success:true,
method:"registry"}`; a UI mostrava sucesso. O registry passou a ser a "verdade" sem nunca ter
   sido hardware.
3. **UI que inferia estado do hardware a partir de uma comparação de números.**
   `paused = care && level > limite` produz "pausa" mesmo quando o limite não existe no EC.
4. **Protocolo mal interpretado.** `0x1003` transporta um código (0/1/4/5/6/7/8), não a percentagem;
   e o serviço de substituição não fala MCPI.

---

## 7. Fix

### Backend (`src-tauri/src/hw/charging.rs`, reescrito)

- Caminho primário: `wmi_write(0x1000, 0x02, code)` + **read-back** `wmi_read(0x1000, 0x02)` e
  comparação. Sucesso só com `SGER=0x8000` **e** nível lido igual ao pedido.
- `percent_to_code` / `code_to_percent` / `parse_level_code` (puros, testados) — 40→8, 50→7, 60→6,
  70→5, 80→1, 90→4, 100→0.
- `set_charging_threshold` devolve `ChargingResult{success, method:"wmi_wmaa", threshold,
hardware_threshold, detail}`; **falha = erro**, nunca "registry" com success.
- `get_charge_limit_state()` → `{requested_threshold, hardware_threshold, hardware_code, applied,
source, detail}`; nunca inventa: sem leitura, `hardware_threshold:null, applied:false`.
- `get_battery_care`/`set_battery_care(enabled, limit)` no mesmo campo do firmware (care e limite
  são o mesmo registo; `0` = desligado = 100%).
- Registry mantém-se apenas como **valor pedido** (`HKLM\SOFTWARE\MI\IoTDriver\ChargingThreshold`),
  nunca como estado aplicado.

### Bridge / comandos

- `elevated.rs`: novos comandos `get_battery_care` e `get_charge_limit_state`; `set_battery_care`
  passa a devolver o resultado verificado. Allowlist de leitura WMAA ganhou `(0x1000,0x02)`.
- `elev_bridge.rs`: os dois novos comandos usam o timeout médio (45 s).
- `commands/hardware.rs`: `get_charging_threshold` continua barato (registry = valor pedido);
  `get_charge_limit_state` é o novo comando de verdade. `get_battery_care`/`set_battery_care`
  passam pelo helper elevado (o query de instância é negado a não-admin: `0x80041003`).

### Serviço (`bin/ecram_service.rs`)

- `iot_set_charging_threshold` / `iot_get_charging_threshold` deixam de tentar ERAM e de responder
  com o registry: devolvem erro explícito a dizer que `0xFE0B03A4/0xA7` está bloqueado e que o
  caminho é o WMAA `0x1000/0x02`. Removido o `read_charge_threshold_registry` que alimentava a
  mentira.

### UI

- `ChargingProtection` recebe `limitState` e **só diz "pausa" com `applied === true`**.
- Sem confirmação → nota `charging.state.notApplied` ("firmware diz {hardware}%, pediste {limit}%")
  ou `charging.state.unconfirmed` (não foi possível reler).
- Linha derivada: acima do limite sem hardware → `battery.limitNotApplied`.
- `useBatteryCare` relê o estado no arranque e depois de cada escrita; `battery.tsx` recarrega
  após aplicar.
- Textos do modal/técnico corrigidos nos 4 locales (diziam pipe `0x1003` + fallback registry).

### Testes

- Rust: `percent_to_code` vs enum OEM/SSDT, roundtrip, rejeição de valores impossíveis,
  `parse_level_code` (só 0/1/4/5/6/7/8), "código ≠ percentagem" (o erro histórico), serialização.
- TS: pausa só com `applied:true`; **regressão do bug do dono** (94% + limite 80% + `applied:false`
  ⇒ nada de "Paused at 94%", mostra "não aplicado"); caso "não confirmado".

---

## 8. Limitações e verificação pendente

1. **Read-back ao vivo não foi possível antes do deploy.** `Get-CimInstance`/`ExecQuery` em
   `MICommonInterface` devolve `0x80041003` a processos não elevados; o helper elevado da build em
   produção só expõe pares WMAA na allowlist antiga. A escrita foi provada (`SGER=0x8000`), a
   leitura fica provada quando a build com `get_charge_limit_state` correr.
2. **`SGER=0x8000` não é, por si, prova do EC.** O AML põe `SGER` antes do `ECWT` e não reporta
   falha do `ECWT`. Por isso o fix **exige** read-back antes de dizer "aplicado"; se o firmware
   aceitar mas não mudar, a UI diz "não aplicado" em vez de fingir.
3. **Histerese do EC desconhecida.** Com 94% e limite 80%, a carga já estava parada por tensão;
   a retoma (ex.: abaixo de ~79%) só se observa num ciclo real.
4. **Serviço deployado é a v0.2.20** (igual ao binário em `C:\Program Files\MiControl`); o novo
   comportamento das ops `iot_*charging*` só entra no próximo deploy do serviço.
5. **Estado final deixado no hardware:** `FUN4=1` → proteção ligada a **80%** (o valor que o dono
   quer). O teste intermédio `FUN4=0` (100%) foi revertido no mesmo minuto.
