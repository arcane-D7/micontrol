# Relatório de Oportunidades — Hardware IoT Xiaomi (MiControl)

**Data**: 2026-08-28 (sessão pós-commit `9f61cc1`)
**Escopo**: Hardware IoT presente no **Xiaomi Book Pro 14 2024 (TM2424)** que hoje está
**subutilizado** porque o software oficial Xiaomi (Xiaomi PC Manager) não é usado.
**Objetivo**: inventário do que é acessível hoje via MiControl, ideias de funcionalidades,
melhorias de estabilidade e projetos open source para integrar.

---

## 1. Inventário do hardware IoT acessível (e seu estado atual)

### 1.1 Chip IoT (classe nRF52) — via `IoTService_IPC_Broker` + `IoTDriver.sys`

O laptop carrega um chip IoT integrado (SoC BLE classe nRF52) com Bluetooth Low Energy.
Ele é gerenciado pelo serviço `IoTSvc` (`IoTService.exe` + driver `IoTDriver.sys`). O MiControl
já fala com ele por dois caminhos:

| Camada                                                                                                  | Protocolo                                                         | Estado                                                 |
| ------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- | ------------------------------------------------------ |
| **IPC nativo** `\\.\pipe\LOCAL\IoTService_IPC_Broker`                                                   | MCPI, magic `0x4950434D`, header 16B, msg_types `0x1001`–`0x5002` | Implementado em `hw/iotservice.rs`                     |
| **Proxy EC RAM** `\\.\pipe\ecram_service` (binário próprio com nome de `IoTService.exe` no DriverStore) | IOCTL `0x22E000` do driver, handshake zerado `0x110B`             | Implementado em `bin/ecram_service.rs` + `hw/ecram.rs` |

**O que esse chip faz hoje (via XPM original) que não fazemos**:

- **Provisão de WiFi** (msg_types `0x4001`–`0x4007`: `WRITE_WIFI_ITEM`, `READ_WIFI_COUNT`,
  `READ_WIFI_STATUS`, `CONNECT_WIFI`) — o MiControl já implementa o cliente IPC (inclusive
  criptografia AES-256-GCM de senhas com chave HKDF), mas o chip ainda não está sendo usado
  como **WiFi provisioner externo** (ex.: configurar rede WiFi do laptop a partir do celular).
- **Status do laptop** (`SetDeviceStatusRequest`, estados WinReady=4, Suspending=6, Shutting=8):
  o XPM conecta o celular ao laptop por BLE e mostra estado/energia. Nós temos o protocolo,
  mas nenhuma UI/noção de "estado remoto".
- **Eventos de energia/EC** (`PowerEventType`, `IotEvent`): o chip emite notificações de eventos
  (charge, AC, teclas) — usamos parte via EC commands, mas não consumimos o canal de eventos
  do chip propriamente dito.

### 1.2 GATT/BLE do chip (fingerprint do firmware)

Decompilação do firmware (Ghidra) revelou o serviço BLE:

| UUID             | Função                 |
| ---------------- | ---------------------- |
| Serviço `0xFFFF` | GATT principal Xiaomi  |
| Char `0x2711`    | Config WiFi (provisão) |
| Char `0x2712`    | Config do device       |
| Char `0x3E9`     | Eventos EC             |
| Char `0x3EA`     | Stream de sensores     |

Formato de advertisement Xiaomi: **Company ID `0x038F`**, **Service UUID `0xFE95`**
(padrão do ecossistema Mi). Ou seja: **o chip é um beacon BLE** — outros dispositivos Xiaomi/o
app Mi Home/Mi Band conseguem "ver" o laptop por BLE.

### 1.3 EC RAM / sensores (via WMAA WMI `MICommonInterface` e regiões EC)

O MiControl já lê/controla via WMI (funciona de forma estável):

- **Modos de performance**: `0x0800/0x00` (Perf, Balanced, Quiet, SuperQuiet, UltraPerf, Extreme).
- **AILM/LBLM** (abertura de tampa / luz de fundo): `0x0A00/0x08-09`.
- **LOTS/RMTS/OD08/OD09/PMBD**: `0x0C00/0x02-06`.
- **Bateria**: `SOH1` (saúde %), `HBDA` (desgaste), `ADPW` (watts de carregamento), `SCOB` (carga sob restrição), `OFBI` (otimização de carga).
- **Fan QFAN** (modos via FUN3 `2-0x0A`) e smart modes SMMT/SMMD.
- **EC fields** (mapa do DSDT): `+0x81 ADPW`, `+0x8C BTCT`, `+0x8E BTPR`, `+0x90 BTVT`,
  `+0xA4 battery care`, `+0xA7 HBDA`, `+0xAB SOH1`, `+0xAD CHA1`, `+0xAE DBLL`,
  `+0xB2 KBLL` (retroiluminação do teclado, 7 bits) e `+0xB4 KBMD`.

