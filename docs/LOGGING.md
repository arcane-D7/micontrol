# Sistema de logs do MiControl

Este documento descreve **onde** ficam os logs, **como** rodam, **que marcadores**
existem e **como ler** um ficheiro sem ruído. Foi escrito a partir da auditoria
de 2026-11-07 (`audit-20261107/findings-applog.md`), que encontrou linhas
corrompidas por escrita concorrente (P5), 30 h 44 min sem qualquer marcador de
ciclo de vida (P4) e campos de “duração” a imprimir *epoch* (P6).

---

## 1. Localizações

### App e helper elevado — `%LOCALAPPDATA%\MiControl\logs`

`%LOCALAPPDATA%` = `C:\Users\<utilizador>\AppData\Local`.

| Processo | Ficheiro | Nível | Notas |
| --- | --- | --- | --- |
| `micontrol.exe` (app) | `tauri-app.log` | Info | log principal; rotação 8 MiB × 7 |
| `micontrol.exe --elevated` (helper) | `tauri-app-helper.log` | Info | **ficheiro separado** desde o fix do P5; rotação 8 MiB × 7 |
| `cargo tauri dev` | `tauri-dev-trace.log` | Trace/Info | `MICONTROL_DEV_TRACE=0` baixa para Info |
| helper em dev | `tauri-dev-trace-helper.log` | Trace/Info | separado pelo mesmo motivo |
| erros do utilizador | `errors.log` | Error | 1 MB × 5, retenção 7 dias (`util::error_log`) |
| estado da sessão | `session.marker` | — | marcador de ciclo de vida (ver §3) |

### Serviço bridge / watchdog — `%PROGRAMDATA%\MiControl\logs`

| Processo | Ficheiro | Notas |
| --- | --- | --- |
| `MiControlBridge` (SYSTEM) | `bridge.log` | rotação 8 MiB × 7; timestamp RFC 3339 por linha |
| watchdog do bridge | `watchdog.log` | rotação 8 MiB × 7 |

O bridge corre como `LOCAL SYSTEM` e não vê o perfil do utilizador — por isso
os logs dele vivem em `ProgramData`, não em `LOCALAPPDATA`.

### Porque é que o helper tem um ficheiro próprio (P5)

A app e cada invocação do helper (`--elevated`) escreviam **no mesmo**
`tauri-app.log`. Com 3 728 arranques de helper num só dia, dois processos
abriam o ficheiro em modo *append* e as escritas intercalavam-se ao meio da
linha:

```text
2026-11-06T22:21:00.115Z2026-11-06T22:21:04.592Z [WARN ] hw::battery: ...
```

A rotação também era insegura: o processo A renomeava o ficheiro enquanto o B
tinha um handle aberto para o ficheiro antigo. A separação por processo resolve
as duas coisas. **Regra:** um ficheiro de log tem exatamente um processo
escritor; nunca apontar dois processos para o mesmo caminho.

---

## 2. Rotação e retenção

- Limite por ficheiro: **8 MiB** (`DEFAULT_MAX_BYTES`).
- Ficheiros rotados mantidos: **7** (`DEFAULT_MAX_FILES`) → `.1` é o mais
  recente, `.7` o mais antigo.
- Teto de disco por família de log: **8 MiB × (1 + 7) = 64 MiB**.
- A rotação acontece **antes** de escrever: quando o ficheiro atinge o limite,
  é renomeado para `.1`, os restantes deslizam e o `.7` é apagado. O registo
  que disparou a rotação já entra no ficheiro novo.
- Um registo maior do que o limite roda uma vez e é escrito na mesma; nunca há
  ciclo infinito de rotação.

A retenção foi aumentada de 8 MiB × 3 para 8 MiB × 7 no fix de 2026-11-07: com
3 ficheiros, a janela de ~30 h que a auditoria precisou de percorrer já não
estava toda disponível.

Para inspecionar do mais antigo para o mais recente:

