import { describe, it, expect, vi, afterAll } from 'vitest';
import { render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { ErrorBoundary } from '../components/ErrorBoundary';

// Mock Tauri APIs
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue({}),
}));

// Component that throws an error
function ThrowOnRender({ error }: { error: Error }): ReactNode {
  throw error;
}

// Component that renders normally
function GoodComponent() {
  return <div data-testid="child">Child content</div>;
}

describe('ErrorBoundary', () => {
  // Suppress console.error for these tests (React logs caught errors)
  const originalError = console.error;
  vi.spyOn(console, 'error').mockImplementation(() => {});

  it('renders children when no error', () => {
    render(
      <ErrorBoundary>
        <GoodComponent />
      </ErrorBoundary>,
    );
    expect(screen.getByTestId('child')).toBeInTheDocument();
  });

  it('catches errors and shows reload button', () => {
    render(
      <ErrorBoundary>
        <ThrowOnRender error={new Error('Test crash')} />
      </ErrorBoundary>,
    );
    // The error boundary should show the reload button
    const reloadButton = screen.getByRole('button', { name: /reload/i });
    expect(reloadButton).toBeInTheDocument();
  });

  it('shows error message when caught', () => {
    render(
      <ErrorBoundary>
        <ThrowOnRender error={new Error('Custom error message')} />
      </ErrorBoundary>,
    );
    // The error message should be displayed
    expect(screen.getByText('Custom error message')).toBeInTheDocument();
  });

  it('shows report issue button when error is caught', () => {
    render(
      <ErrorBoundary>
        <ThrowOnRender error={new Error('Crash test')} />
      </ErrorBoundary>,
    );
    const reportButton = screen.getByRole('button', { name: /report issue/i });
    expect(reportButton).toBeInTheDocument();
  });

  // Restore console.error
  afterAll(() => {
    console.error = originalError;
  });
});