**Gap conhecido**: ERAM (`0xFE0B0300`) e SMA2 (`0xFE0B0A00`) **não são permitidos** pelo driver
(ranges permitidos: `0xFE0B0F00/0x80`, `0xFE0B0AB8/0x08`, `0xFE0B0E00/0x100`). Isso bloqueia
leitura direta de alguns campos de ERAM como `KBLL` (retroiluminação) — solução alternativa é
via WMI sempre que existir método (existe `LBLM` para backlight de tela; retroiluminação do
teclado depende de achar método WMI equivalente, ou reavaliar o range do driver).

### 1.4 WiFi / WMI HQ (métodos de BIOS)

`HQWmiCommonInterface` expõe 11 métodos BIOS: `SetPerformanceMode`, `ChangeBootOption`,
`LoadDefault`, `S5RTCWakeEnable`, `EnablePXEBoot`, `LoadDefaultKey`, `ClearKey`,
`WifiCountryCode`, `ShippingCountryCode`, OOB tests. **Já mapeados, pouco expostos na UI.**

### 1.5 Celular **Xiaomi 14T** (usuário)

Usa Android + apps Xiaomi. É o par natural do chip BLE e do Phone Link (Windows).

---

## 2. O que está subutilizado hoje

Do mapa acima, comparando com as 22 abas já existentes no MiControl:

| Recurso                                                           | Camada disponível                    | Usado na UI?                                                  |
| ----------------------------------------------------------------- | ------------------------------------ | ------------------------------------------------------------- |
| BLE beacon do chip (advertisement Mi `0xFE95`)                    | Chip IoT                             | ❌ (quase nada)                                               |
| Provisão WiFi via chip (write/read/connect wifi)                  | IPC `0x4001-0x4007`                  | ⚠️ parcial (gera item, não usa chip p/ conectividade externa) |
| Eventos EC/energia do chip (canal `IotEvent`)                     | IPC (`0x1001+`)                      | ⚠️ parcial (não consumimos o stream)                          |
| Stream de sensores via GATT `0x3EA`                               | BLE (não usamos BLE do lado Windows) | ❌                                                            |
| Telemetria de sensores (tensão, corrente, temperatura EC)         | WMAA/EC                              | ⚠️ parcial (exibição pontual)                                 |
| Retroiluminação do teclado (KBLL `+0xB2`)                         | ERAM bloqueado; WMI p/ procurar      | ❌                                                            |
| Sincronia de estado laptop↔celular (WinReady/Suspending/Shutting) | IPC                                  | ❌                                                            |
| Detecção de presença/ida-e-vinda (chip BLE como presence/geo)     | BLE                                  | ❌                                                            |
| Métodos BIOS HQ (WLAN country, boot, wake)                        | WMI HQ                               | ⚠️ parcial                                                    |

---

## 3. Ideias de funcionalidades interessantes (por prioridade/impacto)

### A. Instantâneas (0–1 sprint; usam camadas já prontas)

1. **Notificações do chip para OSD**: consumir o canal de eventos EC/energia e mostrar
   notificações nativas (ex.: "carregador conectado", "carga otimizada ativa") — reutiliza
   `IotEvent` + as notificações de sistema já usadas pelo MiControl.
2. **Tomada de decisão por eventos**: plug/unplug de AC → aplicar plano de energia ou modo de
   performance automaticamente (regra configurável). Já temos `ADPW` para detectar watts;
   combinado com `.CanStopCharging`/`ADP` melhora o ciclo de vida da bateria.
3. **Auto perf-mode por cenário**: sensor de tampa (AILM) + AC → alternar Quiet/Perf/UltraPerf
   (ex.: com AC e tampa fechada, modo silencioso; com AC e tampa aberta, Perf).
4. **Dashboard de telemetria de bateria em tempo real**: usar `EC sensors` + WMAA para plotar
   mV, mA, temperatura, watts e SOH1 com histórico local (reusa `EcSensorData`).

### B. Intermediárias (2–4 sprints; exigem BLE do lado Windows)

