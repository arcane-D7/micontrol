# Contratos universais do miControl — revisão pré-implementação (2026-11-19)

Âmbito: centralizar spawn, erros, estado e timeouts **sem** reescrever a app. Cada módulo continua dono do seu domínio; passa a cumprir quatro contratos e um gate. Base: `audits/2026-11-19-architecture-governor/reports/architecture.md` (D1-D14, U1-U9) e `audits/2026-11-18-full-app/`.

## 0. Diagnóstico em números

| Categoria | Hoje |
|---|---|
| `Command::new` | 119 sítios, **0 helpers**; 24 definições de `CREATE_NO_WINDOW` em 16 ficheiros |
| Flashes de consola | **16 reais** (`sys_opt.rs:1039,1053,1075` pisca a cada toggle) em 23 sítios sem guarda |
| `.output()` sem timeout | **98** (17 em `elevated.rs`, 14 em `ecram_service_mgmt.rs`, 13 em `micontrol_bridge.rs`) |
| `sc.exe` | 22+ spawns em 5 ficheiros vs `util/win_service.rs` nativo (só query/start) |
| `schtasks` | 3 implementações em 4 ficheiros |
| PDH | 4 pollers com o mesmo boilerplate `LoadLibrary` |
| Erros | `HardwareError` (38 ficheiros) vs **248 `Result<_, String>`**, 223 `map_err(format!`, 4 enums locais |
| Persistência | ~15 `persist_*`/`load_*` locais, cada um com o seu path, IO e schema |
| Caches | 157 statics em 44 ficheiros; trio `backoff` copiado em `fan.rs` e `thermal.rs` |

## 1. Contrato de spawn — `util/process_spawn.rs` (novo)

Uma porta única para criar processos. `CREATE_NO_WINDOW` é o comportamento **por omissão**; ver uma janela passa a ser uma excepção explícita e justificada.

```rust
pub struct SpawnOutput { pub status: ExitStatus, pub stdout: Vec<u8>, pub stderr: Vec<u8>, pub elapsed: Duration }

pub enum SpawnPolicy {
    Quiet,                  // CREATE_NO_WINDOW (por omissão)
    Visible { reason: &'static str },  // exige justificação no código
}

pub fn run(cmd: &mut Command, policy: SpawnPolicy, timeout: Duration) -> Result<SpawnOutput, SpawnError>;
pub fn run_capture(cmd: &mut Command, timeout: Duration) -> Result<SpawnOutput, SpawnError>; // Quiet
pub fn run_checked(cmd: &mut Command, timeout: Duration) -> Result<SpawnOutput, SpawnError>; // falha se status != 0
pub async fn run_async(cmd: &mut Command, timeout: Duration) -> Result<SpawnOutput, SpawnError>;
pub fn spawn_detached(cmd: &mut Command, policy: SpawnPolicy) -> Result<Child, SpawnError>; // sem timeout = decisão explícita
```

Regras:
1. **Timeout obrigatório** em `run*`. No estouro: `TerminateProcess` no filho **e** na árvore (Job Object com `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`), e `SpawnError::Timeout { program, elapsed }`.
2. **Sem `Command::new` fora deste módulo** — gate estático `check-windowless-spawns.mjs` (já ligado) evolui para proibir `Command` directo em `src-tauri/src`, com allowlist temporária por ficheiro.
3. `Visible` só para `explorer`, `dccw` e o instalador; cada uso leva `reason`.
4. `run_capture` aplica um tecto de 1 MiB por stream (evita o OOM do `output()` com `schtasks /query` gigante).
5. Toast/notificação e `schtasks` passam a usar `run_capture`; `sc.exe` desaparece atrás de `util/win_service.rs`.

Migração: 16 flashes primeiro (`sys_opt.rs` reg.exe, `transcription`, `scrcpy_bridge`, `crash_recovery`, `debug_log`, `myicontrol_face_svc`), depois os 98 `.output()` por módulo (elevated, ecram_service_mgmt, bridge, sys_opt).

