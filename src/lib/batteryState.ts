/**
 * Single source of truth for the battery state shown in the UI.
 *
 * Three states, not two. The old two-state form (`is_charging ? charging :
 * discharging`) printed "Discharging" while the PC was on the charger, because
 * a plugged battery whose charge rate reads 0 is neither charging nor
 * discharging — it is simply connected to AC (charge threshold reached,
 * battery care limit, or a driver that reports `ChargeRate = 0` at 100 %).
 *
 *  - `charging`     — current is flowing into the battery (`Charging` bit)
 *  - `acConnected`  — adapter connected, battery not being charged
 *  - `discharging`  — running on battery
 *
 * Both the tray popup and the battery card derive the state here so they can
 * never disagree again. Kept dependency-free so it is trivially testable.
 */
export type BatteryState = 'charging' | 'acConnected' | 'discharging';

export function batteryState(isCharging: boolean, isPlugged: boolean): BatteryState {
  if (isCharging) return 'charging';
  return isPlugged ? 'acConnected' : 'discharging';
}

/** Badge colour class (see `.badge.success|info|warning` in globals.css). */
export function batteryStateBadge(state: BatteryState): string {
  if (state === 'charging') return 'success';
  return state === 'acConnected' ? 'info' : 'warning';
}
