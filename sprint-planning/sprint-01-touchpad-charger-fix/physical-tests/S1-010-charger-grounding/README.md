# S1-010 — Charger/Grounding Matrix

## Purpose

Identifies which **physical** charger/outlet/grounding conditions trigger the
ghost-touch bug. Since the root cause is analog EMI coupling, the physical
environment is decisive. This test iterates through a matrix of conditions and
captures HID reports for each.

## The matrix

| # | Condition | What it tests |
|---|-----------|---------------|
| 1 | Original Xiaomi charger | Baseline — does the bug reproduce with the stock charger? |
| 2 | Third-party USB-C charger | Different switching ripple spectrum. |
| 3 | 3-prong grounded charger | Earth ground shunts EMI away from chassis. |
| 4 | Different wall outlet | Rules out a faulty/noisy outlet. |
| 5 | Ferrite core on DC cable | Common-mode choke attenuates high-freq EMI. |
| 6 | Battery < 20% | Higher charging current → stronger EMI. |
| 7 | Battery > 90% | Lower charging current → weaker EMI. |

## What the script does

1. Takes a snapshot.
2. For each condition:
   - Prompts you to set up the physical condition (swap charger, move outlet, etc.).
   - Runs a **2-minute** capture window (shorter since there are many conditions).
   - Saves HID reports and a per-condition snapshot.
3. Generates a summary table and CSV at the end.

No configuration is changed — only physical conditions. Nothing to revert.

## How to run

```powershell
cd .\S1-010-charger-grounding
.\run-test.ps1                 # capture without miPC
.\run-test.ps1 -WithMiPC       # also run miPC dev mode during all conditions
```

## Interpreting results

Compare the report counts and byte sizes across conditions:

| Pattern | Conclusion |
|---------|------------|
| Bug only with original Xiaomi charger | Charger-specific EMI spectrum. |
| Bug vanishes with grounded charger | Earth ground is the mitigation. |
| Bug worse at low battery | High charging current amplifies EMI. |
| Bug vanishes with ferrite core | Common-mode EMI on the DC cable. |

## Output

- `results/S1-010-<timestamp>/<condition-id>/hid-reports.log` — per condition.
- `results/S1-010-<timestamp>/matrix-summary.csv` — comparison table.

## Notes

- You must **physically change** the charger/outlet between conditions.
- For battery-level conditions, you may need to charge/discharge first.
- Keep the touchpad usage pattern as consistent as possible across conditions.
