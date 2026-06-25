import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import OnboardingWizard from '../components/OnboardingWizard';

vi.mock('../hooks/useI18n', () => ({
  t: (key: string, opts?: Record<string, unknown>) => {
    if (opts) {
      return Object.entries(opts).reduce((s, [k, v]) => s.replace(`{${k}}`, String(v)), key);
    }
    return key;
  },
}));

describe('OnboardingWizard', () => {
  const mockOnFinish = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
  });

  function renderWizard() {
    return render(<OnboardingWizard onFinish={mockOnFinish} />);
  }

  it('renders first step (welcome)', () => {
    renderWizard();

    expect(screen.getByText('onboarding.welcome.title')).toBeInTheDocument();
    expect(screen.getByText('onboarding.welcome.description')).toBeInTheDocument();
  });

  it('clicking "Next" advances to second step (privacy)', async () => {
    const user = userEvent.setup();
    renderWizard();

    // On the welcome step, the primary button says "onboarding.welcome.getStarted"
    const nextButton = screen.getByText('onboarding.welcome.getStarted');
    await user.click(nextButton);

    expect(screen.getByText('onboarding.privacy.title')).toBeInTheDocument();
    expect(screen.getByText('onboarding.privacy.description')).toBeInTheDocument();
  });

  it('clicking "Back" returns to previous step', async () => {
    const user = userEvent.setup();
    renderWizard();

    // Advance to privacy step
    const nextButton = screen.getByText('onboarding.welcome.getStarted');
    await user.click(nextButton);

    expect(screen.getByText('onboarding.privacy.title')).toBeInTheDocument();

    // Click Back
    const backButton = screen.getByText('onboarding.back');
    await user.click(backButton);

    expect(screen.getByText('onboarding.welcome.title')).toBeInTheDocument();
  });

  it('clicking "Skip" calls onFinish', async () => {
    const user = userEvent.setup();
    renderWizard();

    const skipButton = screen.getByText('onboarding.skip');
    await user.click(skipButton);

    expect(mockOnFinish).toHaveBeenCalledTimes(1);
  });

  it('clicking "Finish" on last step calls onFinish', async () => {
    const user = userEvent.setup();
    renderWizard();

    // Step 1: welcome → privacy
    await user.click(screen.getByText('onboarding.welcome.getStarted'));
    // Step 2: privacy → features (use the "next" button which is onboarding.next or privacy.allow)
    await user.click(screen.getByText('onboarding.privacy.allow'));
    // Step 3: features → ready
    await user.click(screen.getByText('onboarding.next'));

    // Now on the ready step
    expect(screen.getByText('onboarding.ready.title')).toBeInTheDocument();

    const finishButton = screen.getByText('onboarding.ready.finish');
    await user.click(finishButton);

    expect(mockOnFinish).toHaveBeenCalledTimes(1);
  });

  it('pressing Escape calls onFinish', async () => {
    const user = userEvent.setup();
    renderWizard();

    // Focus the wizard and press Escape
    const dialog = screen.getByRole('dialog');
    dialog.focus();
    await user.keyboard('{Escape}');

    expect(mockOnFinish).toHaveBeenCalledTimes(1);
  });
});
