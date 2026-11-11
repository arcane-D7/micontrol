import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import type { ComponentProps } from 'react';
import ChargingProtection from '../components/ChargingProtection';
import { t } from '../hooks/useI18n';
import type { ChargeHoldState } from '../types/charge';

/**
 * Feedback imediato do limite (spec §5.6.3) — o segundo RED do audit:
 *  - clicar marca em ≤100 ms (optimistic), sem esperar pelo EC (300–600 ms);
 *  - falhar a escrita reverte o segmento e diz porquê, com retry;
 *  - com o cuidado desligado, clicar num nível **liga e aplica** num só gesto
 *    (mata o clique morto de `pointer-events:none`).
 */
function setup(overrides: Partial<ComponentProps<typeof ChargingProtection>> = {}) {
  const props = {
    threshold: 80,
    level: 96,
    care: true,
    carePending: false,
    onThresholdChange: vi.fn().mockResolvedValue(undefined),
    onEnableCare: vi.fn().mockResolvedValue(undefined),
    onDisableCare: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
  const utils = render(<ChargingProtection {...props} />);
  return { ...utils, props };
}

const seg = (level: number) => screen.getByRole('radio', { name: new RegExp(`^${level}%`) });

describe('ChargingProtection — limite com feedback imediato', () => {
  it('marca o nível no instante do clique, antes de o hardware confirmar', async () => {
    // Escrita pendente de propósito: o segmento tem de marcar já.
    let confirm: () => void = () => {};
    const onThresholdChange = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          confirm = resolve;
        }),
    );
    setup({ onThresholdChange });

    expect(seg(80)).toHaveAttribute('aria-checked', 'true');
    fireEvent.click(seg(60));

    // Síncrono, sem await do invoke → é isto que se chama "≤100 ms".
    expect(seg(60)).toHaveAttribute('aria-checked', 'true');
    expect(seg(80)).toHaveAttribute('aria-checked', 'false');
    expect(onThresholdChange).toHaveBeenCalledWith(60);
    expect(screen.getByTestId('limit-derived')).toHaveTextContent(
      t('battery.applying', { limit: 60 }),
    );

    confirm();
    // Confirmado o hardware, a linha volta a descrever o limite (o pai é quem
    // passa a ser a fonte de verdade do valor). Aqui o nível é 96 > 80, por isso
    // o que se lê é o aviso de "próximo ciclo".
    await waitFor(() =>
      expect(screen.getByTestId('limit-derived')).toHaveTextContent(t('battery.limitAboveLevel')),
    );
  });

  it('reverte para o valor anterior quando a escrita no EC falha', async () => {
    const onThresholdChange = vi.fn().mockRejectedValue(new Error('EC write failed'));
    setup({ onThresholdChange });

    fireEvent.click(seg(60));
    expect(seg(60)).toHaveAttribute('aria-checked', 'true');

    await waitFor(() => expect(seg(80)).toHaveAttribute('aria-checked', 'true'));
    expect(seg(60)).toHaveAttribute('aria-checked', 'false');

    // O erro é nomeado e tem caminho de retry (nunca um toast mudo).
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(t('battery.applyFailed'));
    expect(alert).toHaveTextContent(t('battery.retry'));
  });

  it('com a proteção desligada, um clique liga-a e aplica o nível (1 gesto)', async () => {
    const { props } = setup({ care: false, threshold: 100 });

    // Não há clique morto: a razão está escrita.
    expect(screen.getByText(t('battery.enableToChoose'))).toBeInTheDocument();

    fireEvent.click(seg(80));

    await waitFor(() => expect(props.onEnableCare).toHaveBeenCalledWith(80));
    expect(props.onThresholdChange).not.toHaveBeenCalled();
  });

  it('oferece 100% explícito para "sem limite"', () => {
    setup();
    expect(seg(100)).toBeInTheDocument();
  });

  it('avisa quando o limite fica abaixo da carga atual', () => {
    setup({ threshold: 80, level: 96 });
    // 96 > 80 → "vai aplicar-se no próximo ciclo"
    expect(screen.getByTestId('limit-derived')).toHaveTextContent(t('battery.limitAboveLevel'));
  });

  it('explica o limite quando está dentro do nível atual', () => {
    setup({ threshold: 80, level: 42 });
    expect(screen.getByTestId('limit-derived')).toHaveTextContent(
      t('battery.limitDerived', { limit: 80 }),
    );
  });

  it('o switch usa role=switch e aria-checked (não um checkbox escondido)', () => {
    setup({ care: true });
    expect(screen.getByRole('switch')).toHaveAttribute('aria-checked', 'true');
  });

  it('desligar a proteção pede "sem limite" ao pai', async () => {
    const { props } = setup({ care: true });
    fireEvent.click(screen.getByRole('switch'));
    await waitFor(() => expect(props.onDisableCare).toHaveBeenCalled());
  });
});

/**
 * F5 — a fase do backend governa a linha de estado.
 *
 * O dono viu "req 80, hw 80, applied:true" com o pack a 100 %. O cartão tem de
 * dizer que está ACIMA do limite (e que este firmware não descarrega), nunca
 * "pausa aos 80 %". E quando a fase é `holding`, tem de o dizer com o nível.
 */
describe('ChargingProtection — fase real do controlo (F5)', () => {
  const hold = (phase: ChargeHoldState['phase'], over: Partial<ChargeHoldState> = {}) =>
    ({
      phase,
      level: 100,
      limit: 80,
      band_low: 78,
      band_high: 80,
      firmware_threshold: 80,
      applied: true,
      charging: false,
      plugged: true,
      can_discharge: false,
      detail: null,
      verified_ms: 1,
      ...over,
    }) satisfies ChargeHoldState;

  it('acima do limite diz que está acima — e que não há descarga', () => {
    setup({ level: 100, holdState: hold('above_limit') });
    const line = screen.getByTestId('charge-hold');
    expect(line).toHaveAttribute('data-phase', 'above_limit');
    expect(line).toHaveTextContent(t('charging.hold.aboveLimit', { level: 100, limit: 80 }));
    // A linha antiga (inferida) não se repete por cima.
    expect(screen.queryByTestId('limit-derived')).toBeNull();
  });

  it('na banda diz "a segurar nos 80%"', () => {
    setup({ level: 79, holdState: hold('holding', { level: 79, charging: false }) });
    expect(screen.getByTestId('charge-hold')).toHaveTextContent(
      t('charging.hold.holding', { level: 79, limit: 80 }),
    );
  });

  it('sem read-back do firmware não afirma nada sobre o hardware', () => {
    setup({ level: 79, holdState: hold('unknown', { firmware_threshold: null, applied: false }) });
    expect(screen.getByTestId('charge-hold')).toHaveTextContent(t('charging.hold.unknown'));
  });
});
