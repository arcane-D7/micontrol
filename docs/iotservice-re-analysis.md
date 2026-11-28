# IoTService.exe — Reverse Engineering Analysis (Phase 1: Strings & IPC Mapping)

> **Data da análise:** 25/05/2026 (Phase 1 — Ghidra strings)  
> **Atualizado:** 30/06/2026 (Phase 2 — radare2 deep analysis + custom replacement)  
> **Binário analisado:** `IoTService.exe` (Xiaomi Mi Notebook, versão 25.0.0.9, compilado Mar 6 2026)  
> **Copyright:** © 2025 小米科技有限责任公司 (Xiaomi Technology Co., Ltd.)  
> **Ferramentas:** Ghidra 12.1 PUBLIC (headless) via Git Bash, radare2  
> **Objetivo:** Mapear todas as funcionalidades e o protocolo IPC do serviço oficial de hardware Xiaomi para avaliar a viabilidade de substituir IOCTLs diretos ao driver.

---

> ⚠️ **DOCUMENTO COMPLEMENTAR** — Este arquivo documenta a **Phase 1** (análise de strings via Ghidra).  
> A **Phase 2** (análise profunda com radare2 — IOCTLs, buffer layout, security check, ranges de endereço, binary customizado) está documentada em:  
> **[RE_ANALYSIS_REPORT.md](./RE_ANALYSIS_REPORT.md)**
>
> **Resumo das descobertas da Phase 2:**
>
> - IOCTL codes: `0x22E000` (READ), `0x22E004` (WRITE), buffer de 0x110 bytes
> - 3 ranges de endereço físico permitidos: `0xFE0B0F00/0x80`, `0xFE0B0AB8/0x08`, `0xFE0B0E00/0x100`
> - Security check: processo deve se chamar `IoTService.exe` e estar no diretório do DriverStore
> - ERAM (`0xFE0B0300`) e SMA2 (`0xFE0B0A00`) **NÃO são acessíveis** — não estão nos ranges permitidos
> - Custom `ecram_service.exe` construído e testado com sucesso (pipe IPC + IOCTL proxy)
> - Ver [RE_ANALYSIS_REPORT.md](./RE_ANALYSIS_REPORT.md) para detalhes completos

---

## 1. Estrutura do binário

O binário é um PE64 (x86-64 Windows Service) compilado com MSVC. As strings de debug revelam os arquivos-fonte originais:

| Arquivo fonte                               | Responsabilidade                             |
| ------------------------------------------- | -------------------------------------------- |
| `IoTDriver\IoTService\IoT.cpp`              | Classe principal do worker IoT               |
| `IoTDriver\IoTService\IoTDriver.cpp`        | Wrapper de comunicação com o driver          |
| `IoTDriver\IoTService\OnServiceEvent.cpp`   | Tratamento de eventos de energia/sessão      |
| `IoTDriver\IoTService\RamIO.cpp`            | Leitura/escrita de EC RAM via IOCTL          |
| `IoTDriver\IoTService\RunAsHelper.cpp`      | Elevação de privilégios, CreateProcessAsUser |
| `IoTDriver\IoTService\Util_Driver.cpp`      | Utilitários de descoberta de dispositivo     |
| `IoTDriver\IoTService\Util_Dump.cpp`        | Configuração de crash dumps                  |
| `IoTDriver\IoTService\Worker_IPC.cpp`       | Worker do cliente IPC                        |
| `IoTDriver\IoTService\Worker_IPCBroker.cpp` | Servidor do named pipe IPC                   |
| `IoTDriver\IoTService\Worker_WMI.cpp`       | Worker de status WMI                         |
| `IoTDriver\IoTService\worker.cpp`           | Orquestrador de workers                      |

---

## 2. Canal IPC (Named Pipe)

O serviço expõe um **named pipe** para comunicação entre processos:

```
\\.\pipe\LOCAL\IoTService_IPC_Broker
```

