import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import StartupManager from '../components/StartupManager';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

interface MockItem {
  id: string;
  name: string;
  command: string;
  executable: string | null;
  publisher: string | null;
  version: string | null;
  file_exists: boolean | null;
  enabled: boolean;
  source: string;
  location: string;
  requires_admin: boolean;
  link_key: string | null;
  linked_count: number;
  toggleable: boolean;
  toggle_blocked: string | null;
  managed_in_settings: boolean;
  principal: string | null;
}

function item(overrides: Partial<MockItem> & Pick<MockItem, 'id' | 'name'>): MockItem {
  return {
    command: `"C:\\Apps\\${overrides.name}.exe"`,
    executable: `C:\\Apps\\${overrides.name}.exe`,
    publisher: 'Example Corp',
    version: '1.2.3.4',
    file_exists: true,
    enabled: true,
    source: 'hkcu_run',
    location: 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run',
    requires_admin: false,
    link_key: null,
    linked_count: 1,
    toggleable: true,
    toggle_blocked: null,
    managed_in_settings: false,
    principal: null,
    ...overrides,
  };
}

const ITEMS: MockItem[] = [
  item({
    id: 'hkcu_run|MiControl',
    name: 'MiControl',
    link_key: 'c:\\apps\\micontrol.exe',
    linked_count: 2,
  }),
  item({
    id: 'hklm_run|MiControl',
    name: 'MiControl',
    source: 'hklm_run',
    location: 'HKLM\\Software\\Microsoft\\Windows\\CurrentVersion\\Run',
    requires_admin: true,
    link_key: 'c:\\apps\\micontrol.exe',
    linked_count: 2,
  }),
  item({
    id: 'hkcu_run|Discord',
    name: 'Discord',
    enabled: false,
    publisher: 'Discord Inc.',
  }),
  item({
    id: 'user_logon_task|\\Legacy App',
    name: 'Legacy App',
    source: 'user_logon_task',
    location: '\\Legacy App',
    executable: 'C:\\Gone\\legacy.exe',
    command: '"C:\\Gone\\legacy.exe" /run',
    file_exists: false,
    publisher: null,
    principal: 'tester',
  }),
];

function snapshot(overrides: Record<string, unknown> = {}) {
  return {
    items: ITEMS,
    tasks_available: true,
    task_scan_error: null,
    tasks_skipped: 0,
    cached: false,
    ...overrides,
  };
}

function mockBackend(data = snapshot()) {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === 'get_startup_items') return Promise.resolve(data);
    if (cmd === 'set_startup_item') return Promise.resolve();
    return Promise.resolve();
  });
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe('StartupManager — listagem e filtros', () => {
  it('mostra a contagem global e agrupa por localização', async () => {
    mockBackend();
    render(<StartupManager />);

    expect(await screen.findByText('3 of 4 active')).toBeInTheDocument();
    expect(screen.getByText('Run — current user')).toBeInTheDocument();
    expect(screen.getByText('Run — all users')).toBeInTheDocument();
    expect(screen.getByText('Logon task — current user')).toBeInTheDocument();
    expect(screen.getAllByText('MiControl')).toHaveLength(2);
  });

  it('filtra pela pesquisa e pelo estado', async () => {
    mockBackend();
    render(<StartupManager />);
    await screen.findByText('3 of 4 active');

    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'discord' } });
    expect(screen.queryByText('MiControl')).not.toBeInTheDocument();
    expect(screen.getByText('Discord')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('radio', { name: 'Enabled' }));
    expect(screen.queryByText('Discord')).not.toBeInTheDocument();
    expect(screen.getByText(/No entry matches/)).toBeInTheDocument();
  });

  it('agrupa entradas ligadas pelo mesmo executável', async () => {
    mockBackend();
    render(<StartupManager />);
    await screen.findByText('3 of 4 active');

    fireEvent.click(screen.getByRole('radio', { name: 'Program' }));
    const group = screen.getByText('MiControl.exe').closest('section');
    expect(group).not.toBeNull();
    expect(within(group as HTMLElement).getAllByText('MiControl')).toHaveLength(2);
    expect(within(group as HTMLElement).getByText('Run — all users')).toBeInTheDocument();
  });
});

