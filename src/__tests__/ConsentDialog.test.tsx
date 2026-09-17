import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ConsentDialog } from '../components/ConsentDialog';

vi.mock('../hooks/useI18n', () => ({
  t: (key: string, opts?: Record<string, unknown>) => {
    if (opts) {
      return Object.entries(opts).reduce((s, [k, v]) => s.replace(`{${k}}`, String(v)), key);
    }
    return key;
  },
}));

describe('ConsentDialog', () => {
  const mockOnAllow = vi.fn();
  const mockOnDeny = vi.fn();
  const mockOnOpenPrivacy = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
  });

  function renderDialog() {
    return render(
      <ConsentDialog onAllow={mockOnAllow} onDeny={mockOnDeny} onOpenPrivacy={mockOnOpenPrivacy} />,
    );
  }

  it('renders dialog with title and buttons', () => {
    renderDialog();

    expect(screen.getByText('consent.dialog.title')).toBeInTheDocument();
    expect(screen.getByText('consent.dialog.deny')).toBeInTheDocument();
    expect(screen.getByText('consent.dialog.intro')).toBeInTheDocument();
  });

  it('clicking "Allow" calls onAllow', async () => {
    const user = userEvent.setup();
    renderDialog();

    // The Allow button is the primary button — it renders the consent.dialog.allow key
    // but the component doesn't have an explicit allow text key, so we find by the
    // deny button and get its sibling
    const denyButton = screen.getByText('consent.dialog.deny');
    const allowButton = denyButton.parentElement?.querySelector('.btn-primary') as HTMLElement;

    await user.click(allowButton);

    // S49 refactor: the choice commits after a 140ms fade-out, so the spy
    // fires asynchronously — wait for it instead of asserting immediately.
    await waitFor(() => expect(mockOnAllow).toHaveBeenCalledTimes(1));
  });

  it('clicking "Deny" calls onDeny', async () => {
    const user = userEvent.setup();
    renderDialog();

    const denyButton = screen.getByText('consent.dialog.deny');
    await user.click(denyButton);

    await waitFor(() => expect(mockOnDeny).toHaveBeenCalledTimes(1));
  });

  it('pressing Escape calls onDeny', async () => {
    renderDialog();

    const dialog = screen.getByRole('dialog');
    fireEvent.keyDown(dialog, { key: 'Escape' });

    await waitFor(() => expect(mockOnDeny).toHaveBeenCalledTimes(1));
  });

  it('clicking privacy link calls onOpenPrivacy', async () => {
    const user = userEvent.setup();
    renderDialog();

    const privacyLink = screen.getByText('consent.dialog.privacyLink');
    await user.click(privacyLink);

    expect(mockOnOpenPrivacy).toHaveBeenCalledTimes(1);
  });
});
