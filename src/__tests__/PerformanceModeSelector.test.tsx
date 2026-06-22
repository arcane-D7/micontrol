import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import PerformanceModeSelector from '../components/PerformanceModeSelector';
import type { PerformanceMode as _PerformanceMode } from '../hooks/useHardware';

describe('PerformanceModeSelector', () => {
  const mockOnChange = vi.fn().mockResolvedValue(undefined);

  beforeEach(() => {
    mockOnChange.mockClear();
  });

  it('renders all 6 performance modes', () => {
    render(<PerformanceModeSelector current="balance" onChange={mockOnChange} />);
    expect(screen.getByText('Silence')).toBeInTheDocument();
    expect(screen.getByText('Balance')).toBeInTheDocument();
    expect(screen.getByText('Turbo')).toBeInTheDocument();
    expect(screen.getByText('Smart')).toBeInTheDocument();
    expect(screen.getByText('Long Battery')).toBeInTheDocument();
    expect(screen.getByText('Smart Acceleration')).toBeInTheDocument();
  });

  it('marks the current mode as active', () => {
    render(<PerformanceModeSelector current="turbo" onChange={mockOnChange} />);
    const turboBtn = screen.getByText('Turbo').closest('button');
    expect(turboBtn).toHaveClass('active');
  });

  it('calls onChange when a mode button is clicked', () => {
    render(<PerformanceModeSelector current="balance" onChange={mockOnChange} />);
    const silenceBtn = screen.getByText('Silence').closest('button');
    fireEvent.click(silenceBtn!);
    expect(mockOnChange).toHaveBeenCalledWith('silence');
  });

  it('disables buttons when disabled prop is true', () => {
    render(<PerformanceModeSelector current="balance" onChange={mockOnChange} disabled={true} />);
    const buttons = screen.getAllByRole('button');
    buttons.forEach((btn) => {
      expect(btn).toBeDisabled();
    });
  });

  it('does not mark wrong mode as active', () => {
    render(<PerformanceModeSelector current="smart" onChange={mockOnChange} />);
    const balanceBtn = screen.getByText('Balance').closest('button');
    expect(balanceBtn).not.toHaveClass('active');
  });
});
