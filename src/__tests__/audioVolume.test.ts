import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { clampVolume, debounceCommit, resolveVolumeIntent } from '../lib/audioVolume';

describe('resolveVolumeIntent', () => {
  it('keeps the real volume while muted and asks for an unmute', () => {
    // Regression for audit P2-C: the slider used to be forced to 0 and
    // set_audio_volume never unmuted, so the user heard nothing.
    expect(resolveVolumeIntent(40, true)).toEqual({ volume: 40, unmute: true });
  });

  it('does not touch mute when the endpoint is already unmuted', () => {
    expect(resolveVolumeIntent(40, false)).toEqual({ volume: 40, unmute: false });
  });

  it('stays muted when the slider is dragged to zero', () => {
    // Unmuting a silent endpoint would only flip the icon and the label.
    expect(resolveVolumeIntent(0, true)).toEqual({ volume: 0, unmute: false });
  });

  it('clamps and rounds out-of-range input', () => {
    expect(clampVolume(140)).toBe(100);
    expect(clampVolume(-5)).toBe(0);
    expect(clampVolume(33.6)).toBe(34);
    expect(clampVolume(Number.NaN)).toBe(0);
    expect(resolveVolumeIntent(140, true)).toEqual({ volume: 100, unmute: true });
  });
});

describe('debounceCommit', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('collapses a burst into one commit carrying the last value', async () => {
    const commit = vi.fn(async (_value: number) => {});
    const debounced = debounceCommit(commit, 150);

    debounced(10);
    debounced(20);
    debounced(30);

    await vi.advanceTimersByTimeAsync(149);
    expect(commit).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(1);
    expect(commit).toHaveBeenCalledTimes(1);
    expect(commit).toHaveBeenCalledWith(30);
  });

  it('flush commits the pending value immediately', async () => {
    const commit = vi.fn(async (_value: number) => {});
    const debounced = debounceCommit(commit, 150);

    debounced(55);
    expect(debounced.hasPending()).toBe(true);

    await debounced.flush();

    expect(commit).toHaveBeenCalledTimes(1);
    expect(commit).toHaveBeenCalledWith(55);
    expect(debounced.hasPending()).toBe(false);

    // The cancelled timer must not fire a second commit.
    await vi.advanceTimersByTimeAsync(500);
    expect(commit).toHaveBeenCalledTimes(1);
  });

  it('cancel drops the pending value', async () => {
    const commit = vi.fn(async (_value: number) => {});
    const debounced = debounceCommit(commit, 150);

    debounced(70);
    debounced.cancel();

    await vi.advanceTimersByTimeAsync(500);
    expect(commit).not.toHaveBeenCalled();
  });

  it('serialises commits in call order', async () => {
    const order: string[] = [];
    let releaseFirst: (() => void) | null = null;
    const commit = vi.fn(async (value: number) => {
      if (value === 1) {
        await new Promise<void>((resolve) => {
          releaseFirst = resolve;
        });
      }
      order.push(`done-${value}`);
    });
    const debounced = debounceCommit(commit, 10);

    debounced(1);
    await vi.advanceTimersByTimeAsync(10);
    debounced(2);
    await vi.advanceTimersByTimeAsync(10);

    // The second commit must wait for the first to settle.
    expect(order).toEqual([]);
    releaseFirst!();
    await debounced.flush();
    expect(order).toEqual(['done-1', 'done-2']);
  });

  it('keeps later commits flowing after one fails', async () => {
    const commit = vi
      .fn<(value: number) => Promise<void>>()
      .mockRejectedValueOnce(new Error('boom'))
      .mockResolvedValue(undefined);
    const debounced = debounceCommit(commit, 10);

    debounced(1);
    await vi.advanceTimersByTimeAsync(10);
    await debounced.flush();

    debounced(2);
    await vi.advanceTimersByTimeAsync(10);

    expect(commit).toHaveBeenCalledTimes(2);
    expect(commit).toHaveBeenLastCalledWith(2);
  });
});
