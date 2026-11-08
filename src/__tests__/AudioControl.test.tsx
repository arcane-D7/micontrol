import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen } from '@testing-library/react';
import AudioControl from '../components/AudioControl';
import { ToastProvider } from '../contexts/ToastContext';
import { t } from '../hooks/useI18n';
import type { AudioVolumeResult } from '../types/hardware';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue({ playback: [], capture: [] }),
}));

const audioState = (volume: number, muted: boolean): AudioVolumeResult => ({
  success: true,
  volume,
  muted,
});

async function renderControl(overrides: { volume?: number; muted?: boolean } = {}) {
  const onVolumeChange = vi.fn(async (_fraction: number) => {});
  const onMuteToggle = vi.fn(async (_muted: boolean) => {});
  render(
    <ToastProvider>
      <AudioControl
        audioState={audioState(overrides.volume ?? 14, overrides.muted ?? false)}
        loading={false}
        onVolumeChange={onVolumeChange}
        onMuteToggle={onMuteToggle}
      />
    </ToastProvider>,
  );
  // Let the mount-time `get_audio_devices` promise settle inside act().
  await act(async () => {});
  return { onVolumeChange, onMuteToggle, slider: screen.getByRole('slider') };
}

/** Let the 150 ms trailing debounce fire inside act(). */
async function advance(ms = 150) {
  await act(async () => {
    vi.advanceTimersByTime(ms);
  });
}

describe('AudioControl slider', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('shows the real volume while muted instead of pinning the thumb to 0', async () => {
    // Audit P2-C: `value={muted ? 0 : volume}` made the slider unusable.
    const { slider } = await renderControl({ volume: 14, muted: true });
    expect(slider).toHaveValue('14');
  });

  it('unmutes before applying a volume change made while muted', async () => {
    const { onVolumeChange, onMuteToggle, slider } = await renderControl({
      volume: 14,
      muted: true,
    });

    fireEvent.change(slider, { target: { value: '40' } });
    await advance();

    expect(onMuteToggle).toHaveBeenCalledWith(false);
    expect(onVolumeChange).toHaveBeenCalledWith(0.4);
    // Unmute must land before the volume write, or the user still hears
    // nothing for one round-trip.
    expect(onMuteToggle.mock.invocationCallOrder[0]).toBeLessThan(
      onVolumeChange.mock.invocationCallOrder[0],
    );
    expect(screen.getByText(t('audio.unmutedByVolume'))).toBeInTheDocument();
  });

  it('does not unmute again within the same drag', async () => {
    const { onMuteToggle, slider } = await renderControl({ volume: 14, muted: true });

    fireEvent.change(slider, { target: { value: '40' } });
    await advance();
    fireEvent.change(slider, { target: { value: '60' } });
    await advance();

    // The polled `muted` flag lags behind the endpoint; the component must not
    // toast/call unmute twice for one gesture.
    expect(onMuteToggle).toHaveBeenCalledTimes(1);
  });

  it('collapses a drag into a single volume write', async () => {
    const { onVolumeChange, slider } = await renderControl({ volume: 14, muted: false });

    for (const value of ['20', '30', '45', '55']) {
      fireEvent.change(slider, { target: { value } });
    }
    await advance();

    expect(onVolumeChange).toHaveBeenCalledTimes(1);
    expect(onVolumeChange).toHaveBeenCalledWith(0.55);
  });

  it('flushes immediately when the pointer is released', async () => {
    const { onVolumeChange, slider } = await renderControl({ volume: 14, muted: false });

    fireEvent.change(slider, { target: { value: '70' } });
    fireEvent.pointerUp(slider);

    // No timer advance needed: release commits right away.
    await act(async () => {});
    expect(onVolumeChange).toHaveBeenCalledWith(0.7);
  });

  it('leaves mute alone when the endpoint is not muted', async () => {
    const { onMuteToggle, slider } = await renderControl({ volume: 14, muted: false });

    fireEvent.change(slider, { target: { value: '25' } });
    await advance();

    expect(onMuteToggle).not.toHaveBeenCalled();
    expect(screen.queryByText(t('audio.unmutedByVolume'))).not.toBeInTheDocument();
  });

  it('stays muted when dragged to zero', async () => {
    const { onVolumeChange, onMuteToggle, slider } = await renderControl({
      volume: 14,
      muted: true,
    });

    fireEvent.change(slider, { target: { value: '0' } });
    await advance();

    expect(onMuteToggle).not.toHaveBeenCalled();
    expect(onVolumeChange).toHaveBeenCalledWith(0);
  });
});
