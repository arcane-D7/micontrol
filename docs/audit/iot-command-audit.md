# IoT command audit — orphaned `#[tauri::command]`s

Scope: every IoT command registered in `src-tauri/src/lib.rs`. "Callers" means a
reference in `src/` (the only code that can `invoke` a Tauri command), verified
with a whole-tree scan of `src/`, `src-tauri/src/`, `e2e/`, `qa/` and `scripts/`.

## Root cause of the wrapper failures (context for the removals)

`get_iot_model` / `get_iot_fw_version` / `get_iot_bind_status` /
`get_iot_device_id` were not a different code path from `get_iot_device_info`:
both end in `iotservice::query_ec_*` → `ecram::send_pipe_request`. The
difference was error handling.

- `send_pipe_request` treated a transient `CreateFileW` failure
  (`ERROR_PIPE_BUSY` 0x800700E7 while every instance was connected, or
  `ERROR_FILE_NOT_FOUND` 0x80070002 in the accept-loop window where the service
  had closed the previous instance and not yet created the next one) as fatal.
  The bridge pipe has had a bounded retry for this since S32-005c / S55 FIX 22;
  the EC pipe never did.
- `get_iot_device_info` swallows every one of those errors with `.ok()` and
  falls back to the cached model (`discovery::global_profile`) and to the
  registry (`read_fw_version_from_registry`, `read_bind_status_from_registry`).
  It therefore _reports_ success while the pipe query failed — the production
  log proves it: `[iot] EC command get_fw_version failed: Open pipe: All pipe
instances are busy` inside one aggregate run whose other queries succeeded.
- The wrappers propagated the error → 0/4 live, while the aggregate looked 2/2.
  External probes never lost the race because process start-up spaces their
  opens milliseconds apart.

Fix (this branch): `ecram::open_ecram_pipe_with_retry` retries exactly those two
transient codes (20 × 50 ms) and `is_pipe_broker_available` uses the same helper
instead of reporting a busy service as absent. Regression test:
`hw::ecram::tests::pipe_mode_tests::pipe_open_retries_while_all_instances_are_busy`.

## Commands removed (14) — deprecated duplicates, zero callers

| Command                    | Replacement (wired in the UI)         | Callers |
| -------------------------- | ------------------------------------- | ------- |
| `get_iot_model`            | `get_iot_device_info().model`         | 0       |
| `get_iot_fw_version`       | `get_iot_device_info().fw_version`    | 0       |
| `get_iot_bind_status`      | `get_iot_device_info().bind_status`   | 0       |
| `get_iot_device_id`        | `get_iot_device_info().device_id`     | 0       |
| `get_iot_device_status`    | `get_iot_device_info().device_status` | 0       |
| `get_iot_wifi_status`      | `get_iot_wifi_list().status`          | 0       |
| `get_iot_wifi_count`       | `get_iot_wifi_list().count`           | 0       |
| `get_iot_wifi_by_index`    | `get_iot_wifi_list().networks`        | 0       |
| `send_iot_laptop_status`   | `iot_notify_event(LaptopStatus)`      | 0       |
| `iot_report_windows_ready` | `iot_notify_event(LaptopStatus)`      | 0       |
| `iot_report_suspending`    | `iot_notify_event(LaptopStatus)`      | 0       |
| `iot_report_shutting_down` | `iot_notify_event(LaptopStatus)`      | 0       |
| `iot_notify_power_event`   | `iot_notify_event(Power)`             | 0       |
| `iot_notify_ec_event`      | `iot_notify_event(Ec)`                | 0       |

All fourteen carried `#[deprecated]` pointing at the consolidated command. With
zero callers there is nothing to migrate, so removal is safer than keeping an
API whose only behaviour is a hard failure.

## Commands kept (6) — no replacement exists, still uncalled

| Command                 | Decision                                                                                                                                     |
| ----------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `iot_connect_wifi`      | Keep. Only implementation of "connect to provisioned SSID"; WiFi provisioning is not wired in the UI yet (the chip is unbound on this host). |
| `iot_write_wifi_item`   | Keep. Only implementation of provisioning-list writes. Destructive → stays command-only until a UI flow exists.                              |
| `iot_delete_wifi_item`  | Keep, same reason.                                                                                                                           |
| `iot_empty_wifi_items`  | Keep, same reason.                                                                                                                           |
| `iot_set_device_status` | Keep. Only implementation; the ecram_service accepts it for protocol parity.                                                                 |
| `iot_reset_device`      | Keep. Destructive device reset — deliberately not wired to the UI.                                                                           |

They are **not** deprecated because there is no successor command; marking them
deprecated would imply a replacement that does not exist.

## Diagnostic command kept (1)

| Command              | Decision                                                                                                      |
| -------------------- | ------------------------------------------------------------------------------------------------------------- |
| `iot_pipe_available` | Keep. Zero UI callers, but it is the cheapest liveness probe for support/diagnostics and costs one pipe open. |

## Wired and in use (for completeness)

`get_iot_device_info` (`IotDeviceCard.tsx`, `useHardware.ts`),
`get_iot_wifi_list` (`useHardware.ts`), `iot_notify_event` (`useHardware.ts`),
`ensure_iot_service` (`IotDeviceCard.tsx`).
