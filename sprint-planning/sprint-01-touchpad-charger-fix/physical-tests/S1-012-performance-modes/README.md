# S1-012 — Performance Modes

## Purpose

Correlates the system **TDP (Thermal Design Power)** mode with ghost-touch
frequency. Higher TDP means stronger switching currents and more EMI.

| Mode | TDP | Expected EMI |
|------|-----|---------------|
| Silence | 16W | Lowest |
| Balance | 32W | Medium |
| Smart | 60W | Highest |

If ghost touches increase with TDP, the EMI is coupled through the power rail.

## What the script does

1. Saves the original active power plan GUID.
2. Enumerates available power plans (`powercfg /list`).
3. For each mode (Silence, Balance, Smart):
   - Tries to match a Windows power plan by name and switch via `powercfg /setactive`.
   - If no match (Xiaomi modes may not be standard Windows plans), prompts you to
     switch via the **Xiaomi Fn shortcut** and press Enter.
   - Runs a **3-minute** capture window with HID monitoring.
   - Saves per-mode results.
4. **Reverts** in a `finally` block: restores the original power plan GUID.
5. Generates a summary table and CSV.

## How to run

```powershell
cd .\S1-012-performance-modes
.\run-test.ps1                 # capture without miPC
.\run-test.ps1 -WithMiPC       # also run miPC dev mode during all modes
```

## Interpreting results

| Pattern | Conclusion |
|---------|------------|
| Ghost touches increase: Silence < Balance < Smart | TDP correlates with EMI — power rail coupling confirmed. |
| No correlation | TDP is not the factor; focus on charger EMI (S1-010). |
| Bug only in Smart mode | High-current switching in Smart mode is the trigger. |

## Output

- `results/S1-012-<timestamp>/<mode-id>/hid-reports.log` — per mode.
- `results/S1-012-<timestamp>/modes-summary.csv` — comparison table.

## Revert

The `finally` block always restores the original power plan GUID via
`powercfg /setactive`. Re-run with `-ResumeFrom Cleanup` if interrupted.

## Notes

- Xiaomi's performance modes map to Windows power plans, but the exact mapping
  is opaque. If `powercfg` can't find a plan, use the **Fn shortcut** to switch.
- Keep touchpad usage consistent across modes for a fair comparison.