## 2. Mapa de erros — `errors/catalog.rs` (novo) + `AppError`

Nenhum erro nasce solto: cada código previsto existe num mapa central com mensagem, severidade e política de retry. O código referencia o código, não uma string.

```rust
pub enum ErrorCode { EcramDriverBlocked, EcramServiceUnavailable, WmiTimeout, SpawnTimeout, ServiceAccessDenied, /* ... */ }

pub struct ErrorSpec {
    pub code: ErrorCode,
    pub id: &'static str,          // "ecram.driver_blocked" — estável, vai para logs
    pub i18n_key: &'static str,    // "errors.ecram.driverBlocked"
    pub severity: Severity,        // Info | Warning | Error | Fatal
    pub retry: RetryPolicy,        // Never | Backoff { base_ms, max }
    pub user_action: UserAction,   // None | Retry | Elevate | Reinstall | ContactSupport
}

pub struct AppError { pub code: ErrorCode, pub context: String, pub source: Option<Box<dyn Error>> }
```

Regras:
1. **Todo o erro novo de `src-tauri/src` é `AppError`**; `Result<_, String>` fica proibido em código novo e é migrado módulo a módulo (nunca big-bang nos 248).
2. `HardwareError` mantém-se como tipo de domínio e ganha `fn code(&self) -> ErrorCode`.
3. **Gate `check-error-catalog.mjs`**: (a) cada `ErrorCode` tem `id`, `i18n_key` e entrada nos 4 locales; (b) sem ids duplicados; (c) `AppError` só constrói com um código do mapa; (d) falha se um ficheiro tocado em `src-tauri/src` tiver `Result<_, String>` novo (diff-based).
4. O mapa é a fonte das mensagens da UI e do log: a UI mostra `i18n_key` + `user_action`, o log guarda `id` + contexto. Nunca se mostra `Display` de um erro Win32 cru.

## 3. Store global + substores — `util/state_store.rs` (novo)

Um template universal para todo o estado persistido. O estado global vive num `StoreRegistry`; cada domínio isolado recebe um `SubStore` governado pelo mesmo contrato.

```rust
pub struct StoreSpec {
    pub name: &'static str,          // "brightness", "governor.policy"
    pub path: PathBuf,               // definido pelo registry, nunca pelo módulo
    pub schema: u32,                 // versão do payload
    pub acl: AclPolicy,              // User | System | SystemAndAdmins
    pub max_bytes: u64,              // recusa payloads acima do tecto
}

pub struct Store<T: Serialize + DeserializeOwned> { spec: StoreSpec, _marker: PhantomData<T> }

impl<T> Store<T> {
    pub fn load(&self) -> Result<Option<T>, StoreError>;      // schema antigo → migrate
    pub fn save(&self, value: &T) -> Result<(), StoreError>;  // tmp + rename atómico + fsync
    pub fn update<F: FnOnce(&mut T)>(&self, f: F) -> Result<(), StoreError>; // lock de ficheiro
    pub fn purge(&self) -> Result<(), StoreError>;
}
```

Regras:
1. **Nenhum módulo escolhe o próprio path**: o `StoreRegistry` (um sítio) resolve `%LOCALAPPDATA%` vs `%PROGRAMDATA%` e aplica a ACL. Isto fecha a classe do GSEC-02 (herança de Users:Write).
2. **Escrita atómica**: `.tmp` + `ReplaceFile`/rename + fsync; nunca `fs::write` directo para estado.
3. **Schema versionado com migração**: cada store declara `schema` e uma função `migrate(from)`. Um payload de versão desconhecida é recusado, não reinterpretado.
4. **Purga** é uma operação do store (`purge`), o que torna o RF-15 testável.
5. Migração dos ~15 `persist_*` locais: módulo a módulo, começando pelos que o governor toca (`policy`, `state`, `ledger`) e pelos que já têm bug de ficheiro (`brightness_override`, `smart_model`).

