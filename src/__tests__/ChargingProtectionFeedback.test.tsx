import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ComponentProps } from 'react';
import ChargingProtection from '../components/ChargingProtection';
import { t } from '../hooks/useI18n';

/**
 * Regressões do audit 2026-11-07 §4.3 #1, #2 e #5 — agora sobre o DOM do POC
 * (Card + Switch + Segmented), que substituiu os botões e o checkbox antigos:
 *  - um clique com a proteção desligada liga-a e aplica o nível (nunca morto);
 *  - o cartão nomeia o estado do hardware (96% com limite de 80% = pausa) e
 *    explica porquê;
 *  - a confirmação leva valor + hora local.
 */
function setup(overrides: Partial<ComponentProps<typeof ChargingProtection>> = {}) {
  const props = {
    threshold: 100,
    level: 96,
    care: false,
    carePending: false,
    onThresholdChange: vi.fn().mockResolvedValue(undefined),
    onEnableCare: vi.fn().mockResolvedValue(undefined),
    onDisableCare: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
  const utils = render(<ChargingProtection {...props} />);
  return { ...utils, props };
}

describe('ChargingProtection feedback', () => {
  it('turns protection on and applies the level in one click', async () => {
    const user = userEvent.setup();
    const { props } = setup({ care: false, threshold: 100, level: 50 });

    // Não há clique morto: a razão está escrita.
    expect(screen.getByText(t('battery.enableToChoose'))).toBeInTheDocument();

    await user.click(screen.getByRole('radio', { name: /^60%/ }));

    await waitFor(() => expect(props.onEnableCare).toHaveBeenCalledWith(60));
    expect(props.onThresholdChange).not.toHaveBeenCalled();
    // A confirmação nomeia o valor e a hora local.
    expect(await screen.findByText(/\d{2}:\d{2}/)).toBeInTheDocument();
  });

  it('names the paused state when the battery is above the limit', () => {
    setup({
      threshold: 80,
      level: 96,
      care: true,
      battery: { level: 96, is_charging: false, is_plugged: true },
    });

    // 96% com limite de 80% lia-se como "AC ligado · 96%" e ficava por aí.
    expect(
      screen.getByText(t('charging.state.paused', { level: 96, limit: 80 })),
    ).toBeInTheDocument();
    expect(screen.getByText(t('common.why'))).toBeInTheDocument();
    expect(screen.getByTestId('limit-derived')).toHaveTextContent(t('battery.limitAboveLevel'));
  });

  it('reports charging up to the limit when the level is below it', () => {
    setup({
      threshold: 80,
      level: 54,
      care: true,
      battery: { level: 54, is_charging: true, is_plugged: true },
    });

    expect(screen.getByTestId('limit-derived')).toHaveTextContent(
      t('battery.limitDerived', { limit: 80 }),
    );
  });

  it('desligar a proteção pede "sem limite" ao pai', async () => {
    const { props } = setup({ threshold: 80, level: 50, care: true });

    fireEvent.click(screen.getByRole('switch'));

    await waitFor(() => expect(props.onDisableCare).toHaveBeenCalled());
  });
});
