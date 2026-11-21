# FIX-REVIEW — matriz de verificação de todos os fixes da campanha

**Data:** 2026-11-11 · **Branch:** `f5/fix-review` · **Base = produção:** `integrate/audit-fixes` @ `3c734f4` (deploy 10-Nov 20:17)
**Âmbito:** verificar, um a um, que cada fix das vagas 1–5 continua presente e coerente no código actual, que os testes de regressão existem e passam, e procurar regressões introduzidas por merges posteriores. Não é um health gate completo (por instrução).

## TL;DR

| Métrica | Resultado |
|---|---|
| Fixes verificados | **87** |
| OK (código + teste de regressão a passar) | **87** |
| PARCIAL | **0** |
| REGRESSÃO (fix desaparecido/partido num merge) | **0** |
| Fixes de baixo risco aplicados nesta revisão | **1** (F-05, ver §5) |
| Dívida/riscos documentados | 12 (ver §4) |

**Conclusão:** nenhum fix da campanha foi revertido, apagado ou partido pelos merges até `3c734f4`. A única correcção de código feita nesta revisão é o F-05 (P1 do STAB-ROUND-1 que ficou por aplicar): o sweep de ficheiros stale deixou de poder abortar um comando elevado.

## 1. Evidência de execução (real, nesta worktree)

| Comando | Resultado |
|---|---|
| `cargo test --lib` | **925 passed; 0 failed; 14 ignored** (924 antes do F-05 + 1 teste novo) |
| `cargo test --lib --features face` | **927 passed; 0 failed; 14 ignored** |
| `cargo test --bins` | **40 passed; 0 failed** (16 `ecram_service` + 24 `micontrol_bridge`) |
| `cargo test --test logging_e2e` | **1 passed** (P4/P5/P6 + rate-limit + rotação, e2e com `LOCALAPPDATA` temporário) |
| `cargo check --features face` | OK (`Finished dev profile`) |
| `cargo clippy --lib --all-targets -- -D warnings` | limpo (só o warning de `build.rs` sobre o shell não construído) |
| `tsc --noEmit` | **0 erros** |
| `vitest run` | **40 ficheiros / 331 testes passed** |
| `node scripts/check-i18n.mjs` | OK — 1915 chaves em en/pt/es/fr, sem vazios nem drift de placeholders |
| Scan de emojis em `src/` | **0** |

Os 14 `#[ignore]` são todos testes live/hardware (radio BLE, ICC real, pipe IoT instalado, event log, catálogo ICC, Phone Link, startup HKLM, STA WMI) — nenhum teste de regressão foi desactivado. A integração `elevated_bridge`/`pipe_dacl_fix` **não** foi corrida de propósito: toca no serviço e na DACL reais desta máquina de produção.

## 2. Matriz de verificação

Legenda: ficheiro:linha é o código actual em `3c734f4`; "testes" é o filtro corrido e o nº de testes que passaram.

### 2.1 Vaga 1 — estabilidade e honestidade (07-Nov)

