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

/* ─────────────────────────────────────────────────────────────────────────────
   ChargePhase — o modelo que faltava.

   `BatteryState` responde "está ligado à corrente?"; não responde à pergunta
   que o utilizador faz de facto: *porque é que não chega aos 100%?*. Uma bateria
   a 96% com limite de 80% está `acConnected` para o estado antigo — indistinta
   de uma bateria a 100% parada. A fase separa as duas.

   Derivação pura (spec §5.6.1), sem I/O, testável em Vitest.
   ──────────────────────────────────────────────────────────────────────────── */

export type ChargePhase =
  | { kind: 'discharging'; level: number; minutes: number | null }
  | { kind: 'charging'; target: number }
  | { kind: 'holding'; level: number; limit: number }
  | { kind: 'limitNotApplied'; level: number; limit: number }
  | { kind: 'resuming'; limit: number }
  | { kind: 'full' };

export interface ChargePhaseInput {
  level: number;
  isCharging: boolean;
  isPlugged: boolean;
  /** Limite de carga em percentagem; 100 (ou mais) significa "sem limite". */
  limit: number;
  /**
   * O firmware confirmou o limite? `false` = pedido ≠ hardware (ou não foi
   * possível relê-lo). Sem esta prova a fase não pode dizer "em pausa ao
   * limite": 94% com limite de 80% que o EC nunca recebeu **não** é pausa.
   * `undefined`/`null` = desconhecido (chamadores sem estado do firmware).
   */
  limitApplied?: boolean | null;
  /** Minutos de autonomia restantes — só usado a descarregar. */
  minutesRemaining?: number | null;
}

export function chargePhase({
  level,
  isCharging,
  isPlugged,
  limit,
  limitApplied = null,
  minutesRemaining = null,
}: ChargePhaseInput): ChargePhase {
  if (!isPlugged) return { kind: 'discharging', level, minutes: minutesRemaining ?? null };

  const capped = limit < 100 ? limit : 100;
  if (isCharging) return { kind: 'charging', target: capped };

  // Desvio consciente ao pseudo-código da spec (`level >= limit-1`): com
  // `level` inteiro, `level < limit` implica `level <= limit-1`, logo o ramo
  // `resuming` seria inalcançável. Usa-se `level >= limit` — pausa = está no
  // limite ou acima; retoma = está abaixo — e ambos os estados ficam vivos.
  if (capped < 100 && level >= capped) {
    // Só é "pausa ao limite" quando o firmware confirmou o limite. Com
    // `applied:false` isto é uma bateria acima de um limite que não existe no
    // hardware — dizer "em pausa" foi o bug de 94% vs 80%.
    if (limitApplied === false) return { kind: 'limitNotApplied', level, limit: capped };
    return { kind: 'holding', level, limit: capped };
  }
  if (capped < 100 && level < capped) return { kind: 'resuming', limit: capped };
  if (level >= 99) return { kind: 'full' };
  return { kind: 'charging', target: 100 };
}