- Criado com `CreateNamedPipeW`
- Gerenciado pelo worker `Worker_IPCBroker.cpp`
- Mensagens trafegam em **JSON** (biblioteca `nlohmann/json` embutida)
- Cada mensagem possui campos `SrcId` (uint16) e `DstId` (uint16) e `Type` (int)
- Há validação de assinatura e tamanho: `Invalid IPC message signature` / `Invalid IPC message size`

### Formato de log de mensagem recebida

```
Received IPC message: SrcId=0x%04X, DstId=0x%04X, Type=%d
```

---

## 3. Comandos IPC disponíveis (mapeados por strings)

Todos os comandos abaixo foram confirmados por strings de log e strings de erro de validação JSON no binário:

### 3.1 Gerenciamento de dispositivo IoT (binding)

| Comando / Operação | Descrição                                             |
| ------------------ | ----------------------------------------------------- |
| `GetBindStatus`    | Retorna UID do dispositivo vinculado e status de bind |
| `GetDeviceID`      | Retorna o Device ID (`DID`) do hardware IoT           |
| `GetFwVersion`     | Versão do firmware do dispositivo IoT                 |
| `GetModel`         | Modelo do dispositivo (`Model`)                       |
| `ResetDevice`      | Reseta o dispositivo IoT                              |
| `SetDeviceStatus`  | Define o status do dispositivo (via JSON)             |
| `GetDeviceStatus`  | Lê o status atual do dispositivo                      |

**Retornos confirmados por string:**

```
GetBindStatus returned %s
GetDeviceID returned %s
GetFwVersion returned %s
GetModel returned %s
GetDeviceStatus returned %d
ResetDevice returned %s
```

### 3.2 Status do laptop (ciclo de vida do OS)

| Operação             | Constante        | Descrição                     |
| -------------------- | ---------------- | ----------------------------- |
| `SendLaptopStatus`   | `IOT_WIN_READY`  | Windows pronto (logon/boot)   |
| `SendLaptopStatus`   | `IOT_SUSPENDING` | Sistema entrando em sleep     |
| `SendLaptopStatus`   | `IOT_SHUTING`    | Sistema desligando (shutdown) |
| `ReportLaptopStatus` | —                | Reporta estado ao driver EC   |

**Logs associados:**

```
IoTDriver::ReportLaptopStatus(IOT_WIN_READY) succeeded.
IoTDriver::ReportLaptopStatus(IOT_SUSPENDING) succeeded.
IoTDriver::ReportLaptopStatus(IOT_SHUTING) succeeded.
Invalid LaptopStatus value: %u
Invalid JSON data for SendLaptopStatus
```

### 3.3 WiFi (provisionamento para dispositivo IoT)

| Comando          | Descrição                                               |
| ---------------- | ------------------------------------------------------- |
| `WriteWiFiItem`  | Salva credenciais WiFi no driver para o dispositivo IoT |
| `DeleteWiFiItem` | Remove uma entrada WiFi por índice/SSID                 |
| `GetWiFiByIndex` | Retorna item WiFi por índice                            |
| `ReadWiFiCount`  | Número de redes WiFi salvas                             |
| `ReadWiFiStatus` | Estado atual da conexão WiFi do dispositivo IoT         |
| `EmptyWiFiItems` | Remove todas as entradas WiFi                           |
| `ConnectWiFi`    | Força tentativa de conexão WiFi no dispositivo IoT      |

**Retornos confirmados:**

```
WriteWiFiItem returned %s / DeleteWiFiItem returned %s
GetWiFiByIndex returned %s / ReadWiFiCount returned %s
ReadWiFiStatus returned %s / EmptyWiFiItems returned %s
ConnectWiFi returned %s
```

**Campos JSON para WriteWiFiItem:**  
`SSID`, `Enable`, `Uid`, `FwVersion` (confirmados por strings de erro de registry)

### 3.4 Eventos do EC e do sistema

