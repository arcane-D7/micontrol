import { describe, it, expect, vi, beforeAll } from 'vitest';
import { render, screen } from '@testing-library/react';
import { Sidebar } from '../pages/MainWindow';
import { t } from '../hooks/useI18n';
import type { useHardware } from '../hooks/useHardware';

// O Sidebar vive em MainWindow.tsx, que arrasta módulos com invocações Tauri
// no arranque. Nenhum é montado aqui; só precisam de carregar.
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue({}) }));

// jsdom não implementa scrollIntoView (o efeito "manter o activo à vista").
beforeAll(() => {
  Element.prototype.scrollIntoView = vi.fn();
});

type Hardware = ReturnType<typeof useHardware>;

function makeHardware(error: string | null): Hardware {
  return { loading: false, error } as unknown as Hardware;
}

function renderSidebar(hardware: Hardware) {
  return render(
    <Sidebar
      activeTab="overview"
      onTabChange={vi.fn()}
      themeMode="auto"
      toggleTheme={vi.fn()}
      hardware={hardware}
      setShowTrayPreview={vi.fn()}
      onOpenSystemLog={vi.fn()}
    />,
  );
}

describe('Sidebar: rodapé limpo e fundo fixo', () => {
  it('sem erro não há linha de estado nem texto de atalhos', () => {
    const { container } = renderSidebar(makeHardware(null));
    expect(container.querySelector('.sidebar-statusrow')).toBeNull();
    expect(container.textContent).not.toContain('Alt+1');
    expect(container.querySelector('.sidebar-pinned')).toBeNull();
  });

  it('com erro o botão System log aparece no rodapé', () => {
    renderSidebar(makeHardware('ecram driver blocked'));
    const log = screen.getByRole('button', { name: t('systemLog.title') });
    expect(log.closest('.sidebar-statusrow')).not.toBeNull();
    expect(log.closest('.sidebar-footer')).not.toBeNull();
  });

  it('Settings e About são fixos no fundo, fora da lista rolável', () => {
    const { container } = renderSidebar(makeHardware(null));
    const scroll = container.querySelector('.sidebar-scroll') as HTMLElement;
    const foot = container.querySelector('.sidebar-foot') as HTMLElement;
    expect(foot).not.toBeNull();
    const settings = screen.getByRole('button', { name: t('nav.settings') });
    const about = screen.getByRole('button', { name: t('nav.about') });
    expect(foot.contains(settings)).toBe(true);
    expect(foot.contains(about)).toBe(true);
    expect(scroll.contains(settings)).toBe(false);
    expect(scroll.contains(about)).toBe(false);
    // O fundo fixo vem depois da lista e é separado por hairline (border-top).
    expect(scroll.compareDocumentPosition(foot) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('toggle de tema é ícone com aria-label e tooltip, à esquerda do About', () => {
    const { container } = renderSidebar(makeHardware(null));
    const foot = container.querySelector('.sidebar-foot') as HTMLElement;
    const toggle = screen.getByRole('button', { name: t('main.themeToggle') });
    expect(toggle).toHaveAttribute('aria-label', t('main.themeToggle'));
    expect(toggle.getAttribute('title')).toBeTruthy();
    expect(toggle.textContent).toBe('');
    // Alvo compacto (28 px) do primitivo de ícone.
    expect(toggle.className).toContain('mc-iconbtn');
    expect(toggle.className).toContain('mc-iconbtn--sm');
    // Imediatamente à esquerda do Acerca dentro do fundo fixo.
    const order = Array.from(foot.children) as HTMLElement[];
    const about = screen.getByRole('button', { name: t('nav.about') });
    const iAbout = order.indexOf(about);
    expect(iAbout).toBeGreaterThan(0);
    expect(order[iAbout - 1]).toBe(toggle);
  });
});
