# MiControl - Estado Atual da Documentação

## Aviso Importante

Esta é a **única documentação ativa** do projeto neste momento.

O projeto está **em construção** e este documento pode conter pontos **desatualizados** enquanto funcionalidades, arquitetura e fluxos internos continuam evoluindo.

Toda documentação anterior foi movida para `docs/deprecated/`.

---

## Checklist de Reorganização

- [x] Consolidar documentação ativa em um único arquivo.
- [x] Mover documentos antigos para `docs/deprecated/`.
- [x] Marcar explicitamente o status "em construção" e risco de desatualização.
- [x] Revisar este documento ao fim de cada ciclo de implementação relevante.
- [x] Vincular cada mudança importante de backend/frontend a um item do roteiro técnico abaixo.
- [ ] Definir critério de "documentação estável" para saída de beta interno.

---

## Changelog Simples

### 2026-05-29

- **WiFi Manager reescrito** para gerenciar WiFi do próprio PC via `netsh wlan` (era gerenciador de WiFi de dispositivo IoT via IPC).
- Novo módulo Rust `hw/wifi.rs` com funções `scan_networks`, `get_status`, `connect`, `disconnect`.
- 4 novos comandos Tauri registrados: `wifi_scan`, `wifi_status`, `wifi_connect`, `wifi_disconnect`.
- `WiFiManager.tsx` reescrito do zero: escaneia redes disponíveis, exibe sinal/segurança, conecta/desconecta com senha.
- `ScreenCast.tsx` atualizado para usar i18n (`t("cast.*")`); lógica inalterada.
- Card "EC Debug" removido do TrayPopup; substituído por card "WiFi" que navega para aba WiFi Manager.
- Seções `"wifi"` e `"cast"` adicionadas a todos os 4 arquivos de locale (`en`, `pt`, `es`, `fr`).
- Checks executados após mudanças:
  - `cargo test -p micontrol` — 48 testes passando
  - `npx tsc --noEmit` — 0 erros
  - `cargo build --release` — build OK

### 2026-05-23

- Hardening do bridge elevado com `request_id` por chamada e arquivos de comando/resultado por requisição.
- Serialização de chamadas elevadas para evitar corrida entre pedidos simultâneos.
- Validação de `request_id` na leitura do resultado elevado.
- Fluxo `install_driver` endurecido: backend elevado passa a aceitar `driver_name` e resolve/valida `.inf` internamente.
- Adicionada validação de segurança de caminho para `.inf` canônico dentro de `resources`.
- Escrita ECRAM protegida por regra de segurança:
- allowlist para escritas conhecidas e seguras.
- escrita avançada condicionada à variável `MICONTROL_ENABLE_RAW_ECRAM_WRITE=1`.
- limite de payload e bloqueio de faixa fora de ERAM.
- Testes adicionados para guard-rails de ECRAM.
- Checks executados após mudanças:
- `cargo fmt`
- `cargo test`
- `npm run build`
- `npm test -- --run`
- Atualização do perfil global de hardware para runtime após `rediscover` (sem reiniciar o app).
- `global_profile` tornou-se atualizável em tempo de execução com leitura por snapshot.
- `useHardware` ganhou sinalização granular de erro por subsistema (`refreshErrors`) e resumo de falhas no refresh.
- Sidebar principal agora exibe falhas por subsistema para diagnóstico rápido.
- Warnings limpos em `hotkeys.rs` (retorno de `PeekMessageW`, campo WMI não usado, variável default não usada).
- Teste frontend `ChargingThreshold` ajustado para fluxo assíncrono com `userEvent`/`waitFor`, removendo warning de `act(...)`.
- Checks executados após esta rodada:
- `cargo fmt --all`
- `cargo test`
- `npm run build`
- `npm test -- --run`

---

## Próximos Passos Imediatos

- [x] Endurecer bridge elevado com correlação por requisição.
- [x] Revalidar instalação de driver no processo elevado.
- [x] Implementar controles de segurança para escrita ECRAM.
- [x] Executar checks de build e testes após hardening.
- [x] Atualizar perfil global de hardware em runtime após `rediscover`.
- [x] Melhorar sinalização de erro por subsistema no `useHardware`.
- [x] Reduzir warnings pendentes no backend (`hotkeys.rs`) e testes frontend (`act(...)`).
- [x] Reescrever WiFi Manager para gerenciar WiFi do PC via `netsh wlan` (não IoT device).
- [x] Adicionar i18n ao ScreenCast.tsx.
- [x] Remover EC Debug do TrayPopup; adicionar card WiFi.
- [x] Adicionar traduções `wifi` e `cast` para EN, PT, ES, FR.
- [ ] Implementar enumeração de dispositivos Miracast via WinRT (atualmente retorna lista vazia).
- [ ] Definir critério de "documentação estável" para saída de beta interno.

## Roteiro Técnico Detalhado

## 1. Objetivo Técnico do Produto

