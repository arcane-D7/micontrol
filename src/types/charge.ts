/**
 * Espelho de `hw::charging::ChargeLimitState` (Rust).
 *
 * `requested_threshold` é o que o utilizador pediu (registry);
 * `hardware_threshold` é o que o firmware respondeu no read-back. Só com os
 * dois é possível dizer a verdade sobre o limite.
 */
export interface ChargeLimitState {
  requested_threshold: number;
  hardware_threshold: number | null;
  hardware_code: number | null;
  applied: boolean;
  /**
   * `firmware` = leitura ao vivo; `cache` = última confirmação dentro do TTL
   * (`last_verified_ms` diz a idade); `registry` = nada lido.
   */
  source: 'firmware' | 'cache' | 'registry' | string;
  detail: string | null;
  /** Quando o firmware foi confirmado pela última vez (epoch ms); null = nunca. */
  last_verified_ms: number | null;
}

/**
 * Fase do controlo de carga — espelho de `hw::charging::HoldPhase`.
 *
 * O limite é um **tecto**, não um setpoint: o EC pára a carga no limite e
 * deixa o pack aí. Este firmware não tem canal de descarga, por isso um pack
 * acima do limite fica acima do limite (`above_limit`) até ser usado.
 */
export type HoldPhase =
  'off' | 'holding' | 'above_limit' | 'charging' | 'idle' | 'on_battery' | 'drifted' | 'unknown';

/** Espelho de `hw::charging::ChargeHoldState` (Rust). */
export interface ChargeHoldState {
  phase: HoldPhase;
  level: number | null;
  limit: number;
  /** Limite inferior da banda considerada "segura" (`limit - 2`). */
  band_low: number;
  band_high: number;
  firmware_threshold: number | null;
  applied: boolean;
  charging: boolean | null;
  plugged: boolean | null;
  /** Sempre `false` neste firmware: não existe canal de descarga. */
  can_discharge: boolean;
  detail: string | null;
  verified_ms: number | null;
}