```bash
for f in tauri-app.log.7 tauri-app.log.6 tauri-app.log.5 tauri-app.log.4 \
         tauri-app.log.3 tauri-app.log.2 tauri-app.log.1 tauri-app.log; do
  [ -f "$f" ] && cat "$f"
done
```

---

## 3. Marcadores de ciclo de vida (P4)

O log tem de responder a uma pergunta simples: **a sessão anterior acabou bem?**

### `session.marker`

Fica ao lado do log (`%LOCALAPPDATA%\MiControl\logs\session.marker`) e é texto
simples, escrito de forma atómica (ficheiro temporário + `rename`):

```text
# MiControl session marker — written by the app, read at startup.
state=running
started_at=2026-11-07T09:12:03.114Z
pid=4242
version=0.2.20
```

| `state` | Significado |
| --- | --- |
| `running` | a sessão arrancou e ainda não registou shutdown |
| `clean` | passou por `app shutdown initiated (clean)` e saiu |
| `panic` | o panic hook registou a falha |

No arranque seguinte, a app lê o marcador e escreve **uma** destas linhas:

```text
[INFO ] applog: previous session ended cleanly at 2026-11-06T22:20:10.001Z
[WARN ] applog: previous session ended without shutdown marker — possible crash/kill (session started 2026-11-06T17:39:33.125Z)
[WARN ] applog: previous session ended in a panic at 2026-11-06T22:19:40.517Z
[INFO ] applog: no previous session marker (first run or clean log dir)
```

`state=running` no arranque = o processo foi morto (task kill, `TerminateProcess`,
power loss, crash antes do hook). É exatamente o buraco de 30 h 44 min que a
auditoria não conseguia explicar.

### Shutdown limpo

Todos os caminhos de saída limpa passam por `RunEvent::Exit` (Quit no tray,
`app.exit(0)` de qualquer comando, fecho da janela em dev). Aí é escrito:

```text
[INFO ] applog: app shutdown initiated (clean)
```

e o marcador passa a `state=clean`. `mark_clean_shutdown()` é idempotente: só
regista a primeira chamada.

### Panic

O panic hook global regista o marcador e escreve uma linha com o prefixo
grepável **`PANIC:`**:

```text
[ERROR] applog: PANIC: at src/hw/foo.rs:123:9: index out of bounds
```

O helper elevado não escreve no marcador da app — ele sai por
`TerminateProcess` de propósito (ver `elevated.rs`, S66-001) e não tem
shutdown limpo por desenho; o log dele é o `tauri-app-helper.log`.

### O que **não** é um crash

- O helper elevado terminar abruptamente (`TerminateProcess`) — é intencional.
- O bridge reiniciar por política do serviço — fica registado em `bridge.log`.
- Uma rotação de log — não afeta o marcador.

---

## 4. Deduplicação de `WARN` / `ERROR`

Uma condição constante (sonda do bridge, timeout WMI da bateria, fallback IGCL)
produzia centenas de linhas iguais: 665 linhas para uma única condição na
auditoria. O logger agora suprime repetições:

- **Assinatura** = nível + `target` + mensagem com os blocos de dígitos
  colapsados em `#` (`attempt=1`, `attempt=2` e `took 5012 ms` caem na mesma
  assinatura). O texto impresso **não** é alterado.
- **Janela**: 60 s. A primeira ocorrência é sempre emitida.
- **Estado**: tabela fixa de 64 assinaturas com despejo LRU — memória constante,
  sem `HashMap` a crescer.
- **Resumo**: quando a janela vira, ou quando o mesmo erro volta depois de um
  período de silêncio, é emitida uma linha:

```text
2026-11-07T10:05:00.000Z [INFO ] applog: ×12 repetições suprimidas (mesma assinatura em 60s)
```

- Só `WARN` e `ERROR` são deduplicados. `INFO`/`DEBUG`/`TRACE`, os marcadores de
  ciclo de vida e as linhas `PANIC:` nunca são suprimidos.
- O limiter usa `try_lock`: se estiver ocupado (por exemplo, o panic hook a
  escrever enquanto outra thread formata), o registo é emitido sem filtro em vez
  de bloquear ou entrar em deadlock.

