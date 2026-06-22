# S1-006 — Disable IoTService

## Purpose

**Decisive test.** IoTService.exe performs periodic EC RAM writes that shift the
charging management state, changing the power-rail ripple spectrum. If ghost
touches **vanish** with IoTService disabled, the service is confirmed as an
*amplifier* of the EMI bug (not the primary analog cause, but a significant
contributor).

## What the script does

1. Saves the original IoTService start type (usually `Automatic`).
2. Disables the service: `sc.exe config IoTSvc start= disabled`.
3. Stops it and installs a RunOnce reboot-resume hook.
4. Prompts you to reboot (required for the service to fully stop).
5. After reboot: confirms IoTService is not running, takes a snapshot, and runs a
   5-minute capture window.
6. **Reverts** in a `finally` block: restores the original start type and starts
   the service again.

## How to run

```powershell
cd .\S1-006-disable-iotservice
.\run-test.ps1                 # capture without miPC
.\run-test.ps1 -WithMiPC       # also run miPC dev mode during capture
```

## Interpreting results

| Observation | Conclusion |
|-------------|------------|
| Ghost touches **stop** with IoTService disabled | IoTService's EC writes amplify the EMI. Mitigation: throttle/harden EC writes. |
| Ghost touches **continue** with IoTService disabled | IoTService is not the amplifier; focus on the analog EMI path (S1-010, S1-007). |

## Revert

The `finally` block always restores IoTService to its original start type and
starts it. If the script is interrupted, re-run with `-ResumeFrom Cleanup`.
