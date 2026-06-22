# S1-007 — Force I2C D0

## Purpose

The Intel Quick I2C Host Controller (`PCI\VEN_8086&DEV_E448`) has
`EnhancedPowerManagementEnabled=0` but `IdleTimerPeriod=10000` (10 seconds),
meaning it still transitions to **D3hot** idle when inactive. D0↔D3hot
transitions change the I2C bus impedance and can couple noise into the touchpad
analog front-end.

This test forces the controller to stay in **D0** (fully active) by setting
`IdleTimerPeriod=0`, then captures to see if ghost touches disappear.

## D0 vs D3hot

| State | Meaning | EMI impact |
|-------|---------|------------|
| **D0** | Fully active, low-impedance | EMI-resistant |
| **D3hot** | Idle, high-impedance, partial power-down | Susceptible to noise coupling |

## What the script does

1. Locates the I2C controller device ID.
2. Reads the current `IdleTimerPeriod` from
   `HKLM\SYSTEM\CurrentControlSet\Enum\<deviceId>\Device Parameters\IdleTimerPeriod`.
3. Saves the original value (or notes it didn't exist).
4. Sets `IdleTimerPeriod=0` via `reg add`.
5. Installs a RunOnce reboot-resume hook and prompts reboot.
6. After reboot: confirms the value, takes a snapshot, runs a 5-min capture window.
7. **Reverts** in a `finally` block: restores the original value (or deletes it).

## How to run

```powershell
cd .\S1-007-force-i2c-d0
.\run-test.ps1                 # capture without miPC
.\run-test.ps1 -WithMiPC       # also run miPC dev mode during capture
```

## Interpreting results

| Observation | Conclusion |
|-------------|------------|
| Ghost touches **stop** with D0 forced | D3hot idle transitions contribute to the EMI. Mitigation: keep I2C in D0. |
| Ghost touches **continue** | D3hot is not the factor; focus on charger EMI (S1-010). |

## Revert

The `finally` block always restores the original `IdleTimerPeriod`. If the value
did not exist before, it is deleted. Re-run with `-ResumeFrom Cleanup` if interrupted.
