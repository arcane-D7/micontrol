import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import BatteryInfo, { chargePhaseMessage, healthBand } from '../components/BatteryInfo';
import { chargePhase } from '../lib/batteryState';
import { t } from '../hooks/useI18n';
import type { BatteryInfo as BatteryData } from '../types/hardware';

/**
 * Os três estados de leitura da spec §5.5 — o achado #10 do audit
 * ("carregar, falhar e vazio são o mesmo ecrã"):
 *   1. skeleton com a forma do conteúdo;
 *   2. dados presentes, nada pisca;
 *   3. leitura falhada → valores mantidos + "sem leitura desde HH:MM" + retry;
 *   + vazio → "não reporta bateria".
 */
const battery: BatteryData = {
  level: 96,
  is_charging: false,
  is_plugged: true,
  health_percent: 95,
  cycle_count: 0,
  designed_capacity_mwh: 68200,
  full_capacity_mwh: 65000,
  manufacturer: 'COSMX',
  device_name: 'BX70',
  serial_number: 'x',
  chemistry: 'LiP',
  temperature_celsius: 31.4,
  time_remaining_minutes: null,
  time_to_full_minutes: null,
  charge_rate_mw: 0,
  voltage_mv: 17280,
  ac_input_power_mw: 65000,
};

describe('BatteryInfo — estados de leitura', () => {
  it('estado 1: sem dados desenha o skeleton com a forma (não um rectângulo de 120 px)', () => {
    render(<BatteryInfo battery={null} />);
    const status = screen.getByRole('status');
    expect(status).toHaveAttribute('aria-busy', 'true');
    expect(status).toHaveTextContent(t('battery.reading'));
    // A forma: anel + 3 linhas + barra.
    expect(status.querySelectorAll('.mc-skeleton').length).toBeGreaterThanOrEqual(5);
  });

  it('estado 2: com dados mostra o herói e a fase, sem skeleton', () => {
    render(<BatteryInfo battery={battery} threshold={80} care={true} updatedAgoSec={12} />);
    expect(screen.getByText('96%')).toBeInTheDocument();
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
    expect(screen.getByText(t('battery.updatedAgo', { s: 12 }))).toBeInTheDocument();
    // 96% com limite 80 → "Em pausa aos 96% — o limite é 80%"
    expect(screen.getByTestId('charge-phase')).toHaveTextContent(
      t('battery.phase.holding', { level: 96, limit: 80 }),
    );
  });

  it('estado 3: leitura falhada mantém os valores e diz desde quando', () => {
    const onRetry = vi.fn();
    render(
      <BatteryInfo
        battery={battery}
        threshold={80}
        care={true}
        staleSince="14:32"
        onRetry={onRetry}
      />,
    );
    // Os valores NÃO desaparecem.
    expect(screen.getByText('96%')).toBeInTheDocument();
    expect(screen.getByText(t('battery.stale', { time: '14:32' }))).toBeInTheDocument();
    expect(screen.getByRole('button', { name: t('battery.retry') })).toBeInTheDocument();
  });

  it('vazio: sem bateria e com erro diz que o equipamento não reporta bateria', () => {
    render(<BatteryInfo battery={null} unavailable />);
    expect(screen.getByText(t('battery.unavailable'))).toBeInTheDocument();
    expect(screen.getByText(t('battery.unavailableHint'))).toBeInTheDocument();
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('a marca do limite só aparece quando há limite', () => {
    const { container, rerender } = render(<BatteryInfo battery={battery} threshold={80} />);
    expect(container.querySelector('.mc-bar__mark')).not.toBeNull();
    rerender(<BatteryInfo battery={battery} threshold={100} />);
    expect(container.querySelector('.mc-bar__mark')).toBeNull();
  });

  it('proteção desligada: a fase diz que carrega até 100%, não "até aos 80%"', () => {
    render(<BatteryInfo battery={{ ...battery, is_charging: true }} threshold={80} care={false} />);
    expect(screen.getByTestId('charge-phase')).toHaveTextContent(t('battery.phase.careOff'));
  });
});

describe('chargePhaseMessage', () => {
  const msg = (phase: Parameters<typeof chargePhaseMessage>[0], care: boolean | null = true) =>
    chargePhaseMessage(phase, { care, time: '5 h 12 min' });

  it('cada fase tem copy própria (nenhuma cai no genérico)', () => {
    expect(msg({ kind: 'charging', target: 80 }).key).toBe('battery.phase.charging');
    expect(msg({ kind: 'holding', level: 96, limit: 80 }).key).toBe('battery.phase.holding');
    expect(msg({ kind: 'resuming', limit: 80 }).key).toBe('battery.phase.resuming');
    expect(msg({ kind: 'full' }).key).toBe('battery.phase.full');
    expect(msg({ kind: 'discharging', level: 40, minutes: 312 }).key).toBe(
      'battery.phase.discharging',
    );
  });

  it('a pausa é o único estado com tom de aviso', () => {
    expect(msg({ kind: 'holding', level: 96, limit: 80 }).tone).toBe('warn');
    expect(msg({ kind: 'charging', target: 80 }).tone).toBe('neutral');
    expect(msg({ kind: 'full' }).tone).toBe('success');
  });

  it('sem limite distingue "proteção desligada" de "sem limite definido"', () => {
    expect(msg({ kind: 'charging', target: 100 }, false).key).toBe('battery.phase.careOff');
    expect(msg({ kind: 'charging', target: 100 }, true).key).toBe('battery.phase.noLimit');
  });

  it('descarregar sem estimativa não inventa um tempo', () => {
    expect(
      chargePhaseMessage(
        { kind: 'discharging', level: 40, minutes: null },
        { care: true, time: '' },
      ).key,
    ).toBe('battery.phase.dischargingNoTime');
  });
});

describe('healthBand', () => {
  it('dá escala ao número (≥80 boa, 60–79 razoável, <60 fraca)', () => {
    expect(healthBand(95).key).toBe('battery.healthBand.good');
    expect(healthBand(80).key).toBe('battery.healthBand.good');
    expect(healthBand(79).key).toBe('battery.healthBand.fair');
    expect(healthBand(60).key).toBe('battery.healthBand.fair');
    expect(healthBand(59).key).toBe('battery.healthBand.poor');
  });
});

describe('chargePhase + copy (integração)', () => {
  it('96% ligado à corrente com limite 80 lê-se como pausa, não como "AC ligado"', () => {
    const phase = chargePhase({
      level: 96,
      isCharging: false,
      isPlugged: true,
      limit: 80,
    });
    expect(chargePhaseMessage(phase, { care: true, time: '' }).key).toBe('battery.phase.holding');
  });
});
