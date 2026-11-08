import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

import Card from '../components/ui/card';
import Hairline from '../components/ui/hairline';
import StatRow, { StatGroup } from '../components/ui/stat-row';
import Badge from '../components/ui/badge';
import Button, { IconButton } from '../components/ui/button';
import Switch from '../components/ui/switch';
import Slider from '../components/ui/slider';
import Dialog from '../components/ui/dialog';
import NavItem, { NavGroup } from '../components/ui/nav-item';
import Skeleton, { SkeletonSwap } from '../components/ui/skeleton';
import Tooltip from '../components/ui/tooltip';
import Segmented from '../components/ui/segmented';
import { Icon } from '../components/ui/icons';

import ToggleRow from '../components/ToggleRow';
import ToggleSwitch from '../components/ToggleSwitch';
import InfoModal, { InfoRow, InfoSection } from '../components/InfoModal';

/**
 * Os 6 estados obrigatórios da vaga 1 (theme-migration.md §5.4):
 * default / hover / focus-visible / active / disabled / pending.
 * O hover e o focus são CSS; o que se testa aqui é o CONTRATO (atributos,
 * handlers, semântica) que a folha de estilos pinta.
 */

describe('Card', () => {
  it('renderiza título, acção, corpo e rodapé', () => {
    render(
      <Card
        title="Proteção da bateria"
        action={<IconButton icon="info" label="Ajuda" />}
        footer="rodapé"
      >
        corpo
      </Card>,
    );
    expect(screen.getByText('Proteção da bateria')).toBeInTheDocument();
    expect(screen.getByText('corpo')).toBeInTheDocument();
    expect(screen.getByText('rodapé')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Ajuda' })).toBeInTheDocument();
    expect(document.querySelector('.mc-card')).toBeTruthy();
  });

  it('sem sombra: a classe do card não pede elevação', () => {
    render(<Card title="x">y</Card>);
    const card = document.querySelector('.mc-card') as HTMLElement;
    expect(card.className).not.toMatch(/shadow|glass|elevated/);
  });

  it('selectable vira botão com aria-pressed', async () => {
    const onClick = vi.fn();
    render(
      <Card title="Modo" selectable selected onClick={onClick}>
        Silencioso
      </Card>,
    );
    const button = screen.getByRole('button', { pressed: true });
    await userEvent.click(button);
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('loading marca aria-busy', () => {
    render(<Card title="A ler" loading />);
    expect(document.querySelector('.mc-card')).toHaveAttribute('aria-busy', 'true');
  });
});

describe('Hairline', () => {
  it('é um separador semântico', () => {
    render(<Hairline strong />);
    const hr = screen.getByRole('separator');
    expect(hr).toHaveClass('mc-hairline', 'mc-hairline--strong');
  });
});

describe('StatRow', () => {
  it('mostra label, valor, unidade e dica', () => {
    render(
      <StatGroup>
        <StatRow label="Tensão" value="17,28" unit="V" mono />
        <StatRow label="Carregador" value="65" unit="W" hint="AC" />
      </StatGroup>,
    );
    expect(screen.getByText('Tensão')).toBeInTheDocument();
    expect(screen.getByText('17,28')).toHaveClass('mc-stat-row__value--mono');
    expect(screen.getByText('V')).toHaveClass('mc-stat-row__unit');
    expect(screen.getByText('AC')).toHaveClass('mc-stat-row__hint');
  });

  it('estado "sem leitura" é uma variante, não um valor inventado', () => {
    render(<StatRow label="Tensão do carregador" value="não exposto" empty />);
    expect(document.querySelector('.mc-stat-row--empty')).toBeTruthy();
  });
});

describe('Badge', () => {
  it('tone pinta superfície pastel e texto de tinta', () => {
    render(<Badge tone="success">Boa</Badge>);
    expect(screen.getByText('Boa').closest('.mc-badge')).toHaveClass('mc-badge--success');
  });

  it('o dot é decorativo e o count é tabular', () => {
    render(
      <Badge tone="warning" dot count={3}>
        Avisos
      </Badge>,
    );
    const badge = screen.getByText('Avisos').closest('.mc-badge') as HTMLElement;
    expect(badge.querySelector('.mc-badge__dot')).toHaveAttribute('aria-hidden', 'true');
    expect(badge.querySelector('.mc-badge__count')).toHaveTextContent('3');
  });
});

describe('Button', () => {
  it('primário é tinta, não accent', () => {
    render(<Button variant="primary">Aplicar</Button>);
    expect(screen.getByRole('button', { name: 'Aplicar' })).toHaveClass('mc-btn--primary');
  });

  it('pending desliga o segundo clique e anuncia aria-busy', async () => {
    const onClick = vi.fn();
    render(
      <Button variant="primary" pending onClick={onClick}>
        Aplicar
      </Button>,
    );
    const button = screen.getByRole('button', { name: 'Aplicar' });
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute('aria-busy', 'true');
    expect(button.querySelector('.mc-btn__spinner')).toBeTruthy();
    await userEvent.click(button);
    expect(onClick).not.toHaveBeenCalled();
  });

  it('disabled continua visível e nomeado', () => {
    render(<Button disabled>Sem efeito</Button>);
    expect(screen.getByRole('button', { name: 'Sem efeito' })).toBeDisabled();
  });

  it('IconButton exige nome acessível e o svg é decorativo', () => {
    render(<IconButton icon="info" label="O que é a proteção" />);
    const button = screen.getByRole('button', { name: 'O que é a proteção' });
    expect(button.querySelector('svg')).toHaveAttribute('aria-hidden', 'true');
  });
});

describe('Switch', () => {
  it('é um switch com aria-checked e um input real por baixo', () => {
    render(
      <Switch checked={false} onChange={() => {}} label="Limitar carga" desc="Pausa aos 80%" />,
    );
    const control = screen.getByRole('switch');
    expect(control).not.toBeChecked();
    expect(control.tagName).toBe('INPUT');
    expect(screen.getByText('Limitar carga')).toBeInTheDocument();
    expect(screen.getByText('Pausa aos 80%')).toBeInTheDocument();
  });

  it('clique e teclado chegam ao onChange', async () => {
    const onChange = vi.fn();
    render(<Switch checked={false} onChange={onChange} label="Proteção" />);
    const control = screen.getByRole('switch');
    await userEvent.click(control);
    expect(onChange).toHaveBeenCalledWith(true);
    control.focus();
    await userEvent.keyboard(' ');
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it('disabled diz porque está desligado', () => {
    render(
      <Switch
        checked={false}
        onChange={() => {}}
        label="Limite"
        disabled
        disabledReason="Ligue a proteção para escolher o limite"
      />,
    );
    expect(screen.getByRole('switch')).toBeDisabled();
    expect(screen.getByText('Ligue a proteção para escolher o limite')).toBeInTheDocument();
  });

  it('pending bloqueia o controlo', () => {
    render(<Switch checked onChange={() => {}} label="A gravar" pending />);
    expect(screen.getByRole('switch')).toBeDisabled();
    expect(document.querySelector('.mc-switch--pending')).toBeTruthy();
  });

  it('bare (ToggleSwitch) fica só com o controlo', () => {
    render(<ToggleSwitch checked onChange={() => {}} ariaLabel="Wi-Fi" />);
    expect(screen.getByRole('switch', { name: 'Wi-Fi' })).toBeInTheDocument();
    expect(document.querySelector('.mc-switch--bare')).toBeTruthy();
  });

  it('ToggleRow mantém a API antiga sobre o primitivo', () => {
    const onChange = vi.fn();
    render(<ToggleRow label="Modo escuro" desc="Segue o sistema" checked onChange={onChange} />);
    expect(screen.getByRole('switch', { name: 'Modo escuro' })).toBeChecked();
    expect(screen.getByText('Segue o sistema')).toBeInTheDocument();
  });
});

describe('Slider', () => {
  it('liga label, valor formatado e aria-valuetext', async () => {
    const onChange = vi.fn();
    render(
      <Slider
        label="Brilho"
        value={40}
        min={0}
        max={100}
        onChange={onChange}
        format={(v) => `${v}%`}
      />,
    );
    expect(screen.getByText('40%')).toBeInTheDocument();
    const input = screen.getByRole('slider', { name: 'Brilho' });
    expect(input).toHaveAttribute('aria-valuetext', '40%');
    fireEvent.change(input, { target: { value: '70' } });
    expect(onChange).toHaveBeenCalledWith(70);
  });

  it('unidade entra no valor lido', () => {
    render(<Slider label="Volume" value={35} onChange={() => {}} unit="%" />);
    expect(screen.getByRole('slider')).toHaveAttribute('aria-valuetext', '35 %');
  });
});

describe('Dialog', () => {
  it('não renderiza nada fechado', () => {
    render(
      <Dialog open={false} onClose={() => {}} title="x">
        y
      </Dialog>,
    );
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('é modal, nomeado pelo título e fecha com Escape', () => {
    const onClose = vi.fn();
    render(
      <Dialog open onClose={onClose} title="O que é a proteção">
        corpo
      </Dialog>,
    );
    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveAttribute('aria-modal', 'true');
    expect(dialog).toHaveAccessibleName('O que é a proteção');
    fireEvent.keyDown(dialog, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('clique no véu fecha; clique no painel não', async () => {
    const onClose = vi.fn();
    render(
      <Dialog open onClose={onClose} title="Detalhe">
        <button type="button">dentro</button>
      </Dialog>,
    );
    await userEvent.click(screen.getByRole('button', { name: 'dentro' }));
    expect(onClose).not.toHaveBeenCalled();
    await userEvent.click(screen.getByTestId('mc-dialog-backdrop'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('o botão de fechar tem nome próprio', async () => {
    const onClose = vi.fn();
    render(
      <Dialog open onClose={onClose} title="Detalhe" closeLabel="Fechar ajuda">
        x
      </Dialog>,
    );
    await userEvent.click(screen.getByRole('button', { name: 'Fechar ajuda' }));
    expect(onClose).toHaveBeenCalled();
  });

  it('InfoModal continua a ser o mesmo contrato sobre o Dialog', () => {
    render(
      <InfoModal open onClose={() => {}} title="Glossário">
        <InfoSection title="Carga">
          <InfoRow label="AC ligado">Carregador ligado</InfoRow>
        </InfoSection>
      </InfoModal>,
    );
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(screen.getByText('Carga')).toHaveClass('mc-info-section__title');
    expect(screen.getByText('AC ligado')).toHaveClass('mc-info-row__label');
  });
});

describe('NavItem', () => {
  it('activo expõe aria-current, não só uma classe', async () => {
    const onNavigate = vi.fn();
    render(
      <NavGroup label="Energia">
        <NavItem icon="battery" active onClick={onNavigate}>
          Bateria
        </NavItem>
        <NavItem icon="fan">Ventoinha</NavItem>
      </NavGroup>,
    );
    expect(screen.getByRole('button', { name: 'Bateria' })).toHaveAttribute('aria-current', 'page');
    expect(screen.getByRole('button', { name: 'Ventoinha' })).not.toHaveAttribute('aria-current');
    expect(screen.getByRole('group', { name: 'Energia' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Bateria' }));
    expect(onNavigate).toHaveBeenCalled();
  });
});

describe('Skeleton', () => {
  it('tem a forma do conteúdo e é invisível para o leitor', () => {
    render(
      <div>
        <Skeleton shape="ring" size={96} />
        <Skeleton shape="line" width="40%" />
      </div>,
    );
    const ring = document.querySelector('.mc-skeleton--ring') as HTMLElement;
    expect(ring).toHaveAttribute('aria-hidden', 'true');
    expect(ring).toHaveStyle({ width: '96px', height: '96px' });
    expect(document.querySelector('.mc-skeleton--line')).toHaveStyle({ width: '40%' });
  });

  it('SkeletonSwap anuncia a leitura e troca sem perder o conteúdo', () => {
    const { rerender } = render(
      <SkeletonSwap loading fallback={<Skeleton shape="block" />} label="A ler a bateria">
        <span>96%</span>
      </SkeletonSwap>,
    );
    expect(screen.getByRole('status')).toHaveAttribute('aria-busy', 'true');
    expect(screen.getByText('A ler a bateria')).toHaveClass('sr-only');
    rerender(
      <SkeletonSwap loading={false} fallback={<Skeleton shape="block" />} label="A ler a bateria">
        <span>96%</span>
      </SkeletonSwap>,
    );
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
    expect(screen.getByText('96%')).toBeInTheDocument();
  });
});

describe('Tooltip', () => {
  it('substitui o title nativo: abre no hover e descreve o alvo', async () => {
    render(
      <Tooltip label="Limita a carga a 80%" delay={0}>
        <button type="button">Limite</button>
      </Tooltip>,
    );
    const bubble = document.querySelector('.mc-tooltip__bubble') as HTMLElement;
    expect(bubble).toHaveAttribute('data-open', 'false');

    await userEvent.hover(screen.getByRole('button', { name: 'Limite' }));
    expect(bubble).toHaveAttribute('data-open', 'true');
    expect(screen.getByRole('button', { name: 'Limite' })).toHaveAttribute(
      'aria-describedby',
      bubble.id,
    );

    await userEvent.unhover(screen.getByRole('button', { name: 'Limite' }));
    expect(bubble).toHaveAttribute('data-open', 'false');
  });

  it('abre no foco (teclado) e fecha com Escape', () => {
    render(
      <Tooltip label="Ajuda" delay={400}>
        <button type="button">?</button>
      </Tooltip>,
    );
    const button = screen.getByRole('button', { name: '?' });
    fireEvent.focus(button);
    expect(document.querySelector('.mc-tooltip__bubble')).toHaveAttribute('data-open', 'true');
    fireEvent.keyDown(button, { key: 'Escape' });
    expect(document.querySelector('.mc-tooltip__bubble')).toHaveAttribute('data-open', 'false');
  });
});

describe('Segmented', () => {
  const options = [
    { value: 40, label: '40%' },
    { value: 60, label: '60%' },
    { value: 80, label: '80%', badge: 'recomendado' },
  ];

  it('marca o escolhido com aria-pressed, não com uma classe', () => {
    render(<Segmented label="Limite" options={options} value={80} onChange={() => {}} />);
    expect(screen.getByRole('button', { name: /80%/ })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('button', { name: '40%' })).toHaveAttribute('aria-pressed', 'false');
    expect(screen.getByText('recomendado')).toHaveClass('mc-segmented__badge');
  });

  it('clique aplica o valor', async () => {
    const onChange = vi.fn();
    render(<Segmented options={options} value={80} onChange={onChange} />);
    await userEvent.click(screen.getByRole('button', { name: '60%' }));
    expect(onChange).toHaveBeenCalledWith(60);
  });

  it('teclado ←/→ anda entre opções', () => {
    const onChange = vi.fn();
    render(<Segmented options={options} value={60} onChange={onChange} ariaLabel="Limite" />);
    fireEvent.keyDown(screen.getByRole('group', { name: 'Limite' }), { key: 'ArrowRight' });
    expect(onChange).toHaveBeenCalledWith(80);
    fireEvent.keyDown(screen.getByRole('group', { name: 'Limite' }), { key: 'ArrowLeft' });
    expect(onChange).toHaveBeenLastCalledWith(40);
  });

  it('desligado explica-se em vez de morrer em silêncio', () => {
    render(
      <Segmented
        options={options}
        value={80}
        onChange={() => {}}
        disabled
        disabledReason="Ligue a proteção para escolher o limite"
      />,
    );
    expect(screen.getByText('Ligue a proteção para escolher o limite')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '40%' })).toBeDisabled();
  });
});

describe('Icon', () => {
  it('decorativo por omissão', () => {
    render(<Icon name="battery" />);
    const svg = document.querySelector('svg.mc-icon') as SVGElement;
    expect(svg).toHaveAttribute('aria-hidden', 'true');
    expect(svg).toHaveAttribute('stroke-width', '1.6');
    expect(svg).toHaveAttribute('stroke-linecap', 'round');
    expect(svg).toHaveAttribute('viewBox', '0 0 24 24');
  });

  it('com title passa a imagem nomeada', () => {
    render(<Icon name="plug" title="Carregador" />);
    expect(screen.getByRole('img', { name: 'Carregador' })).toBeInTheDocument();
  });
});