Para ver **todas** as ocorrências de um erro que foi deduplicado, procure o
resumo `×N repetições suprimidas` e conte a partir da primeira linha; o resumo
diz quantas foram omitidas.

---

## 5. Escrita de linha inteira

Cada registo é entregue ao sistema num **único** `write_all` sobre um handle
aberto em modo *append*. No Windows isso corresponde a `FILE_APPEND_DATA`: o
kernel coloca o buffer inteiro no fim do ficheiro, pelo que dois escritores não
se misturam dentro da mesma linha. O formato do registo inclui o `\n` final e o
`fern` é configurado com separador vazio precisamente para que a linha seja uma
unidade — e para que um registo suprimido não deixe uma linha em branco.

Consequências práticas:

- Uma linha física = um registo. Nunca dois timestamps na mesma linha.
- `append_line` (bridge/watchdog) constrói a linha completa em memória antes de
  escrever e é o **único** sítio que acrescenta o timestamp; não voltar a
  prefixar `{ts}` nas chamadas.
- Ficheiros de log não são partilhados entre processos (§1).

---

## 6. Como ler sem poluição

### Últimas linhas em tempo real (PowerShell)

```powershell
Get-Content "$env:LOCALAPPDATA\MiControl\logs\tauri-app.log" -Tail 200 -Wait
```

### Só problemas

```bash
cd "$LOCALAPPDATA/MiControl/logs"
grep -E '\[(WARN|ERROR)\]' tauri-app.log | tail -100
```

### Linha temporal (app + helper intercalados)

```bash
cd "$LOCALAPPDATA/MiControl/logs"
cat tauri-app-helper.log tauri-app.log | sort | less
```

O timestamp RFC 3339 no início de cada linha torna a ordenação lexical
equivalente à cronológica (tudo em UTC, com milissegundos).

### Marcadores e eventos de ciclo de vida

```bash
grep -nE 'app shutdown initiated|previous session|PANIC:|repetições suprimidas|persistent (app|dev) logging enabled' tauri-app.log
cat session.marker
```

### Um comando por linha

```bash
grep -n 'elevated command' tauri-app-helper.log | tail -50
grep -n 'battery WMI query exceeded' tauri-app.log
```

### Ler um relatório de bug

1. `session.marker` — a sessão anterior acabou bem?
2. `grep 'previous session' tauri-app.log` — confirma o veredicto no log.
3. `grep -E '\[(WARN|ERROR)\]'` — problemas, já sem spam repetido.
4. `grep 'PANIC:'` — houve pânico? A linha tem ficheiro:linha:coluna.
5. `tauri-app-helper.log` — o que os comandos elevados fizeram (o log da app já
   não é poluído por eles).

---

## 7. O que não vai para estes logs

- Segredos, tokens e chaves HMAC nunca são impressos (o pipeline de crash report
  faz *scrubbing* de PII; o log não recebe credenciais).
- Erros visíveis ao utilizador vão também para `errors.log` (retenção 7 dias,
  lido pela UI via `read_error_log`).
- O bridge não escreve em `LOCALAPPDATA`: os logs dele são os de
  `%PROGRAMDATA%\MiControl\logs`.

---

## 8. Manutenção

- Constantes de rotação: `src-tauri/src/util/log_rotation.rs`
  (`DEFAULT_MAX_BYTES`, `DEFAULT_MAX_FILES`) — há um teste que as fixa em
  8 MiB × 7.
- Nomes de ficheiro e encaminhamento por processo: `src-tauri/src/debug_log.rs`
  (`APP_LOG_FILE`, `HELPER_LOG_FILE`, `log_file_name`).
- Deduplicação: `src-tauri/src/util/log_ratelimit.rs`.
- Marcadores de sessão: `src-tauri/src/util/session_marker.rs`.
- Campos de duração: `src-tauri/src/util/log_fields.rs` — usar
  `duration_field()` em vez de `now - 0`; o sentinela `0` significa “nunca
  registado” e imprime `unset`, não *epoch*.