O serviço registra-se para receber eventos do EC via WMI e para notificações de energia via `RegisterPowerSettingNotification`:

**Eventos de energia monitorados (PowerSettingNotification GUIDs):**

| GUID                                | Descrição                                  |
| ----------------------------------- | ------------------------------------------ |
| `GUID_ACDC_POWER_SOURCE`            | Fonte AC/DC/Hot (PoAc, PoDc, PoHot)        |
| `GUID_BATTERY_PERCENTAGE_REMAINING` | Percentual de bateria restante             |
| `GUID_MONITOR_POWER_ON`             | Monitor ligado/desligado                   |
| `GUID_POWER_SAVING_STATUS`          | Modo de economia ativo/inativo             |
| `GUID_POWERSCHEME_PERSONALITY`      | Perfil de energia (High Perf, Saver, Auto) |
| `GUID_SYSTEM_AWAYMODE`              | Modo away ativo/inativo                    |
| `GUID_LIDSWITCH_STATE_CHANGE`       | Tampa aberta/fechada                       |
| `GUID_CONSOLE_DISPLAY_STATE`        | Estado do display (on/off/dimmed)          |
| `GUID_GLOBAL_USER_PRESENCE`         | Presença do usuário (ativo/ausente)        |
| `GUID_SESSION_DISPLAY_STATUS`       | Status do display por sessão               |
| `GUID_SESSION_USER_PRESENCE`        | Presença por sessão                        |

**Eventos de energia WM (PBT):**

| Evento                     | Descrição                                  |
| -------------------------- | ------------------------------------------ |
| `PBT_APMPOWERSTATUSCHANGE` | Mudança de status de energia (AC, bateria) |
| `PBT_APMRESUMESUSPEND`     | Resume por input do usuário                |
| `PBT_APMRESUMEAUTOMATIC`   | Resume automático                          |
| `PBT_APMSUSPEND`           | Sistema suspendendo                        |
| `PBT_APMBATTERYLOW`        | Bateria fraca                              |
| `PBT_APMOEMEVENT`          | Evento OEM                                 |

**Eventos de sessão WTS monitorados:**

- `WTS_SESSION_LOGON` / `WTS_SESSION_LOGOFF`
- `WTS_SESSION_LOCK` / `WTS_SESSION_UNLOCK`
- `WTS_SESSION_REMOTE_CONTROL`

**Evento WMI do EC:**

```sql
SELECT * FROM HID_EVENT20
```

### 3.5 EC RAM (RamIO)

O serviço acessa a RAM do Embedded Controller diretamente via IOCTL ao `IoTDriver.sys`:

```
RamDevice::Read()   — leitura de registradores EC
RamDevice::Write()  — escrita de registradores EC
RamIsReady          — verifica se EC não está ocupado (0x%02X)
```

Fluxo de comando EC:

```
Write command → ReadCmdAck (polling com retry, 4 bytes de ACK) → ReadCmdRet (4 bytes de retorno)
```

Strings de protocolo EC:

```
ReadCmdAck: expected ack %02X %02X %02X %02X, but got %02X %02X %02X %02X
ReadCmdAck: command ack is next cmd: %02X %02X %02X %02X, retry: %d. go on
ReadCmdAck: command %02X read ack timeout.
ReadCmdRet: command %02X read ret timeout.
```

### 3.6 Controle de energia do sistema

| Operação  | Mecanismo                                  |
| --------- | ------------------------------------------ |
| Sleep     | `SetSuspendState` ou `CreateProcessAsUser` |
| Hibernate | Win32 hibernate API                        |
| Shutdown  | `CreateProcessAsUser` como usuário logado  |
| Wakeup    | Sinalização ao driver                      |

O serviço tenta obter token de usuário logado (`WTSQueryUserToken`) para sleep/shutdown; caso não haja usuário, executa diretamente.

### 3.7 Controle de interface WMI (Worker_WMI)

