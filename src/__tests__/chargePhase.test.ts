import { describe, it, expect } from 'vitest';
import { chargePhase, type ChargePhase } from '../lib/batteryState';

/**
 * Transições da máquina de estados de carga (spec §5.6.1).
 * Estes testes fixam as cinco fases e a sua fronteira — é aqui que morre o
 * UX-02 ("o estado de carregamento não existe no modelo do sistema").
 */
describe('chargePhase', () => {
  it('descarregando: sem carregador, leva os minutos de autonomia', () => {
    expect(
      chargePhase({
        level: 43,
        isCharging: false,
        isPlugged: false,
        limit: 80,
        minutesRemaining: 312,
      }),
    ).toEqual<ChargePhase>({ kind: 'discharging', level: 43, minutes: 312 });
  });

  it('descarregando sem estimativa: minutes = null (nunca um número inventado)', () => {
    expect(
      chargePhase({ level: 43, isCharging: false, isPlugged: false, limit: 80 }),
    ).toEqual<ChargePhase>({ kind: 'discharging', level: 43, minutes: null });
  });

  it('a carregar com limite: target é o limite, não 100', () => {
    expect(chargePhase({ level: 61, isCharging: true, isPlugged: true, limit: 80 })).toEqual({
      kind: 'charging',
      target: 80,
    });
  });

  it('a carregar sem limite: target 100', () => {
    expect(chargePhase({ level: 61, isCharging: true, isPlugged: true, limit: 100 })).toEqual({
      kind: 'charging',
      target: 100,
    });
  });

  it('em pausa: ligado, sem corrente, no limite ou acima dele', () => {
    expect(chargePhase({ level: 80, isCharging: false, isPlugged: true, limit: 80 })).toEqual({
      kind: 'holding',
      level: 80,
      limit: 80,
    });
    // 96% com limite 80 — o caso exacto do audit (o ecrã dizia só "AC ligado")
    expect(chargePhase({ level: 96, isCharging: false, isPlugged: true, limit: 80 })).toEqual({
      kind: 'holding',
      level: 96,
      limit: 80,
    });
  });

  it('retoma: ligado, sem corrente, abaixo do limite', () => {
    expect(chargePhase({ level: 72, isCharging: false, isPlugged: true, limit: 80 })).toEqual({
      kind: 'resuming',
      limit: 80,
    });
  });

  it('cheia: ligado, sem corrente, sem limite e >= 99%', () => {
    expect(chargePhase({ level: 100, isCharging: false, isPlugged: true, limit: 100 })).toEqual({
      kind: 'full',
    });
    expect(chargePhase({ level: 99, isCharging: false, isPlugged: true, limit: 100 })).toEqual({
      kind: 'full',
    });
  });

  it('ligado, sem corrente, sem limite e abaixo de 99 → a carregar até 100', () => {
    // Adaptador ligado mas a bateria ainda não reporta corrente (driver lento,
    // logo após ligar): não é "full", não é "pausa" — é a caminho dos 100%.
    expect(chargePhase({ level: 84, isCharging: false, isPlugged: true, limit: 100 })).toEqual({
      kind: 'charging',
      target: 100,
    });
  });

  it('limite 100 é tratado como "sem limite" (nunca holding a 100)', () => {
    const phase = chargePhase({ level: 100, isCharging: false, isPlugged: true, limit: 100 });
    expect(phase.kind).toBe('full');
  });

  it('o ramo resuming mantém-se alcançável (desvio documentado a level >= limit-1)', () => {
    // Com `level >= limit-1` (pseudo-código da spec) e níveis inteiros, este
    // caso (79 com limite 80) cairia em `holding` e `resuming` nunca existiria.
    expect(chargePhase({ level: 79, isCharging: false, isPlugged: true, limit: 80 }).kind).toBe(
      'resuming',
    );
  });
});
