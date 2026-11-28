import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import AiUsagePanel from '../components/AiUsagePanel';
import { t } from '../hooks/useI18n';

const invoke = vi.fn();

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}));

describe('AiUsagePanel', () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it('states that AI is not configured instead of showing zeroed counters', () => {
    render(<AiUsagePanel configured={false} onOpenSettings={vi.fn()} />);

    expect(screen.getByText(t('ai.notConfigured'), { exact: false })).toBeInTheDocument();
    // The fake-data smell: "0 requests · 0 tokens · $0.0000".
    expect(screen.queryByText(t('ai.usage.requests'), { exact: false })).not.toBeInTheDocument();
    expect(screen.queryByText(t('ai.usage.reset'))).not.toBeInTheDocument();
    // No pointless IPC when there is nothing to read.
    expect(invoke).not.toHaveBeenCalled();
  });

  it('hides the counters until the first request is recorded', async () => {
    invoke.mockResolvedValue({
      total_requests: 0,
      total_input_tokens: 0,
      total_output_tokens: 0,
      estimated_cost_usd: 0,
    });

    render(<AiUsagePanel configured onOpenSettings={vi.fn()} />);

    await waitFor(() => expect(screen.getByText(t('ai.usage.noRequests'))).toBeInTheDocument());
    expect(screen.queryByText(t('ai.usage.reset'))).not.toBeInTheDocument();
  });

  it('renders the real counters once requests exist', async () => {
    invoke.mockResolvedValue({
      total_requests: 3,
      total_input_tokens: 1200,
      total_output_tokens: 300,
      estimated_cost_usd: 0.0456,
    });

    render(<AiUsagePanel configured onOpenSettings={vi.fn()} />);

    await waitFor(() => expect(screen.getByText(t('ai.usage.reset'))).toBeInTheDocument());
    expect(screen.getByText('3')).toBeInTheDocument();
    expect(screen.getByText('1500')).toBeInTheDocument();
    expect(screen.getByText('$0.0456')).toBeInTheDocument();
  });
});