```
Worker_WMI::setDeviceStatus()
MICommonInterface.InstanceName='ACPI\PNP0C14\MIFS_0'
IoT App Service
```

Utiliza a interface WMI `ACPI\PNP0C14\MIFS_0` para comunicação complementar com ACPI/EC.

---

## 4. Registro e configuração

Chaves de registro utilizadas:

| Caminho                                    | Campos                   |
| ------------------------------------------ | ------------------------ |
| `SOFTWARE\MI\IoTDriver`                    | Status do driver, modelo |
| `SOFTWARE\MI\IoTService`                   | Configuração do serviço  |
| `C:\ProgramData\MI\IoTService\`            | Logs, dumps              |
| `C:\ProgramData\MI\IoTService\service.log` | Log principal do serviço |

---

## 5. Flags e modos especiais

| Feature          | Descrição                                                                   |
| ---------------- | --------------------------------------------------------------------------- |
| Stress Test Mode | `IoT Stress Test Mode is ENABLED` — bloqueia comandos do sistema            |
| IPC Client ID    | `Failed to start IPC client with ID 0x%04X` — múltiplos clientes suportados |
| Dump config      | `DumpCount`, `DumpFolder`, `DumpType` em registry                           |

---

## 6. Avaliação de viabilidade: substituir IOCTLs por IPC

> **Atualizado 30/06/2026** — Após análise profunda com radare2 (Phase 2), as limitações abaixo foram confirmadas e uma solução customizada foi implementada.

### O que pode ser feito via IPC

| Funcionalidade                  | Via IPC?     | Observação                                                                        |
| ------------------------------- | ------------ | --------------------------------------------------------------------------------- |
| Ler status do dispositivo       | ✅ Sim       | `GetDeviceStatus`, `GetBindStatus`, `GetFwVersion`, `GetModel`                    |
| Escrever status                 | ✅ Sim       | `SetDeviceStatus` (JSON)                                                          |
| Reportar estado do OS           | ✅ Sim       | `SendLaptopStatus` (IOT_WIN_READY, etc.)                                          |
| WiFi provisioning               | ✅ Sim       | `WriteWiFiItem`, `DeleteWiFiItem`, etc.                                           |
| Eventos de energia              | ✅ Sim       | Subscrever eventos do serviço                                                     |
| Tampa/monitor/display           | ✅ Sim       | Via eventos passados pelo serviço                                                 |
| Sleep / Shutdown / Hibernate    | ✅ Sim       | Notificação via IPC, serviço executa                                              |
| Leitura/escrita EC RAM raw      | ✅ Sim       | **Confirmado na Phase 2** — Veja [RE_ANALYSIS_REPORT.md](./RE_ANALYSIS_REPORT.md) |
| Fan control / perfis de energia | ✅ Via WMI   | **Confirmado** — WMI `MICommonInterface` fornece performance mode read/write      |
| Backlight / teclado             | ⚠️ Verificar | Não confirmado nas strings; pode ser via EC RAM interno                           |

### Limitações conhecidas (confirmadas na Phase 2)

1. **ERAM não acessível** — O driver valida endereços físicos contra 3 ranges hardcoded. ERAM (`0xFE0B0300`) e SMA2 (`0xFE0B0A00`) **não estão** nos ranges permitidos. Retornam `STATUS_ACCESS_DENIED`.
2. **Secure Boot impede modificação do driver** — IoTDriver.sys é assinado e o Secure Boot está ativo. Não é possível adicionar ranges ao driver.
3. **Security check do driver** — O processo deve se chamar `IoTService.exe` E estar no diretório do DriverStore. MiControl.exe não pode acessar o driver diretamente.
4. **AC adapter wattage (ADPW)** — Está em ERAM+0x81 (`0xFE0B0381`), que **não é acessível** via driver. Alternativa: usar WMI para adapter power status.

### Solução implementada: Custom IoTService.exe (`ecram_service.exe`)

Uma solução customizada foi construída e testada com sucesso:

1. **Binary customizado** em Rust (`src-tauri/src/bin/ecram_service.rs`)
2. **Nomeado `IoTService.exe`** — Passa no security check do driver
3. **Colocado no DriverStore** — Passa no path check
4. **Named pipe IPC** em `\\.\pipe\ecram_service` com protocolo JSON
5. **Proxy de IOCTL** — Encaminha reads/writes de ECRAM para IoTDriver.sys
6. **Testado** — Lê IOT_STATUS, IOT_SENSORS, ECRAM sensor block com sucesso

**Integração no MiControl:** `src-tauri/src/hw/ecram.rs` tem um pipe client (`read_ecram_via_pipe()`) que se comunica com o serviço customizado.

**Detalhes completos:** [RE_ANALYSIS_REPORT.md](./RE_ANALYSIS_REPORT.md)

---

## 7. Como reproduzir a análise com Ghidra (headless via Git Bash)

### Pré-requisitos

- [Ghidra 12.1 PUBLIC](https://ghidra-sre.org/) extraído em `C:\Temp\ghidra12\ghidra_12.1_PUBLIC\`
- Java 21+ no PATH (ex: `C:\Program Files\Microsoft\jdk-21.0.11.10-hotspot\bin`)
- Git Bash instalado (`C:\Program Files\Git\bin\bash.exe`)
- Binário alvo: `IoTService.exe` copiado para `C:\Temp\iotservice_analysis\`
- Scripts de skill: `C:\Users\mafsc\Documents\Projects\miPC\.agents\skills\ghidra\scripts\ghidra_scripts\`

### Passo 1 — Preparar o diretório de saída

```bash
mkdir -p /c/Temp/iotservice_analysis
cp /c/caminho/para/IoTService.exe /c/Temp/iotservice_analysis/
```

### Passo 2 — Exportar strings com Ghidra headless (via Git Bash)

Abra o Git Bash e execute:

```bash
GHIDRA="/c/Temp/ghidra12/ghidra_12.1_PUBLIC"
BINARY="/c/Temp/iotservice_analysis/IoTService.exe"
SCRIPTS="/c/Projects/miPC/.agents/skills/ghidra/scripts/ghidra_scripts"
OUTDIR="/c/Temp/iotservice_analysis"
PROJDIR="$OUTDIR/proj"

