import { describe, it, expect, vi, beforeAll, beforeEach } from 'vitest';
import { render, screen, fireEvent, within } from '@testing-library/react';
import { setLanguage } from '../hooks/useI18n';
import { SystemLogPanel } from '../components/SystemLogPanel';

/**
 * Painel "System log" — o sítio próprio dos erros (a sidebar não é um log).
 * Contrato fixado: abre/fecha por teclado (Escape), `role=dialog` com nome
 * acessível e `aria-modal`, entradas com timestamp + severidade + Copy, o
 * erro vivo do hook entra no topo sem duplicar, e log vazio tem estado próprio.
 */

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke }));

const LOG_RAW = [
  '2026-11-18T09:00:00.000Z [hw::fan] ERROR: get_fan_info failed: WMI stall',
  '2026-11-19T11:30:00.000Z [hw::ecram] ERROR: SMA2 could not be read: Access is denied (os error 5)',
].join('\n');

beforeAll(() => {
  // jsdom não tem Clipboard API; o painel usa `navigator.clipboard.writeText`.
  Object.defineProperty(navigator, 'clipboard', {
    value: { writeText: vi.fn().mockResolvedValue(undefined) },
    configurable: true,
  });
});

beforeEach(() => {
  setLanguage('en');
  vi.mocked(navigator.clipboard.writeText).mockClear();
  invoke.mockReset();
  invoke.mockResolvedValue(LOG_RAW);
});

function rows() {
  return screen.getAllByRole('listitem');
}

describe('SystemLogPanel — estrutura acessível', () => {
  it('renderiza um dialog nomeado "System log" com aria-modal', () => {
    render(<SystemLogPanel open onClose={vi.fn()} currentError={null} />);

    const dialog = screen.getByRole('dialog', { name: 'System log' });
    expect(dialog).toBeInTheDocument();
    expect(dialog).toHaveAttribute('aria-modal', 'true');
  });

  it('fechado não renderiza nada', () => {
    const { container } = render(<SystemLogPanel open={false} onClose={vi.fn()} />);
    expect(container).toBeEmptyDOMElement();
  });
});

describe('SystemLogPanel — teclado', () => {
  it('Escape fecha o painel', () => {
    const onClose = vi.fn();
    render(<SystemLogPanel open onClose={onClose} currentError={null} />);

    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe('SystemLogPanel — entradas', () => {
  it('lista as entradas do errors.log, mais recente primeiro, com severidade', async () => {
    render(<SystemLogPanel open onClose={vi.fn()} currentError={null} />);

    expect(await screen.findByText(/SMA2 could not be read/)).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith('read_error_log', { maxLines: 100 });

    const list = rows();
    expect(list).toHaveLength(2);
    expect(list[0]).toHaveTextContent('SMA2 could not be read');
    expect(list[1]).toHaveTextContent('get_fan_info failed');
    expect(within(list[0]).getAllByText('ERROR')).toHaveLength(1);
    // Célula de tempo com timestamp formatado (a mensagem também tem dígitos,
    // por isso o alvo é a célula, não a linha).
    const time = list[0].querySelector('.mc-logrow__time');
    expect(time?.textContent).toMatch(/\d/);
  });

  it('o erro vivo do hook entra no topo quando o log já o tem (dedupe)', async () => {
    render(
      <SystemLogPanel
        open
        onClose={vi.fn()}
        currentError="SMA2 could not be read: Access is denied (os error 5)"
      />,
    );

    await screen.findByText(/SMA2 could not be read/);
    // Uma só entrada com essa mensagem — e nenhuma "Now": a sobrevivente é a
    // do log, que tem timestamp.
    expect(screen.getAllByText(/SMA2 could not be read/)).toHaveLength(1);
    expect(screen.queryAllByText('Now')).toHaveLength(0);
    expect(rows()).toHaveLength(2);
  });

  it('erro vivo novo (sem par no log) entra primeiro, sem timestamp', async () => {
    render(<SystemLogPanel open onClose={vi.fn()} currentError="fresh failure" />);

    await screen.findByText('fresh failure');
    expect(rows()[0]).toHaveTextContent('fresh failure');
    expect(within(rows()[0]).getByText('Now')).toBeInTheDocument();
    expect(rows()).toHaveLength(3);
  });

  it('log vazio e sem erro vivo mostra o estado vazio', async () => {
    invoke.mockResolvedValue('');
    render(<SystemLogPanel open onClose={vi.fn()} currentError={null} />);

    expect(await screen.findByText('No errors recorded.')).toBeInTheDocument();
    expect(screen.queryAllByRole('listitem')).toHaveLength(0);
  });

  it('falha do comando não apaga o erro vivo', async () => {
    invoke.mockRejectedValue('read failed');
    render(<SystemLogPanel open onClose={vi.fn()} currentError="fresh failure" />);

    expect(await screen.findByText('fresh failure')).toBeInTheDocument();
    expect(rows()).toHaveLength(1);
  });
});

describe('SystemLogPanel — Copy', () => {
  it('copia a entrada completa e confirma com "Copied"', async () => {
    render(<SystemLogPanel open onClose={vi.fn()} currentError="fresh failure" />);

    await screen.findByText('fresh failure');
    fireEvent.click(within(rows()[0]).getByRole('button', { name: 'Copy' }));

    expect(navigator.clipboard.writeText).toHaveBeenCalledWith('fresh failure');
    // O estado "Copiado" chega num microtask depois da cópia.
    expect(await within(rows()[0]).findByRole('button', { name: 'Copied' })).toBeInTheDocument();
  });

  it('a entrada com timestamp copia "ts [SEVERITY] mensagem"', async () => {
    render(<SystemLogPanel open onClose={vi.fn()} currentError={null} />);

    await screen.findByText(/SMA2 could not be read/);
    fireEvent.click(within(rows()[0]).getByRole('button', { name: 'Copy' }));

    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
      '2026-11-19T11:30:00.000Z [ERROR] SMA2 could not be read: Access is denied (os error 5)',
    );
  });
});