Entregar um app desktop Tauri (Rust + React) para controle de hardware Xiaomi no Windows com foco em:

- Performance mode (WMI/VHF/overlay Windows)
- Charging threshold (IoT pipe/registry)
- Telemetria de sistema (CPU/GPU/bateria/display/fan)
- Descoberta e instalação guiada de drivers
- Hotkeys e recursos avançados (touchpad, OSD, ECRAM)

## 2. Estado Arquitetural Atual

### Frontend (`src/`)
- React + TypeScript + Vite.
- `useHardware` concentra chamadas Tauri (`invoke`) e estado de hardware.
- `MainWindow.tsx` agrega múltiplas abas e parte relevante da lógica de UI/fluxo.
- Testes existentes com Vitest para componentes-chave e i18n.

### Backend (`src-tauri/src/`)
- Camada `commands/*` como API Tauri.
- Camada `hw/*` com integrações Win32/WMI/HID/IoT.
- Fluxo de elevação de privilégio via bridge (`elev_bridge.rs` + `elevated.rs`).
- Descoberta de hardware com cache de perfil.

## 3. Prioridades Técnicas (Ordem Recomendada)

## Fase 1 - Endurecimento de segurança da elevação

1. Remover dependência de arquivos previsíveis de comando/resultado sem correlação robusta.
2. Introduzir identificador único por requisição e validação de origem.
3. Revalidar no processo elevado todos os argumentos sensíveis (especialmente instalação de driver e escrita de hardware).
4. Limitar superfície de comandos elevados a uma allowlist estrita.

## Fase 2 - Contenção de operações de alto risco (ECRAM)

1. Restringir escrita bruta em ECRAM por feature flag de debug.
2. Implementar allowlist por região/offset para operações de produção.
3. Exigir confirmação forte e logging de auditoria local para cada escrita.
4. Garantir limites de tamanho e sanitização completa dos payloads hex.

## Fase 3 - Consistência de estado e sincronização

1. Revisar `useHardware` para evitar estado silenciosamente stale em falhas parciais.
2. Tornar status de erro granular por subsistema (bateria/display/fan/performance).
3. Atualizar o perfil global de hardware em runtime após rediscovery (sem exigir restart, se viável).
4. Definir política de polling/backoff e timeout por comando.

## Fase 4 - Refatoração estrutural de módulos grandes

1. Dividir `MainWindow.tsx` por domínio de aba e handlers dedicados.
2. Dividir módulos Rust extensos (`hotkeys.rs`, `touchpad.rs`, `display.rs`) em submódulos orientados a responsabilidade.
3. Padronizar contratos de erro entre `hw/*` e `commands/*`.
4. Criar camadas utilitárias para conversão/serialização repetida.

## Fase 5 - Qualidade e testes

1. Ampliar cobertura de testes para:
   - bridge elevado
   - validação de comandos privilegiados
   - discovery/install driver
   - parsing e limites de operações ECRAM
2. Adicionar lint e checks automáticos:
   - frontend: lint + test + build
   - backend: fmt + clippy + test
3. Definir suite mínima obrigatória para merge.

## Fase 6 - Observabilidade e diagnóstico

1. Padronizar logs estruturados por subsistema (`security`, `performance`, `iot`, `ui`).
2. Inserir códigos de erro estáveis para falhas críticas.
3. Criar visão de diagnóstico no app para exportar snapshot técnico.
4. Registrar métricas de latência e taxa de falha dos comandos Tauri.

## Fase 7 - UX técnica e operação

1. Melhorar feedback de erro acionável na UI para casos de permissão/driver ausente.
2. Garantir fallback explícito quando canais primários (WMI/VHF/IoT) não estiverem disponíveis.
3. Definir fluxo de recuperação pós-falha (re-scan, reinstalação driver, modo seguro).
4. Revisar experiência de setup inicial em hardware não homologado.

## Fase 8 - Critérios de prontidão

Concluir beta interno somente quando:

1. Operações elevadas estiverem com validação forte e rastreabilidade mínima.
2. Escritas de hardware de risco estiverem protegidas por políticas de segurança.
3. Cobertura de testes de fluxos críticos estiver estável.
4. Falhas críticas tiverem mensagens de recuperação claras para usuário.

---

## Política de Documentação a Partir de Agora

1. Este arquivo permanece como fonte única de referência ativa.
2. Toda documentação antiga ou parcialmente inválida deve ir para `docs/deprecated/`.
3. Mudanças de arquitetura/segurança devem atualizar primeiro o checklist e o roteiro acima.

---

## Development

### Prerequisites
- Node.js 20+
- Rust (stable)
- Windows SDK (for Tauri builds)

### Running Checks Locally

**Rust:**
```bash
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo fmt --check --manifest-path src-tauri/Cargo.toml
```

**Frontend:**
```bash
npm ci
npx tsc --noEmit
npm run lint
npm run format:check
npm run build
```

**Full Tauri build:**
```bash
npm run tauri build
```

These checks run automatically in CI on every pull request.
