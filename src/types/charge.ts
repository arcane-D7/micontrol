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
  /** `firmware` quando a leitura do hardware funcionou; `registry` quando não. */
  source: 'firmware' | 'registry' | string;
  detail: string | null;
}