5. **Beacon BLE próprio**: anunciar via BLE (ou provisionar o chip para anunciar o estado do
   laptop, ex.: "livre", "ocupado", "carregando"). Companion Android veria o laptop como
   dispositivo Xiaomi (company ID `0x038F`, UUID `0xFE95`).
6. **Detecção de presença / lock-unlock por proximidade**: RSSI do celular (usando btleplug)
   para destravar/travar o laptop ao se afastar/aproximar (Windows Hello companion).
7. **WiFi provisioner externo**: usar o chip para expor SSID configurados e permitir que o
   celular conecte o laptop a uma rede (somente se o firmware suportar; hoje o IPC permite
   escrever item de WiFi com criptografia).
8. **Sensores como "vitals"**: stream `0x3EA` para mostrar CPU/EC health no telefone via app
   companion — reutiliza o backend de telemetria já existente.

### C. Avançadas (roadmap cross-device)

9. **Orquestração Phone Link** (ms-phone: URIs) já prototipada na aba `crossdevice`.
10. **Transferência de arquivos LocalSend** (protocolo aberto v2.2, Rust) — na rede local, sem
    apps Xiaomi.
11. **Espelhamento/uso do celular como webcam** (scrcpy com câmera) para chamadas.
12. **Legenda/transcrição local** (sherpa-onnx / whisper.cpp) para gravações — totalmente local.
13. **NFC tap-to-pair** (MiLinkNFC / NDEF + BLE handshake) para parear com um toque — o 14T tem NFC.

---

## 4. Melhorias de estabilidade (aproveitando o mesmo hardware/canais)

### 4.1 Já implementado na v0.1.25

- **Pipeline de crash reporting agnóstico** (`util/crash_report.rs`, desativado por padrão):
  `CrashEvent` → backend `Noop` (default) | `LocalFileBackend` | `HttpBackend` — pronto para
  conectar a qualquer serviço (Sentry, endpoint próprio) via registro
  `SOFTWARE\MiControl\CrashReporting` (`Enabled`, `Endpoint`, `InstallId`). Chamado a partir do
  panic hook escovado (sem PII no path).
- **Loading real do remap Copilot**: `set_hotkey_config` agora aguarda a aplicação de hardware e
  devolve `HotkeyApplyResult` (`applied`, `reboot_required`, `note`), com estado consultável
  (`get_remap_apply_state`).

### 4.2 Recomendações (novas)

- **Timeout e retry da bateria via WMI** (crash `ControlLib.dll`/`combase.dll` estão ligados a
  timeouts de `get_battery_info` de 15s): reduzir para ~5s e usar retry exponencial com cache.
- **Watchdog do canal IoT**: monitorar IPC do chip; se cair, reiniciar `IoTSvc`/reconectar pipe
  (o MiControl já tem self-heal para o app; estender para o serviço).
- **Persistência de eventos não entregues**: fila local de eventos de energia/EC com replay ao
  religar (evita perder "plug/unplug" enquanto o app está fechado).
- **Telemetria opcional (opt-in)**: enviar `SOH1/HBDA/temperatura` anonimizados junto com o
  crash report — mesma pipeline `CrashReporting`, dado `extra`.
- **Backend HTTP de crash reports**: conectar futuramente um endpoint próprio ou Sentry, ligando
  `Enabled=1`; manter `LocalFileBackend` como fallback local-first (já previsto na arquitetura).

---

## 5. Projetos open source para integrar