## 4. Template de output com timeout — `util/blocking.rs` + `process_spawn`

1. Todo o `run*` tem `timeout` obrigatório (ver §1); não existe `output()` directo em `src-tauri/src`.
2. `blocking.rs` passa a expor `run_blocking<F, T>(label, budget, f)` para trabalho síncrono em contexto async, com `spawn_blocking` + `tokio::time::timeout` + log do estouro.
3. Estouro de tempo = processo morto (árvore) + `AppError { code: SpawnTimeout }` no ledger, nunca um `Err(String)`.
4. Gate `check-spawn-timeout.mjs` (extensão do spawn gate): falha em `.output()`/`.status()`/`.wait()` sem timeout associado no mesmo bloco.

## 5. Contratos de módulo — `docs/architecture/MODULE-CONTRACTS.md`

Cada módulo declara, num bloco fixo no topo do ficheiro:

```rust
// contract: owner=hw::fan; stores=[fan.profile]; spawn=forbidden; errors=[FanReadFailed, FanWriteRefused]; timeout_ms=2000
```

Um gate (`check-module-contracts.mjs`) verifica que: (a) todos os módulos de `hw/` têm o bloco; (b) os stores que usam existem no registry; (c) os códigos de erro existem no catálogo; (d) um módulo com `spawn=forbidden` não chama `process_spawn`. Isto é o "independentes mas padronizados" pedido: o contrato é declarativo e o gate é a única autoridade.

## 6. Fases

| Fase | Entrega | Gate |
|---|---|---|
| **C0** (1-2 d) | `process_spawn` + migração dos 16 flashes + `check-spawn-timeout`; `errors/catalog` com os códigos existentes + `AppError` para código novo; `state_store` + 3 pilotos (`brightness`, `fan`, `governor.policy`); `MODULE-CONTRACTS.md` para os 6 módulos do governor | gate de spawns a 0 flashes; catálogo completo nos 4 locales; 3 stores com teste de migração e purga |
| **C1** (2-3 d) | U3 `win_service` (22+ sc.exe), U4 `schtasks`, U5 motor PDH único | serviços com readback igual; `taskmgr` < 300 ms com 400 processos |
| **C2** (2 d) | Timeouts nos 98 `.output()` por prioridade (elevated, ecram_service_mgmt, bridge) | zero `.output()` sem timeout; teste de processo que não morre é morto em ≤ timeout |
| **C3** (2-3 d) | Migração dos `persist_*` para `state_store`; U8 registry nos módulos tocados | nenhum `fs::write` de estado fora do store; migrações testadas |
| **C4** (incremental) | Migração de erros por módulo; gate diff-based a bloquear `String` novo | 0 `Result<_, String>` novo por PR |

## 7. O que não se faz

- Big-bang nos 248 `Result<_, String>` nem nos 157 statics.
- Tocar em `os_turbo` (energia), `hw/startup.rs` (autostart) ou no servidor MCP próprio (9223) nesta ronda.
- Substituir o `elevated.rs` inteiro; o split (3 227 l.) fica para o início de F3.
- Migrar o PowerShell dos hotkeys: é feature com allowlist System32; fica.
- Adicionar um 5.º poller PDH: o governor consome o motor da U5.

## 8. Riscos

| Risco | Mitigação |
|---|---|
| Helper de spawn com bug parte todos os fluxos | Migração por módulo, testes de timeout/árvore, rollback por commit |
| `state_store` atómico em ficheiros quentes (brightness 2 s) | Escrita só quando o valor muda; medir IO antes/depois |
| Catálogo de erros vira burocracia | Códigos só para erros que a UI trata ou o log distingue; não para `unwrap` interno |
| Contratos declarativos apodrecem | O gate lê o bloco; um módulo sem bloco falha |
