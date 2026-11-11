/**
 * Battery Care (EC registo 0xA4) — leitura + escrita isoladas.
 *
 * O estado subiu para o pai da tab porque a linha de fase do cartão de estado
 * precisa de o conhecer: com a proteção desligada o limite não governa nada e
 * a fase tem de dizer "carrega até 100%", não "a carregar até aos 80%".
 *
 * Não toca no contrato do `useHardware`: usa os mesmos dois comandos que o
 * `ChargingProtection` já usava (`get_battery_care` / `set_battery_care`).
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getUserFriendlyMessage, parseErrorResponse, type TranslateFn } from '../types/error';
import type { ChargeHoldState, ChargeLimitState } from '../types/charge';
import { t } from './useI18n';

const translate: TranslateFn = (key) => t(key as never);

export interface BatteryCareState {
  /** null enquanto a leitura inicial não chegou (ou falhou). */
  care: boolean | null;
  /**
   * Verdade do firmware (pedido vs aplicado + motivo). `null` = ainda não lido.
   * É isto que impede o cartão de chamar "pausa" a um limite que nunca chegou
   * ao controlador (94% com limite de 80%).
   */
  limitState: ChargeLimitState | null;
  /**
   * Fase real do controlo de carga ("holding at 80", "above limit", …).
   * Barata: o backend lê o snapshot da bateria e o último read-back do
   * firmware, sem WMI nem elevação. `null` = ainda não lido.
   */
  holdState: ChargeHoldState | null;
  pending: boolean;
  error: string | null;
  setCare: (enabled: boolean) => Promise<void>;
  reload: () => Promise<void>;
}

export function useBatteryCare(): BatteryCareState {
  const [care, setCareState] = useState<boolean | null>(null);
  const [limitState, setLimitState] = useState<ChargeLimitState | null>(null);
  const [holdState, setHoldState] = useState<ChargeHoldState | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      const value = await invoke<boolean>('get_battery_care');
      setCareState(Boolean(value));
      setError(null);
    } catch (e) {
      setCareState(null);
      setError(getUserFriendlyMessage(parseErrorResponse(e), translate));
    }
    try {
      // Não fatal: sem este estado o cartão simplesmente não afirma "pausa".
      setLimitState(await invoke<ChargeLimitState>('get_charge_limit_state'));
    } catch {
      setLimitState(null);
    }
    try {
      setHoldState(await invoke<ChargeHoldState>('get_charge_hold_state'));
    } catch {
      setHoldState(null);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const setCare = useCallback(async (enabled: boolean) => {
    setPending(true);
    try {
      await invoke('set_battery_care', { enabled });
      setCareState(enabled);
      setError(null);
      // O write devolve a leitura de confirmação — reaproveita-a.
      try {
        setLimitState(await invoke<ChargeLimitState>('get_charge_limit_state'));
      } catch {
        /* mantém o estado anterior */
      }
    } catch (e) {
      const message = getUserFriendlyMessage(parseErrorResponse(e), translate);
      setError(message);
      // Rejeita já com a mensagem amigável: o chamador volta a passá-la por
      // `parseErrorResponse`, que aceita strings sem as transformar em genérico.
      throw message;
    } finally {
      setPending(false);
    }
  }, []);

  return { care, limitState, holdState, pending, error, setCare, reload };
}