describe('StartupManager — estados honestos', () => {
  it('marca admin, ficheiro inexistente e conta da tarefa', async () => {
    mockBackend();
    render(<StartupManager />);
    await screen.findByText('3 of 4 active');

    expect(screen.getAllByText('Admin required')).toHaveLength(1);
    expect(screen.getByText('File missing')).toBeInTheDocument();
    expect(screen.getByText(/Account: tester/)).toBeInTheDocument();
  });

  it('diz quando as tarefas agendadas não puderam ser lidas', async () => {
    mockBackend(
      snapshot({
        tasks_available: false,
        task_scan_error: 'Task Scheduler unavailable',
        tasks_skipped: 3,
      }),
    );
    render(<StartupManager />);

    expect(await screen.findByText(/Scheduled tasks could not be read/)).toHaveTextContent(
      'Task Scheduler unavailable',
    );
    expect(screen.getByText(/3 tasks could not be read/)).toBeInTheDocument();
  });

  it('mostra a entrada do MiControl como gerida em Definições', async () => {
    mockBackend(
      snapshot({
        items: [
          item({
            id: 'hkcu_run|MiControl',
            name: 'MiControl',
            managed_in_settings: true,
          }),
        ],
      }),
    );
    render(<StartupManager />);

    expect(await screen.findByText('Managed in Settings')).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: 'Enable MiControl at startup' })).toBeDisabled();
  });

  it('mostra empty state quando não há entradas', async () => {
    mockBackend(snapshot({ items: [] }));
    render(<StartupManager />);

    expect(await screen.findByText('No startup entries')).toBeInTheDocument();
    expect(screen.getByText('0 of 0 active')).toBeInTheDocument();
  });

  it('mostra o erro de carregamento com retry', async () => {
    invokeMock.mockRejectedValueOnce(new Error('registry locked'));
    render(<StartupManager />);

    expect(await screen.findByText(/Could not read the startup entries/)).toHaveTextContent(
      'registry locked',
    );
    expect(screen.getByRole('button', { name: /Try again/ })).toBeInTheDocument();
  });
});

describe('StartupManager — toggle', () => {
  it('envia o id e o novo estado e atualiza o interruptor', async () => {
    mockBackend();
    render(<StartupManager />);
    await screen.findByText('3 of 4 active');

    const discord = screen.getByRole('switch', { name: 'Enable Discord at startup' });
    expect(discord).toHaveAttribute('aria-checked', 'false');
    fireEvent.click(discord);

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_startup_item', {
        id: 'hkcu_run|Discord',
        enabled: true,
      }),
    );
    await waitFor(() => expect(discord).toHaveAttribute('aria-checked', 'true'));
  });

  it('mantém o estado e mostra o erro quando a escrita falha', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_startup_items') return Promise.resolve(snapshot());
      if (cmd === 'set_startup_item') return Promise.reject(new Error('access denied'));
      return Promise.resolve();
    });
    render(<StartupManager />);
    await screen.findByText('3 of 4 active');

    const discord = screen.getByRole('switch', { name: 'Enable Discord at startup' });
    fireEvent.click(discord);

    expect(
      await screen.findByText(/Could not change this entry: access denied/),
    ).toBeInTheDocument();
    expect(discord).toHaveAttribute('aria-checked', 'false');
  });

  it('desativa o interruptor de ficheiros diretos na pasta Startup', async () => {
    const blocked = item({
      id: 'user_startup_folder|direct.exe',
      name: 'direct',
      source: 'user_startup_folder',
      location: 'C:\\Users\\tester\\Startup',
      toggleable: false,
      toggle_blocked: 'folder_entry_not_shortcut',
    });
    mockBackend(snapshot({ items: [blocked] }));
    render(<StartupManager />);

    expect(await screen.findByText(/Only shortcuts/)).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: 'Enable direct at startup' })).toBeDisabled();
  });
});