mkdir -p "$PROJDIR"

"$GHIDRA/support/analyzeHeadless" \
  "$PROJDIR" GhidraProject \
  -import "$BINARY" \
  -postScript ExportStrings.java "$OUTDIR" \
  -scriptPath "$SCRIPTS" \
  -deleteProject \
  > "$OUTDIR/ghidra_output.log" 2>&1
```

> **Atenção:** O script `ExportStrings.java` deve estar na pasta `ghidra_scripts/` apontada por `-scriptPath`. Certifique-se de usar o nome exato do arquivo (case-sensitive no Java).
>
> **Nota (2026-11-11):** o directório `.agents/skills/ghidra/` já não existe no repositório — aponte `SCRIPTS` para a cópia local de `ExportStrings.java` que estiver a usar.

### Passo 3 — Filtrar strings relevantes (PowerShell)

```powershell
Get-Content "C:\Temp\iotservice_analysis\IoTService.exe_strings.json" |
  ConvertFrom-Json |
  Select-Object -ExpandProperty strings |
  Where-Object { $_.value -match "IoT|pipe|IPC|WiFi|Bind|Status|Cmd|Report|Laptop|Event|Ram|ACPI" } |
  Select-Object -ExpandProperty value |
  Sort-Object -Unique
```

### Passo 4 — Exportar funções (opcional, análise mais profunda)

```bash
"$GHIDRA/support/analyzeHeadless" \
  "$PROJDIR" GhidraProject \
  -import "$BINARY" \
  -postScript ExportFunctions.java "$OUTDIR" \
  -scriptPath "$SCRIPTS" \
  -deleteProject \
  >> "$OUTDIR/ghidra_output.log" 2>&1
