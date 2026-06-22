# S1-002 — HID Reports

## Purpose

Captures raw HID input reports from the BLTP7853 touchpad in two phases and
compares them:

- **Phase A:** IoTService **running** (normal state).
- **Phase B:** IoTService **stopped** (no EC RAM writes).

The comparison reveals whether IoTService's EC writes inject extra HID traffic
or alter the report stream, and surfaces vendor-defined collection traffic
(COL04/COL05).

## COL04 / COL05

The BLTP7853 touchpad exposes 5 HID collections. COL04 and COL05 use
vendor-defined Usage Pages (`0xFF00` / `0xFF01`), which carry proprietary
touch-controller diagnostics and tuning data — not standard mouse/pointer
reports. An increase in COL04/COL05 traffic during charging may indicate the
controller is reporting noise/interference events.

## What to look for

| Metric | Meaning |
|--------|---------|
| Report count difference (A − B) | Extra reports generated while IoTService is active. |
| Report ID distribution | COL04/COL05 vendor reports vs standard pointer reports. |
| Report byte patterns | Corrupted/anomalous bytes suggest analog corruption vs clean digital events. |

## What the script does

1. Takes a snapshot.
2. Phase A: starts HID monitor, 5-min capture with IoTService running, stops monitor.
3. Phase B: stops IoTService (`sc.exe stop`), waits 5s, starts HID monitor, 5-min capture, stops monitor.
4. Generates a comparison summary (report counts, file sizes).
5. **Reverts** in a `finally` block: restarts IoTService.

No reboot is required — stopping a service takes effect immediately.

## How to run

```powershell
cd .\S1-002-hid-reports
.\run-test.ps1                 # capture without miPC
.\run-test.ps1 -WithMiPC       # also run miPC dev mode during both phases
```

## Output

- `results/S1-002-<timestamp>/phaseA-svc-running-hid.log`
- `results/S1-002-<timestamp>/phaseB-svc-stopped-hid.log`
- `results/S1-002-<timestamp>/comparison-summary.txt`

## Revert

The `finally` block always restarts IoTService. No reboot needed.
