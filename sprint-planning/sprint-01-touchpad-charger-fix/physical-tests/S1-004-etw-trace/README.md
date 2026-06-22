# S1-004 — ETW Trace

## Purpose

Captures kernel-level **Event Tracing for Windows (ETW)** events from the I2C,
ACPI, and HID subsystems during a 5-minute capture window. This distinguishes
between two failure modes:

| Failure mode | ETW signature |
|--------------|---------------|
| **I2C bus errors** (digital) | NACKs, transfer timeouts, CRC errors in `hidi2c` events |
| **Analog corruption** (EMI) | Clean I2C transfers but corrupted HID report bytes |

## What ETW reveals

- **hidi2c (`Microsoft-Windows-Hidi2c`)** — I2C transfer errors, NACKs, timeouts.
- **HidClass (`Microsoft-Windows-HidClass`)** — Input report delivery, gaps, bursts.
- **Kernel-PnP / ACPI** — Power transitions (D0↔D3hot) around ghost-touch moments.

If I2C errors appear at the exact ghost-touch timestamps, the problem is digital
(signal integrity / D3hot). If I2C transfers are clean but the pointer jumps,
the corruption happens *before* the ADC — confirming analog EMI.

## What the script does

1. Takes a snapshot.
2. Starts an ETW session via `logman` (or `wpr.exe` fallback) with the three providers.
3. Also starts the HID monitor for cross-reference.
4. Runs a 5-min capture window — **you must actively reproduce ghost touches**.
5. Stops the ETW session and saves the `.etl` file.
6. Writes analysis instructions to `ANALYSIS-INSTRUCTIONS.txt`.

No configuration is changed — ETW is read-only, so there is nothing to revert.

## How to run

```powershell
cd .\S1-004-etw-trace
.\run-test.ps1                 # capture without miPC
.\run-test.ps1 -WithMiPC       # also run miPC dev mode during capture
```

## Analyzing the trace

1. Install the **Windows Performance Toolkit (WPT)** via the Windows ADK.
2. Open the `.etl` file in **Windows Performance Analyzer (WPA)**.
3. Add the *System Activity* and *Device I/O* graphs.
4. Filter for `hidi2c`, `HidClass`, and ACPI power events.
5. Correlate ghost-touch moments (from the HID report log) with ETW events.

## Output

- `results/S1-004-<timestamp>/MiPC_EMI_Trace*.etl` — the ETW trace.
- `results/S1-004-<timestamp>/hid-reports.log` — HID reports for cross-reference.
- `results/S1-004-<timestamp>/ANALYSIS-INSTRUCTIONS.txt` — analysis guide.