```

### Dicas e troubleshooting

| Problema                                | Solução                                                                                                        |
| --------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `Script not found: ExportStrings.java`  | Verificar se o arquivo está na pasta apontada por `-scriptPath`                                                |
| `analyzeHeadless` não encontrado        | Usar path completo com `/support/analyzeHeadless` (sem `.bat` no Bash)                                         |
| Projeto já existe / lock file           | Usar `-deleteProject` ou apagar manualmente a pasta `proj/`                                                    |
| Saída JSON vazia                        | Checar `ghidra_output.log` e `ghidra_analysis.log` para erros de script                                        |
| Java não encontrado                     | Adicionar JDK ao PATH no Git Bash: `export PATH="/c/Program Files/Microsoft/jdk-21.0.11.10-hotspot/bin:$PATH"` |
| Script não roda no Windows nativo (cmd) | **Usar Git Bash** — o script usa paths Unix-style internamente                                                 |

### Caminhos relevantes desta sessão

| Recurso                   | Caminho                                                                                |
| ------------------------- | -------------------------------------------------------------------------------------- |
| Ghidra                    | `C:\Temp\ghidra12\ghidra_12.1_PUBLIC\`                                                 |
| Scripts da skill Ghidra   | `C:\Users\mafsc\Documents\Projects\miPC\.agents\skills\ghidra\scripts\ghidra_scripts\` |
| Binário analisado         | `C:\Temp\iotservice_analysis\IoTService.exe`                                           |
| Strings exportadas        | `C:\Temp\iotservice_analysis\IoTService.exe_strings.json`                              |
| Log de análise Ghidra     | `C:\Temp\iotservice_analysis\ghidra_analysis.log`                                      |
| Projeto temporário Ghidra | `C:\Temp\iotservice_analysis\proj\`                                                    |

---

## 8. Próximos passos sugeridos

> **Atualizado 30/06/2026** — Itens 1-4 abaixo foram parcialmente ou totalmente completos. Status atualizado.

1. ~~**Decompilação do handler IPC**~~ — ✅ **Feito** (radare2, Phase 2). O handler IPC foi analisado, mas o protocolo do pipe original (`IoTService_IPC_Broker`) não foi reversado porque construímos nosso próprio serviço customizado.
2. ~~**Fuzzing do pipe**~~ — ✅ **Feito**. O custom `ecram_service.exe` foi testado com pipe ping, read_region, read/write direto. Ver [RE_ANALYSIS_REPORT.md](./RE_ANALYSIS_REPORT.md).
3. ~~**Mapeamento de EC opcodes**~~ — ✅ **Feito**. IOCTLs `0x22E000` (READ) e `0x22E004` (WRITE) mapeados, buffer de 0x110 bytes, 3 ranges de endereço permitidos confirmados.
4. ~~**Integração no miControl**~~ — ✅ **Feito**. `src-tauri/src/hw/ecram.rs` tem pipe client (`read_ecram_via_pipe()`, `is_pipe_broker_available()`). O binário customizado está em `src-tauri/src/bin/ecram_service.rs`.

### Próximos passos reais (pendentes)

1. **Deploy do ecram_service.exe** — Copiar o binário para o DriverStore como `IoTService.exe` e reiniciar o serviço IoTSvc.
2. **Rebuild e reinstalação do MiControl** — O binário instalado (v0.1.2, 30-Jun 02:38) está desatualizado em relação ao código atual (pipe client em `ecram.rs`).
3. **Testar tabs não verificadas** — Cast, Touchpad, WiFi, Keyboard precisam de QA no browser.
4. **Investigar acesso a ERAM/SMA2** — Explorar WMI como alternativa para AC adapter wattage (já que ERAM+0x81 não é acessível via driver).
