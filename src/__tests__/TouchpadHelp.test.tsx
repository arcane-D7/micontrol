import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { InfoHint } from '../components/ui/info-hint';
import { Accordion } from '../components/ui/accordion';
import EdgeSlideGuide from '../components/EdgeSlideGuide';
import TouchpadSettings from '../components/TouchpadSettings';
import type { TouchpadInfo } from '../types/hardware';

/**
 * O InfoHint é o par acessível do Tooltip: abre por clique **e** por foco de
 * teclado (nunca só hover), descreve o gatilho por `aria-describedby` e fecha
 * com Escape. Estes testes são o contrato — se um falhar, o "i" voltou a ser
 * uma decoração que só o rato alcança.
 *
 * Os textos vêm do locale por omissão (`en`): os testes não fixam traduções,
 * fixam comportamento.
 */
describe('InfoHint', () => {
  it('abre por clique e liga o painel ao gatilho por aria-describedby', async () => {
    render(<InfoHint label="The limit lives in the embedded controller" />);
    const button = screen.getByRole('button', {
      name: 'The limit lives in the embedded controller',
    });
    const panel = document.querySelector('.mc-infohint__panel') as HTMLElement;

    expect(button).toHaveAttribute('aria-expanded', 'false');
    expect(panel).toHaveAttribute('data-open', 'false');

    await userEvent.click(button);

    expect(button).toHaveAttribute('aria-expanded', 'true');
    expect(panel).toHaveAttribute('data-open', 'true');
    expect(button).toHaveAttribute('aria-describedby', panel.id);
    expect(panel).toHaveTextContent('The limit lives in the embedded controller');
  });

  it('abre no foco de teclado — o rato não é o único caminho', () => {
    render(<InfoHint label="Help" />);
    const button = screen.getByRole('button', { name: 'Help' });
    fireEvent.focus(button);
    expect(document.querySelector('.mc-infohint__panel')).toHaveAttribute('data-open', 'true');
  });

  it('fecha com Escape sem deixar o painel descrito', () => {
    render(<InfoHint label="Help" />);
    const button = screen.getByRole('button', { name: 'Help' });
    fireEvent.focus(button);
    fireEvent.keyDown(button, { key: 'Escape' });
    expect(document.querySelector('.mc-infohint__panel')).toHaveAttribute('data-open', 'false');
    expect(button).not.toHaveAttribute('aria-describedby');
  });

  it('um clique depois do foco fecha (focus + click não se somam)', async () => {
    render(<InfoHint label="Help" />);
    const button = screen.getByRole('button', { name: 'Help' });
    await userEvent.click(button);
    expect(document.querySelector('.mc-infohint__panel')).toHaveAttribute('data-open', 'true');
    await userEvent.click(button);
    expect(document.querySelector('.mc-infohint__panel')).toHaveAttribute('data-open', 'false');
  });

  it('um `buttonLabel` próprio nomeia o gatilho quando o texto é longo', () => {
    render(<InfoHint label="Long explanation" buttonLabel="More about sensitivity" />);
    expect(screen.getByRole('button', { name: 'More about sensitivity' })).toBeInTheDocument();
  });
});

describe('Accordion', () => {
  it('usa details/summary nativo e começa fechado', () => {
    render(
      <Accordion summary="How it works">
        <p>Body</p>
      </Accordion>,
    );
    const details = document.querySelector('details.mc-accordion') as HTMLDetailsElement;
    expect(details).toBeTruthy();
    expect(details.open).toBe(false);
    expect(screen.getByText('How it works').closest('summary')).toBeTruthy();
  });

  it('defaultOpen abre à partida', () => {
    render(
      <Accordion summary="Open" defaultOpen>
        <p>Body</p>
      </Accordion>,
    );
    expect((document.querySelector('details.mc-accordion') as HTMLDetailsElement).open).toBe(true);
  });
});

describe('EdgeSlideGuide', () => {
  it('explica a banda de 16 %, o pré-rolo de 8 mm e as chaves de registo', () => {
    render(<EdgeSlideGuide />);
    // O corpo de um <details> fechado continua no DOM (escondido por CSS), por
    // isso as asserções não precisam de abrir o acordeão.
    expect(screen.getAllByText(/16 %/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/8 mm/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/ZoneStartPct/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/ZoneContinuePct/).length).toBeGreaterThan(0);
  });

  it('diz o que não dispara e como diagnosticar', () => {
    render(<EdgeSlideGuide />);
    expect(screen.getByText(/Two or more fingers/)).toBeInTheDocument();
    expect(screen.getByText(/RUST_LOG=hw::touchpad=debug/)).toBeInTheDocument();
    expect(screen.getByText(/pointer pinned/)).toBeInTheDocument();
    expect(screen.getByText(/edge-slide captured/)).toBeInTheDocument();
  });

  it('numera os passos com uma lista ordenada', () => {
    render(<EdgeSlideGuide />);
    expect(document.querySelectorAll('.mc-accordion__body ol li').length).toBeGreaterThanOrEqual(5);
  });

  it('o diagrama tem nome acessível e duas bandas destacadas', () => {
    render(<EdgeSlideGuide />);
    expect(screen.getByRole('img', { name: /Touchpad diagram/ })).toBeInTheDocument();
    expect(document.querySelectorAll('.mc-touchdiagram__band')).toHaveLength(2);
  });
});

describe('TouchpadSettings', () => {
  const touchpad: TouchpadInfo = {
    sensitivity: 'medium',
    haptics_enabled: true,
    haptics_intensity: 'medium',
    gesture_screenshot: false,
    trackpad_repress: false,
    edge_slide: true,
  };

  const noop = vi.fn(async () => {});

  const renderSettings = () =>
    render(
      <TouchpadSettings
        touchpad={touchpad}
        onSensitivityChange={noop}
        onHapticsChange={noop}
        onHapticsIntensityChange={noop}
        onGestureScreenshotChange={noop}
        onRepressChange={noop}
        onEdgeSlideChange={noop}
      />,
    );

  it('dá a cada controlo do trackpad uma descrição não-óbvia', () => {
    renderSettings();
    // 6 controlos: sensibilidade, háptica, intensidade, gesto, repress, edge slide.
    expect(document.querySelectorAll('.mc-infohint')).toHaveLength(6);
    expect(screen.getByRole('button', { name: 'More about Sensitivity' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'More about Edge Slide' })).toBeInTheDocument();
  });

  it('o acordeão do Edge Slide vive dentro do cartão, depois do toggle', () => {
    renderSettings();
    expect(screen.getByText('Where to touch, and how')).toBeInTheDocument();
    expect(screen.getByRole('img', { name: /Touchpad diagram/ })).toBeInTheDocument();
  });

  it('sem háptica ligada, a intensidade desaparece com o seu hint', () => {
    render(
      <TouchpadSettings
        touchpad={{ ...touchpad, haptics_enabled: false }}
        onSensitivityChange={noop}
        onHapticsChange={noop}
        onHapticsIntensityChange={noop}
        onGestureScreenshotChange={noop}
        onRepressChange={noop}
        onEdgeSlideChange={noop}
      />,
    );
    expect(document.querySelectorAll('.mc-infohint')).toHaveLength(5);
  });
});