| Projeto                                  | Licença                           | O que resolve                                                                                      | Fit no MiControl                                                          |
| ---------------------------------------- | --------------------------------- | -------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| **btleplug** (deviceplug)                | BSD-3                             | Cliente BLE central no Windows (WinRT) — scan, GATT, notify, RSSI                                  | Base para beacon/app companion, presence, ler chars `0x3E9/0x3EA` do chip |
| **LocalSend** (localsend)                | AGPL/FOSS (protocolo documentado) | Transferência de arquivos P2P local (porta 53317, multicast `224.0.0.167:53317`, TLS, fingerprint) | Troca de arquivos PC↔celular sem apps Xiaomi; crate Rust disponível       |
| **KDE Connect** (kdeconnect)             | GPLv2                             | Protocolo aberto multi-dispositivo: clipboard share, find phone, ssh, presenter, ping              | Fase 4: integração profunda via REST/TLS do protocolo (JNI/companion)     |
| **scrcpy** (Genymobile)                  | Apache-2.0                        | Espelhar tela do Android; modo câmera UVC                                                          | Usar o 14T como webcam/câmera remota                                      |
| **sherpa-onnx** (k2-fsa)                 | Apache-2.0                        | ASR/STT/TTS local em Rust/Tauri (tem exemplos Tauri)                                               | Transcrição/legendas 100% local (Gap 11)                                  |
| **whisper.cpp** (ggml-org)               | MIT                               | ASR local leve em C/C++ (Rust bind crates)                                                         | Alternativa mais leve ao sherpa-onnx                                      |
| **ArgyllCMS / DisplayCAL**               | AGPL/GPL                          | Calibração de cores ICC via instrumento/software                                                   | Integração ao perfil de cor (Gap 17)                                      |
| **MpCmdRun.exe** (Windows Defender)      | Proprietária (integrável)         | Scan de segurança (quick/full/custom, update assinaturas)                                          | Aba Security Scan (já usada)                                              |
| **Syncthing**                            | MPL-2.0                           | Sync de pastas P2P entre dispositivos (protocolo BEP)                                              | Backup/sync seletivo PC↔celular (alternativa ao LocalSend p/ pastas)      |
| **MiLinkNFC** (XFY9326)                  | MIT                               | Reimplementação do ecossistema Xiaomi NFC/Lyra em Python                                           | Referência para NFC tap-to-pair do 14T                                    |
| **windows crate** (microsoft/windows-rs) | MIT/Apache                        | APIs WinRT (BLE Advertisement, GATT, Radio, YourPhone/Phone Link não incl.)                        | Implementação BLE direta sem btleplug se necessário                       |
| **btleplug** forks de advertising        | —                                 | Peripheral mode (anunciar) no Windows ainda limitado; usar chip IoT p/ anunciar                    | Confirmar se `BluetoothLEAdvertisementPublisher` resolve                  |

### Notas de decisão

- **Ler chars do chip via BLE** pode conflitar com o `IoTSvc` (o serviço já consome o GATT do
  chip). Ideal: BLE para **receber** (beacon/existência) e IPC para **controlar**.
- **LocalSend vs KDE Connect**: LocalSend é mais simples e maduro para arquivos; KDE Connect
  cobre mais plugins (clipboard, presenter, find-my-phone) com esforço maior.
- **Multicast da LocalSend**: precisa abrir `udp 53317` no firewall (documentado oficialmente).
- **Licenças**: LocalSend (AGPL-3.0 caso use o código do app; protocolo é aberto), KDE Connect
  (GPL-2.0), scrcpy (Apache-2.0) — ok para integrar por protocolo; **não copiar o app**; crates:
  LocalSend tem crate `localsend`; btleplug é BSD-3.

---

## 6. Roadmap sugerido

| Fase                       | Entrega                                                             | Esforço     |
| -------------------------- | ------------------------------------------------------------------- | ----------- |
| **P1 — Quick wins**        | Eventos→OSD, auto perf por cenário, watchdog IoT, timeout bateria   | 1–2 sprints |
| **P2 — Presença & beacon** | btleplug: scan RSSI do 14T; lock/unlock por proximidade; telemetria | 2–4 sprints |
| **P3 — Cross-device**      | LocalSend (arquivos), scrcpy (câmera), sherpa-onnx (transcrição)    | 1–2 meses   |
| **P4 — Profundo**          | KDE Connect protocol, NFC tap-to-pair (MiLinkNFC), sync Syncthing   | 3+ meses    |

---

## 7. Riscos e limitações

- **driver não oficial**: `ecram_service` contorna o check de nome de processo; range de ERAM
  bloqueia campos (KBLL etc.) — revisitar a tabela de ranges quando o driver atualizar.
- **BLE central do Windows** não suporta _advertising_ fácil (peripheral mode); para o laptop
  "anunciar" como Xiaomi, o caminho natural é o **próprio chip IoT** (ele anuncia por hardware).
- **Concorrência no GATT do chip**: se o Windows (bluetooth) conectar, pode disputar com o
  `IoTSvc` — manter IPC como fonte de verdade.
- **Privacidade**: tel de presença/telemetria deve ser opt-in; crash reports já escovados.

---

_Consulte também: `HARDWARE_INVESTIGATION.md`, `HARDWARE_GAP_ANALYSIS.md`,
`CROSS_DEVICE_ALTERNATIVES_REPORT.md`, `RE_ANALYSIS_REPORT.md`, `crash-reporting.md`._
