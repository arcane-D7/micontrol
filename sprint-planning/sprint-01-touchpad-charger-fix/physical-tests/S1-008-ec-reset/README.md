# S1-008 — EC Reset (baseline)

## Purpose

Establishes a **clean EC (Embedded Controller) baseline** by performing a hard
EC reset (30-second power-button hold). This rules out stale EC state as a
contributor to the ghost-touch bug and gives all subsequent tests a known
starting point.

## Why this is first

A residual charge or stuck EC register can keep the charging management in an
abnormal state. Resetting the EC before any other test ensures later results
reflect the *current* hardware condition, not a leftover from prior testing.

## Manual steps (performed by the user)

The script **cannot** hold the power button for 30s — that is a physical action.
It will:

1. Capture a pre-reset system snapshot (battery, power plan, IoTService, I2C idle timer).
2. Install a RunOnce hook so it re-launches automatically after reboot.
3. Prompt you to perform the manual reset:
   - Unplug the charger.
   - Shut down Windows.
   - **Press and hold the power button for 30 seconds** (hard EC reset / static discharge).
   - Wait 10 seconds.
   - Plug the charger back in.
   - Power on.
4. After reboot + login, the script resumes, takes a post-reset snapshot, and runs
   a 5-minute capture window.

## How to run

```powershell
cd .\S1-008-ec-reset
.\run-test.ps1                 # baseline capture
.\run-test.ps1 -WithMiPC       # also run miPC dev mode during capture
```

## What it produces

- `logs/S1-008-<timestamp>.log` — full test log with pre/post snapshots.
- `results/S1-008-<timestamp>/hid-reports.log` — HID input reports during the capture window.

## Revert

None required — the EC reset **is** the reset. No configuration is changed by the
script itself.
