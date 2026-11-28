import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import FanControl from '../components/FanControl';
import type { FanInfo, FanModeResult } from '../types/hardware';

/**
 * Fan Control honesty tests (audit P2-1 / P2-2).
 *
 * The production bug was a card that showed "0 RPM" and a 50 % gauge at the
 * same time, because the percentage came from the registry while the RPM came
 * from a sensor that does not exist on this platform.
 */
function makeFan(overrides: Partial<FanInfo> = {}): FanInfo {
  return {
    mode: 'auto',
    speed_rpm: null,
    speed_percent: null,
    requested_speed_percent: 50,
    gpu_temp_celsius: 56,
    cpu_temp_celsius: 57,
    ambient_temp_celsius: 27.85,
    tdp_watts: 15.2,
    temp_source: 'esif',
    ...overrides,
  };
}

describe('FanControl — no RPM sensor', () => {
  it('shows N/A instead of a fake 0 RPM, and draws no gauge', () => {
    const { container } = render(<FanControl fan={makeFan()} onModeChange={vi.fn()} />);

    expect(screen.getByText('N/A')).toBeInTheDocument();
    expect(screen.queryByText(/^0\s*RPM$/)).not.toBeInTheDocument();
    // The 50 % registry value must not become a gauge.
    expect(container.querySelector('.fan-segments')).toBeNull();
    expect(screen.getByText(/no RPM sensor/i)).toBeInTheDocument();
  });

  it('does not pretend the requested percentage is the current speed', () => {
    const { container } = render(
      <FanControl fan={makeFan({ requested_speed_percent: 80 })} onModeChange={vi.fn()} />,
    );
    expect(container.querySelector('.fan-segments')).toBeNull();
    expect(screen.queryByText('80%')).not.toBeInTheDocument();
  });

  it('reports ambient from TZ00 separately from the GPU temperature', () => {
    render(<FanControl fan={makeFan()} onModeChange={vi.fn()} />);
    expect(screen.getByText(/27\.9°/)).toBeInTheDocument();
  });
});

describe('FanControl — real sensor', () => {
  it('shows the measured RPM and derives the gauge from it', () => {
    const { container } = render(
      <FanControl fan={makeFan({ speed_rpm: 3000, speed_percent: 60 })} onModeChange={vi.fn()} />,
    );
    expect(container.querySelector('.fan-segments')).not.toBeNull();
    expect(screen.getByText(/3[,.]?000\s*RPM/)).toBeInTheDocument();
    expect(screen.queryByText('N/A')).not.toBeInTheDocument();
  });
});

describe('FanControl — set_fan_mode result', () => {
  it('shows the honest caveat when Fixed Speed cannot pin RPM', async () => {
    const result: FanModeResult = {
      applied: true,
      method: 'registry+ec_profile',
      rpm_control: false,
      warning: "Fixed speed requested, but no RPM control: profile 'Balanced' applied instead.",
    };
    const onModeChange = vi.fn().mockResolvedValue(result);
    render(<FanControl fan={makeFan({ mode: 'fixed' })} onModeChange={onModeChange} />);

    fireEvent.click(screen.getByRole('button', { name: /Fixed Speed/i }));

    await waitFor(() => expect(onModeChange).toHaveBeenCalledWith('fixed', 50));
    expect(await screen.findByText(/no RPM control: profile 'Balanced'/i)).toBeInTheDocument();
  });

  it('keeps the slider on the stored request, not on a measurement', async () => {
    const onModeChange = vi.fn().mockResolvedValue({
      applied: true,
      method: 'registry+ec_profile',
      rpm_control: false,
      warning: null,
    });
    render(
      <FanControl
        fan={makeFan({ mode: 'fixed', requested_speed_percent: 75 })}
        onModeChange={onModeChange}
      />,
    );

    const slider = screen.getByRole('slider') as HTMLInputElement;
    expect(slider.value).toBe('75');

    fireEvent.change(slider, { target: { value: '85' } });
    fireEvent.mouseUp(slider);
    await waitFor(() => expect(onModeChange).toHaveBeenCalledWith('fixed', 85));
  });

  it('surfaces a failure as an error instead of a success, and reverts the button', async () => {
    const onModeChange = vi.fn().mockRejectedValue(new Error('elevation denied'));
    render(<FanControl fan={makeFan()} onModeChange={onModeChange} />);

    fireEvent.click(screen.getByRole('button', { name: /Off/i }));
    await waitFor(() => expect(onModeChange).toHaveBeenCalledWith('off', undefined));

    // Nothing was applied, so the Off button must not stay highlighted.
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Off/i }).className).not.toMatch(/active/),
    );
    expect(screen.getByRole('button', { name: /Auto/i }).className).toMatch(/active/);
  });
});