| # | Fix | Ficheiro(s) | Teste(s) | Veredicto | Evidência |
|---|---|---|---|---|---|
| 1 | C-01 pipe: `ERROR_PIPE_CONNECTED` tratado como conectado | `bin/micontrol_bridge.rs:908-975`, `1150-1165` | `cargo test --bins` → `pipe_server::c01_accept_tests::*` (4) | OK | `AcceptOutcome::{Connected,AlreadyConnected,Failed}`; o loop faz handoff nos dois estados de sucesso; teste `accept_returns_when_client_connects_after_connect_named_pipe` |
| 2 | RAII dos handles da pipe (`PipeInstance`, `OwnedHandle/OwnedEvent`, `into_raw`) | `bin/micontrol_bridge.rs:981-1060`, `util/win_handle.rs:69` | `cargo test --bins` (24) | OK | `Drop` fecha ambos; `into_handles()` liberta para o worker; comentário S67 |
| 3 | Watchdog: matriz de decisão (`parse_heartbeat` estrito, `ui_ok`, graças 60/120 s, orçamento) | `bin/micontrol_bridge.rs:2217-2248`, `2410-2630` | `cargo test --bins` → `watchdog::tests::*` (20) | OK | `rejects_empty_garbage_and_partial_heartbeats`, `restart_budget_is_bounded_then_gives_up`, `ui_ok_zero_*` |
| 4 | `crash_recovery` escala `runtime_starved` aos 10 falhos + marker | `hw/crash_recovery.rs:66`, `455-490` | `cargo test --lib hw::crash_recovery` (4) | OK | `escalation_starts_at_ten_consecutive_failures`, `starvation_marker_round_trips` |
| 5 | `leak_monitor` calibrado (10 amostras/10 min, piso 5000, warm-up 30 min) | `util/leak_monitor.rs:44-93` | `cargo test --lib util::leak_monitor` (11) | OK | `startup_ramp_false_positive_from_audit_is_suppressed`, `handle_leak_below_absolute_floor_is_quiet` |
| 6 | P1: detecção do serviço por SCM (`decide_bridge_action`) | `elev_bridge.rs:145` | `cargo test --lib elev_bridge` (13) | OK | `scm_states_collapse_to_actionable_buckets`, `stopped_service_is_started_not_reinstalled`, `running_service_is_available_even_when_the_pipe_is_wedged` |
| 7 | P8: política UAC explícita + resumo de arranque | `elev_bridge.rs:486-523`, `784-844` | `cargo test --lib elev_bridge` (13) | OK | `background_uac_fallback_enabled()` off por desenho; `boot_summary_*` (4) |
| 8 | P2: adapter power sem latch permanente | `hw/adapter_power.rs:151` (ProbeGate) | `cargo test --lib hw::adapter_power` (11) | OK | **Substituído por desenho** na vaga 4 (#63): o latch de 07-Nov era o bug; hoje é gate temporizado com retry 3× (`wmi_ec.rs:828-858`) |
| 9 | P15: fan cooldown RAII + coalescing (`ThermalCircuit`) | `hw/fan.rs:253-292` | `cargo test --lib hw::fan` (19) | OK | `circuit_opens_after_threshold_and_blocks_for_the_full_cooldown` |
| 10 | Bateria: latência real no log (não o threshold fixo) | `hw/battery.rs:142` | `cargo test --lib hw::battery` (15) | OK | `timeout_message_reports_measured_latency_not_the_threshold`, `previous query took` |
| 11 | P3: COM MTA idempotente por thread (`RPC_E_CHANGED_MODE`) | `hw/wmi_cache.rs:35-75` | `cargo test --lib` (compila) + `sta_thread_can_query_wmi` (ignored live) | OK | contador de `CoInitializeEx` por thread; `RPC_E_CHANGED_MODE` aceite em `startup_manager.rs:646`/`phone_link.rs:301` |
| 12 | P11: `ACCESS_DENIED` one-time + backoff 60→900 s | `hw/thermal.rs:119-156`, `hw/fan.rs:253-292`, `hw/wmi_cache.rs:376-384` | `cargo test --lib hw::wmi_cache`/`hw::fan` | OK | `is_access_denied`, `grow_failure_backoff` (60→120→240→480→900) |
| 13 | P10: `brightness_source` persistido + UI | `hw/display.rs:136`, `src/components/DisplaySettings.tsx:115` | `cargo test --lib hw::display` (36) | OK | `brightness_source_transition_is_persisted_with_reason`, `brightness_source_names_are_stable` |
| 14 | P7: recuperação de hotkey já registada | `hw/hotkeys/mod.rs:805-814` | `cargo test --lib hw::hotkeys` | OK | `register_hotkey_with_recovery` (`ERROR_HOTKEY_ALREADY_REGISTERED`), testes `already_registered_*` |
| 15 | P5: helper em `tauri-app-helper.log` (appends atómicos) | `debug_log.rs:18`, `util/log_rotation.rs` | `cargo test --test logging_e2e` (1) | OK | e2e escreve âmbito/rotação `8 MiB x 7` e uma linha por registo |
| 16 | P4: `session.marker` + shutdown/panic/crash/installer | `util/session_marker.rs` | `cargo test --lib util::session_marker` (13) | OK | `installer_shutdown_state_is_parsed_and_is_not_a_crash`, `machine_wide_marker_rescues_a_running_session_and_is_consumed` |
| 17 | P6: durações reais (nunca epoch) | `util/log_fields.rs` | `cargo test --lib util::log_fields` + e2e | OK | `duration_field`; e2e verifica que nenhuma linha imprime epoch |
| 18 | Rate-limit 60 s, LRU 64 slots | `util/log_ratelimit.rs:17` | `cargo test --lib util::log_ratelimit` (11) | OK | `RateLimiter` com tabela fixa de 64 e evicção LRU; e2e: 3 WARN idênticos → 1 linha |
| 19 | Rotação 8 MiB × 7 | `util/log_rotation.rs:40-46` | `cargo test --lib util::log_rotation` (13) | OK | `production_defaults_bound_total_disk_usage`, `keeps_at_most_max_files_and_drops_the_oldest` |
| 20 | Retry do WebView em `0x800700AA` (250 ms/1 s/2 s) | `util/webview_retry.rs:35`, `lib.rs:137` | `cargo test --lib util::webview_retry` (6) | OK | `retries_twice_then_succeeds_on_third_attempt`, `permanent_error_is_not_retried` |
| 21 | Tray popup recriada se 0×0 | `lib.rs:1347-1396` | `vitest TrayPopup/TrayThemeRegression` | OK | `[tray] popup physical size looks stale (0x0 px …)` → recreate + re-check |
| 22 | Brilho automático: clamp ±100 (intenção) / ±30 (captura) + prune 7 d + anti-eco | `hw/display.rs:716/729/860/2026` | `cargo test --lib hw::display` (36) | OK | `capture_echo_does_not_undo_a_user_offset`, `captured_offset_clamp_limits_range`, `smart_model_prune_*` |
| 23 | Consent HMAC com `.old` na rotação | `util/consent_audit.rs:16` | `cargo test --lib util::consent_audit` (12) | OK | `entries_verify_against_current_or_previous_key`, `pin_key_files_preserves_previous_key` |

### 2.2 Vaga 2 — funcional (07-Nov, 20:22)

| # | Fix | Ficheiro(s) | Teste(s) | Veredicto | Evidência |
|---|---|---|---|---|---|
| 24 | Bateria: 3 estados com SSOT (`charging/acConnected/discharging`) | `src/lib/batteryState.ts:19`, `src/pages/TrayPopup.tsx:60` | `vitest batteryState.test.ts` (6) + `TrayPopup.test.tsx` | OK | `batteryState(false,true)==='acConnected'`; tray mostra "Charger connected" |
| 25 | Gate da sonda WMI morta (provider) | `hw/adapter_power.rs:151` | `cargo test --lib hw::adapter_power` (11) | OK | `gate_starts_open_and_closes_for_a_bounded_window`; log 1×/janela |
| 26 | `is_charging` pela flag `Charging` (não só rate>0) | `hw/battery.rs:55` | `cargo test --lib hw::battery` (15) | OK | `plugged_battery_with_charging_flag_charges_at_zero_rate`, `unplugged_battery_ignores_a_stale_charging_flag` |
| 27 | Blocos EC mostram `—` quando indisponíveis | `src/lib/ecram.ts:21-33`, `pages/tabs/setup.tsx` | `vitest IotModulePanel` | OK | `isEcramAvailable`/`ecramText`; `ecram_unavailable_hint` nos 4 locales |
| 28 | OS Turbo F1–F7 (snapshot vivo + recaptura, descoberta contínua, falhas≠drift, toggle re-aplica, restore no arranque, log, quadro medido) | `hw/os_turbo.rs:155/1176`, `hw/os_turbo_log.rs` | `cargo test --lib hw::os_turbo` (93) | OK | `enabled + dead snapshot must recapture and re-apply`, `drift_counts_pending_and_live_failures_but_not_terminations`, `restore_os_turbo()` chamado em `lib.rs:1024` |
| 29 | Display: regressão do clamp + eco de captura + refresh por dispositivo | `hw/display.rs:1128/3054-3097` | `cargo test --lib hw::display` (36) | OK | ver #22; `refresh-rate source: … internal=…` escolhe o painel interno, não o LG externo |
| 30 | IoT pipe BYTE (0x80070057) + guard MCPI + thread-per-client | `bin/ecram_service.rs:1131/1386/1423` | `cargo test --bins` → `mcpi_header_is_rejected_without_waiting_for_json` | OK | header binário rejeitado sem esperar por `}`; cliente servido em thread própria |
| 31 | `read_ecram_raw` com fallback pela pipe | `hw/ecram.rs:896`, `commands/hardware.rs:340` | `cargo test --lib hw::ecram` (21) | OK | `read_ecram_via_pipe_result` propaga o erro real |
| 32 | `get_ecram_map` honesto (`available/blocked_regions/blocked_reason`) | `hw/errors.rs:19`, `hw/ecram.rs` | `cargo test --lib hw::ecram` (21) | OK | `RegionBlockKind::{DriverBlocked,ServiceUnavailable,Unknown}` |
| 33 | Áudio: efeitos no HKCU + migração one-time | `hw/audio_effects.rs:9` | `cargo test --lib hw::audio_effects` (10) | OK | `migration_never_overwrites_an_explicit_user_preference`, `preferences_are_stored_in_the_user_hive_only` |
| 34 | Áudio: toggles honestos (`applies_to_audio:false`) | `hw/audio_effects.rs:21` | idem (10) | OK | comentário/flag `applies_to_audio`; sem PKEY enganador |
| 35 | "Definir como padrão" ligado | `hw/audio.rs:279` | `cargo test --lib hw::audio` (2) | OK | `set_default_endpoint` (IPolicyConfig) |
| 36 | Voice Clarity detetado por build+NPU | `hw/audio_effects.rs:70-111` | `cargo test --lib hw::audio_effects` (10) | OK | `voice_clarity_is_only_available_when_detected`, `voice_clarity_requires_a_new_enough_os_and_an_npu` |
| 37 | OSD Fn+F4 lê o estado real (0xE2 deixa de ser mic-mute) | `hw/hotkeys/mod.rs:2475-2530`, `hw/osd.rs:196` | `cargo test --lib hw::hotkeys` | OK | `detail[1]=0x21` → `set_system_mic_mute_osd(muted)`; sem branch 0xE2 |
| 38 | BLE presence: MAC canónica + fallback por nome | `hw/ble_presence.rs:564` | `cargo test --lib hw::ble_presence` (20) | OK | `canonical_mac_accepts_every_separator_format`, `target_matches_btleplug_colon_address_against_stored_dash_mac` |
| 39 | Phone Link: ativa a app real (IApplicationActivationManager) | `hw/phone_link.rs:96` | `cargo test --lib hw::phone_link` (10) | OK | `ActivateApplication` + verificação de janela |
| 40 | Cast honesto (painel real, sem prometer descoberta) | `hw/screen_cast.rs` | `cargo test --lib hw::screen_cast` (3) | OK | `list_cast_devices`/`start_casting` com estado real |
| 41 | Wi-Fi: perfil em memória + associação até 15 s (sem password em TEMP) | `hw/wifi.rs:660/836` | `cargo test --lib hw::wifi` (20) | OK | `set_profile_in_memory` ("no temporary file, so the passphrase never…"), `plan_profile` |
| 42 | BLE advertising re-armado no arranque | `hw/ble_advertise.rs` | `cargo test --lib hw::ble_advertise` | OK | publisher iniciado no arranque; WARN 0x80070005 único |
| 43 | Limpeza no contexto certo (user vs SYSTEM) | `hw/cleanup.rs:6` | `cargo test --lib hw::cleanup` (18) | OK | `every_category_has_exactly_one_context`, `system_context_refuses_user_categories` |
| 44 | Histórico de ameaças honesto | `hw/security_scan.rs:400` | `cargo test --lib hw::security_scan` (9) | OK | `Get-MpThreatDetection`; erro visível em vez de "Nenhuma ameaça" |
| 45 | Recycle Bin real (`SHQueryRecycleBinW`) | `hw/cleanup.rs:661` | `cargo test --lib hw::cleanup` (18) | OK | `shquery_rbinfo_layout_matches_c`, `recycle_counters_are_clamped` |
| 46 | Task manager sem NET falsa e sem `truncate(100)` | `hw/taskmgr.rs:310-535` | `cargo test --lib hw::taskmgr` (6) | OK | `NET_CACHE` + poller real; grep `truncate` = 0 |
| 47 | Drivers: estado real por `Win32_PnPEntity` | `hw/update.rs:394` | `cargo test --lib hw::update` | OK | query `DeviceID, Status, ConfigManagerErrorCode` |
| 48 | Toggle de auto-update governa os checks | `util/auto_update.rs:29`, `bin/micontrol_bridge.rs:151` | `cargo test --lib` (compila) | OK | `if !explicit_url && !is_enabled() { … }` antes de qualquer check |
| 49 | F2/F3: LocalSend consent + upload bounded + WlanAPI sem fugas | `hw/localsend.rs:33` | `cargo test --lib hw::localsend` (21) | OK | `consent_requires_a_ui_and_never_defaults_to_accepting`, `describe_transfer_enforces_the_limits_and_sanitizes_names` |
| 50 | F3: display HDR/DRRS honestos | `hw/display.rs` | `cargo test --lib hw::display` (36) | OK | `drrs_write_is_pending_until_the_next_boot`, `wizard_is_blocked_while_hdr_is_on` |
| 51 | F3: scenario rules pelo caminho elevado | `hw/scenario_rules.rs:37` | `cargo test --lib hw::scenario_rules` (11) | OK | `apply via the existing elevated path` |
| 52 | F3: fan RPM unavailable honesto | `hw/fan.rs:40` | `cargo test --lib hw::fan` (19) | OK | `speed_rpm: None` + `rpm_control` |
| 53 | F3: AI usage "não configurado" em vez de zeros | `src/components/AiUsagePanel.tsx:47` | `vitest AiUsagePanel` (3) | OK | `if (!configured) return <Card>…notConfigured` |

### 2.3 Vaga 4 / 4b / 4c / 4d — tema, firewall, BLE, edge slide, startup, cor

| # | Fix | Ficheiro(s) | Teste(s) | Veredicto | Evidência |
|---|---|---|---|---|---|
| 54 | Tokens hairline + 11 primitivos + 48 ícones | `src/styles/globals.css:5`, `src/components/ui/*` | `vitest themeTokens.test.ts` (328 linhas) + `uiPrimitives.test.tsx` | OK | pares de tema, amplitudes, 4,5:1/3:1, zero sombra/blur/`oklch(from…)` |
| 55 | Sidebar em 5 grupos + pesquisa + `aria-current` | `src/pages/MainWindow.tsx:74`, `ui/nav-item.tsx:18` | `vitest` (suite) | OK | `NAV_GROUPS` (5) + `aria-current={active?'page':undefined}` |
| 56 | a11y P0 (botões destrutivos nomeados, th ordenáveis) | `src/components/TaskManager…`, `ui/button.tsx` | `vitest TaskManagerA11y.test.tsx` | OK | testes de teclado/nome acessível |
| 57 | Tab Battery POC (fases, 3 estados de leitura, limite otimista) | `src/lib/batteryState.ts:60+`, `components/BatteryInfo.tsx` | `vitest BatteryTab*/BatteryInfo/chargePhase` | OK | `96% ligado à corrente com limite 80 lê-se como pausa`; `94% … firmware NÃO confirma não é pausa` (bug do dono) |
| 58 | fontes system-ui + `--error` alias + `color-mix` (12 sítios) | `src/styles/globals.css:103/120` | `themeTokens.test.ts` | OK | `--error: var(--danger)`; `color-mix(in oklab, …)`; zero `oklch(from` |
| 59 | Carregador: retry 3× (200/400 ms) + ProbeGate temporizado | `hw/wmi_ec.rs:828-858`, `hw/adapter_power.rs:151` | `cargo test --lib hw::adapter_power` (11) | OK | `ATTEMPTS=3`, backoff `200*(attempt+1)`; gate nunca permanente |
| 60 | MCP host-only (127.0.0.1:9223) + opt-in persistido | `util/mcp_config.rs:39` | `cargo test --lib` (compila) | OK | `Config::localhost_only()`; `MCPIntegrationEnabled` off por defeito |
| 61 | `firewall::apply` 9223 / 53317 TCP+UDP / 1716 UDP + block public | `util/firewall.rs:68-150` | `cargo test --lib util::firewall` (9) | OK | `public_profile_is_blocked_for_every_protocol`, `dev_build_artifacts_are_never_given_firewall_rules` |
| 62 | Setup shell 0 bytes: validador 500 KB..16 MB (3 guardas) | `build.rs:266`, `bootstrap.rs` | `cargo test --lib bootstrap` (5) | OK | `empty shell rejected`, `valid shell must not be re-downloaded` |
| 63 | Shutdown marker do instalador + `un.WriteInstallerShutdownMarker` | `nsis/installer.nsi:1000/1323` | `cargo test --lib util::session_marker` (13) | OK | `Function un.WriteInstallerShutdownMarker`; `Call un.…` no uninstaller |
| 64 | BLE LockGate: 3 amostras + janela 60 s + cooldown 120 s + histerese ±10 dB + grace 75 s | `hw/ble_presence.rs:61-72` | `cargo test --lib hw::ble_presence` (20) | OK | `one_far_sample_never_locks`, `three_far_samples_over_a_minute_lock`, `hysteresis_holds_far_until_ten_db_above_the_threshold` |
| 65 | Edge slide: `ClipCursor`, banda 16/28 % ajustável, deadband/pré-rolo/contacto | `hw/touchpad.rs:103-109/306` | `cargo test --lib hw::touchpad` (28) | OK | `edge_slide_real_geometry_emits_steps_at_250hz`, `edge_slide_releases_the_pin_on_the_first_frame_without_contact`, `edge_slide_survives_a_contact_slot_shuffle` |
| 66 | EC RAM calma (`RegionBlockKind`) + `IotModulePanel` único | `hw/errors.rs:19`, `components/IotModulePanel.tsx` | `vitest IotModulePanel` | OK | "—" + cartão neutro; `driver_blocked` ≠ serviço morto |
| 67 | Emojis → SVG (zero emojis) | `components/ui/icons.tsx:628` | scan Python em `src/` | OK | **0** caracteres emoji em ts/tsx/json/css |
| 68 | KDE discovery em porta fixa 1716 + teste live `#[ignore]` | `hw/kde_connect.rs:4`, `nsis/installer-hooks.nsi` | `cargo test --lib hw::kde_connect` | OK | regra "MiControl KDE Connect (UDP 1716)"; `discover_returns_empty_without_network` ignored |
| 69 | Accordion + InfoHint (touchpad help, 17 hints) | `ui/accordion.tsx`, `ui/info-hint.tsx` | `vitest TouchpadHelp.test.tsx` | OK | `aria-expanded/aria-describedby`, Escape |
| 70 | i18n 4 locales com paridade | `src/i18n/{en,pt,es,fr}.json` | `vitest i18nParity.test.ts` | OK | 1915 chaves iguais nos 4; `check-i18n` OK |
| 71 | Overview-plus (`get_overview_snapshot`, NPU, OnceLock) | `hw/overview.rs:328`, `commands/system.rs:370` | `cargo test --lib hw::overview` (13) | OK | 82 ms frio/1 ms quente; specs em `OnceLock` |
| 72 | Tray `surface-solid` sem ciclo de alias | `globals.css:92/280-294` | `themeTokens.test.ts` (§6) + `TrayThemeRegression` | OK | comentário do ciclo + teste que falha com alias circular |
| 73 | Startup manager (`StartupApproved`) + bridge `startup_toggle` | `hw/startup_manager.rs:49-53`, `elevated.rs:1798` | `cargo test --lib hw::startup_manager` (13) | OK | `validate_toggle_args` (hives fechados, HKCU recusado no elevado), `apply_elevated_toggle` |
| 74 | Cor: WCS `Associate/Disassociate/WcsGetDefaultColorProfile` | `hw/color_calibration.rs:463-472` | `cargo test --lib hw::color_calibration` (15; 1 live ignored) | OK | `live_xiaomi_profile_round_trips_through_wcs` (ignored, com guard de restauro) |
| 75 | EDID + catálogo ICC validado (sha256) + `install_icc_profile` | `hw/panel_edid.rs`, `hw/icc_catalog.rs`, `elevated.rs:1869` | `vitest ColorTab.catalog` + Rust | OK | catálogo 3/2/2 entradas; allowlist de nomes; re-hash como SYSTEM |

### 2.4 Vaga 5 — estabilização + perf/lid/battery (10-Nov 20:17)

| # | Fix | Ficheiro(s) | Teste(s) | Veredicto | Evidência |
|---|---|---|---|---|---|
| 76 | `fixed_pool` async (`tokio::oneshot` + `timeout().await`) + POOL_SIZE 8 | `util/fixed_pool.rs:25/44` | `cargo test --lib util::fixed_pool` (5) | OK | comentário "Do not reintroduce a synchronous receive here"; teste prova timers durante a espera |
| 77 | `iot_events` fora da task async (bateria no pool, budget 3 s) | `hw/iot_events.rs:286-320` | `cargo test --lib hw::iot_events` (3) | OK | `run_blocking_timeout(BATTERY_POLL_TIMEOUT, get_battery_info)` |
| 78 | `boundedPoll` (limite 2 + jitter + cancelamento por epoch) | `src/lib/boundedPoll.ts`, `hooks/useHardware.ts:275-290` | `vitest boundedPoll.test.ts` (93 linhas) | OK | `isCancelled` antes/depois do jitter; `epoch` no hook |
| 79 | Watchdog espera o :4000 livre (probe por bind, backoff, budget 10 s) | `bin/micontrol_bridge.rs:2834` | `cargo test --bins` → `watchdog::tests::{a_free_port…,wait_*}` | OK | `launch_when_mcp_port_is_free`; connect não envenena com TIME_WAIT |
| 80 | `BatteryGate` circuit breaker (3 timeouts, 30/60/120/240→300 s, half-open) | `hw/battery.rs:161-260` | `cargo test --lib hw::battery` (15) | OK | `three_consecutive_timeouts_open_the_circuit_with_backoff`, `a_reply_resets_the_breaker_and_the_streak` |
| 81 | Touchpad 2 s / startup 5 s | `commands/system.rs:277/471` | `cargo test --lib` (compila) | OK | `TOUCHPAD_READ_TIMEOUT=2s`, `STARTUP_ITEMS_TIMEOUT=5s` |
| 82 | JSDoc que partiu o tsc restaurado | `src/types/hardware.ts` | `tsc --noEmit` | OK | **0 erros** |
| 83 | Perf mode: `EcActiveMode` + `read_active_mode` + escrita verificada + fallback SSDB→QFAN | `hw/wmi_ec.rs:168/240/716`, `hw/fan.rs:1180-1240`, `hw/performance.rs:510` | `cargo test --lib hw::wmi_ec` (6) + `hw::fan` (19) | OK | `reportable_writes_need_an_exact_readback`, `smm_profile_writes_only_verify_on_a_real_transition`; UI `EcModeReadout` renderizado em Fan e Performance |
| 84 | Lid listener `GUID_LIDSWITCH_STATE_CHANGE` + snapshot térmico + `FanInfo.lid_open` | `hw/power_listener.rs:55/160`, `hw/fan.rs:997` | `cargo test --lib hw::power_listener` (5) | OK | `lid_payload_zero_is_closed_and_nonzero_is_open`, `unknown_state_is_not_closed` |
| 85 | Charge limit por ACPI WMAA `0x1000/0x02` + read-back obrigatório | `hw/charging.rs:171-228`, `elevated.rs:332-353` | `cargo test --lib hw::charging` (7) | OK | `state_never_claims_applied_without_hardware`, `threshold_is_a_code_not_a_percentage`; allowlist read+write |
| 86 | `wmi_ec_set_brightness_data` removido (escrevia o limite de carga) | `commands/hardware.rs:759`, `elevated.rs:1569` | grep = 0 chamadas | OK | só comentários a explicar a remoção |
| 87 | F-05 (aplicado nesta revisão): sweep de ficheiros stale não aborta o comando | `elev_bridge.rs:986-1005/1830` | `cargo test --lib elev_bridge` (13) | OK | `a_failed_stale_file_sweep_is_a_warning_not_an_abort` |

## 3. Regressões encontradas

**Nenhuma regressão de um fix.** O que foi verificado, além da leitura caso a caso:

1. **Sobrevivência linha-a-linha:** para os 87 commits não-merge da campanha (82 com linhas de código mensuráveis), extraí as linhas adicionadas (≥30 chars, não-comentário) e confirmei a presença em HEAD. Os valores <90% são todos explicados: commits de `rustfmt`/`prettier`/docs, ou linhas substituídas por um fix posterior (ex.: `LOCAL_AC_PROBE_DISABLED` → `ProbeGate`; `MAX_OVERRIDE_OFFSET_PP` → `MAX_USER/CAPTURED_OFFSET_PP`; handles raw → `OwnedHandle`). Nenhuma desapareceu sem substituto.
2. **Reverts:** `git log --all --grep=revert` → 1 commit antigo, não relacionado (`785bbdd`, S5).
3. **Testes apagados:** só 3 `it()` removidos, todos em `BatteryTab.test.tsx`, substituídos por 4 testes mais fortes (incluindo o bug do dono 94%/80%). Nenhum `it.skip`/`describe.skip` novo.
4. **Ficheiros apagados:** 1 (`hw/driver_update.rs`, portal morto, remoção intencional em `fca428e`).
5. **Marcadores de merge:** 0 (`<<<<<<<`/`>>>>>>>`).
6. **Branches:** `git branch --no-merged HEAD` → só dependabot, os dois ramos irmãos `f5/*` e um `feature/*` antigo. Todos os `f2/*`, `f3/*`, `f4*/*`, `f4f/*` estão merged.
7. **Contratos Tauri:** 230 comandos `#[tauri::command]` → todos em `generate_handler!`; 198 `invoke('…')` do frontend → todos registados; 36 `run_elevated("…")` → todos com arm no dispatcher.
8. **Código morto introduzido pelos fixes:** 0 `TODO/FIXME/HACK/XXX` novos; 244 `unwrap/expect` novos, todos em `#[cfg(test)]` ou pré-existentes (verificado por ficheiro).

## 4. Dívida e riscos (não são regressões; ficam reportados)

| # | Item | Impacto | Nota |
|---|---|---|---|
| D1 | `scenario_rules::apply_lid_action()` continua **sem chamadores** (`hw/scenario_rules.rs:493`) | A opção "ação da tampa" da UI não faz nada | Agora que existe listener de tampa, ligar é possível — mas muda comportamento do sistema (sleep), não é fix de baixo risco |
| D2 | `performance::set_performance_mode` continua a escrever WMAA 5..8 (no-op: SMMT/SMMD) e devolve `success` no fallback registry+overlay | Ruído no EC e resultado otimista no separador Performance | O fix do perf-mode resolveu a honestidade via `EcModeReadout` + `FanModeResult.warning`; a recomendação #1 do PERF-MODE-DEBUG ficou por aplicar |
| D3 | `ChargingThreshold.tsx` (e o seu teste) é um **componente órfão** — a tab usa `ChargingProtection`; `SystemInfoCard.tsx` ficou órfão com o rewrite do overview (`b034787`); `SetupTab` já estava órfão na base | Caminho de UI duplicado, morto | `ChargingThreshold` e `SetupTab` já estavam órfãos em `9f83ee4`; `SystemInfoCard` foi substituído pelos cartões do overview-plus e ficou para trás |
| D4 | `elevated.rs:535` tem um arm `set_hdr` sem chamadores | Código morto | `set_hdr` corre in-process (CCD API), não precisa de elevação |
| D5 | `TrayThemeRegression.test.ts` faz `return` silencioso se `dist/assets` não existir | O teste de especificidade da tray pode passar sem correr | No health gate o build corre antes; em `vitest` isolado não. `themeTokens.test.ts` lê o CSS fonte e cobre o ciclo |
| D6 | `chargePhase` com `limitApplied:null` (estado ainda não lido) devolve `holding` → "Paused at …" | Honestidade transitória: até o helper responder (timeout 45 s), o cartão pode dizer "em pausa" sem prova | O caso `false` (bug do dono) está corrigido e testado; `null` é o único buraco |
| D7 | `micontrol_bridge.rs:1081` usa `.expect("worker rx mutex poisoned")` | Se um worker entrar em panic com o lock, todos os workers morrem | Pré-existente (commit `23c9141`, set-2026), não introduzido pela campanha |
| D8 | `get_fan_info` (poll 2 s) + `get_ec_active_mode` (cache 3 s) fazem leituras elevadas periódicas | ~20 chamadas/min ao bridge enquanto a UI está aberta | Aceitável (38 ms medidos), mas é carga nova do fix de perf-mode |
| D9 | `set_battery_care`/`get_charge_limit_state` passam sempre pelo helper elevado (timeout 45 s) | Se o bridge estiver degradado, a tab Battery demora a obter a verdade | Por desenho (a query WMI é negada a não-admin) |
| D10 | Cópia do repo `audit-20261107/STAB-ROUND-1.md` estava 23 linhas atrás do original (sem a adenda pós-fix) | Documentação da campanha inconsistente | **Corrigido nesta revisão** (sync com `C:/Projects/miPC/audit-20261107/`) |
| D11 | Os restantes relatórios (AUDIT, POST-FIX, FUNC-FIX, WAVE4, PERF/LID/BATTERY) só existem fora do repo | Um clone limpo não tem a história da campanha | Não copiei 120 KB de docs sem pedido; fica registado |
| D12 | `logging_e2e`/`elevated_bridge`/`pipe_dacl_fix` não correm no gate normal | Cobertura de integração limitada | `logging_e2e` correu nesta revisão (1 passed); os outros tocam no serviço/DACL reais e não foram corridos de propósito |

## 5. Correções aplicadas nesta revisão

1. **F-05 (P1 do STAB-ROUND-1) — o sweep de ficheiros stale já não aborta o comando elevado.**
   - `src-tauri/src/elev_bridge.rs`: `run_blocking_timeout(10 s, cleanup_stale_elev_files)` deixou de usar `?`; um timeout/falha passa a `log::warn!` e o comando é escrito e despachado na mesma.
   - Novo `sweep_warning(Result<(), String>) -> Option<String>` (devolve `Option`, não `Result`, para tornar impossível reintroduzir `?` sem mudar o tipo).
   - Teste: `elev_bridge::tests::a_failed_stale_file_sweep_is_a_warning_not_an_abort`.
   - Prova: `cargo test --lib elev_bridge` → 13 passed; `cargo fmt --check` e `cargo clippy --lib -D warnings` limpos.
   - Porquê: era o único P1 do STAB-ROUND-1 que ficou por aplicar; em produção falhou `wmi_ec_read_adapter_power` após 30,8 s porque o cleanup gastou os 10 s do pool.

2. **Sync de documentação:** `audit-20261107/STAB-ROUND-1.md` passou a incluir a "ADIÇÃO PÓS-FIX (deploy 10-Nov 20:17)" que só existia no original fora do repo (D10).

Não foi alterado mais nada: os restantes achados são dívida pré-existente ou mudanças de comportamento que exigem decisão do dono (D1, D2, D6).

## 6. Limitações

- Verificação **estática + testes unitários**, não validação live: não fechei a tampa, não fiz swipe no bordo, não emparelhei o relógio BLE nem observei um ciclo de carga. Os três fixes que dependem disso (lid, edge-slide, LockGate) estão provados por código e testes, com a validação de campo ainda pendente como os relatórios já diziam.
- `cargo test --lib --features face` correu a lib (927); os bins foram corridos sem `face` (não usam a feature).
- O gate completo (build NSIS, eslint, handle-leak, i18n script no pipeline) não foi corrido, por instrução; corri as peças que provam os fixes (tsc, clippy, vitest, i18n, cargo test).