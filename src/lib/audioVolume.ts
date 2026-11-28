/**
 * Volume-slider helpers shared by the audio surfaces.
 *
 * Two behaviours live here so they can be unit-tested without a DOM:
 *
 * 1. `resolveVolumeIntent` — Windows' own flyout unmutes the endpoint when the
 *    slider moves while muted. MiControl used to keep the endpoint muted *and*
 *    force the thumb to 0 (`value={muted ? 0 : volume}`), so dragging produced
 *    silence and the thumb snapped back on the next poll (audit P2-C).
 * 2. `debounceCommit` — the slider used to invoke `set_audio_volume` once per
 *    pixel (audit P3). A short trailing debounce plus a flush on release keeps
 *    the same perceived latency with one IPC call per gesture.
 */

/** Volume applied to the endpoint, always an integer in 0..=100. */
export interface VolumeIntent {
  volume: number;
  /** True when the endpoint must be unmuted before `volume` is applied. */
  unmute: boolean;
}

/** Round + clamp a raw `<input type="range">` value to the 0-100 contract. */
export function clampVolume(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(100, Math.max(0, Math.round(value)));
}

/**
 * Decide what a slider move means for the endpoint.
 *
 * Dragging to 0 while muted stays muted: unmuting a silent endpoint would only
 * flip the icon (and the "Muted" label) without producing any sound, which is
 * the confusing half-state the bug report came from.
 */
export function resolveVolumeIntent(nextVolume: number, muted: boolean): VolumeIntent {
  const volume = clampVolume(nextVolume);
  return { volume, unmute: muted && volume > 0 };
}

export interface DebouncedCommit<T> {
  /** Queue `value`; replaces any value still waiting for the timer. */
  (value: T): void;
  /** Commit the pending value now and wait for every queued commit. */
  flush: () => Promise<void>;
  /** Drop the pending value (in-flight commits are unaffected). */
  cancel: () => void;
  /** True while a value is waiting for the timer. */
  hasPending: () => boolean;
}

/**
 * Trailing debounce for async commits.
 *
 * `commit` is expected to handle its own errors (the audio component toasts
 * them); the returned chain never rejects, so one failed write cannot wedge
 * later ones. Commits are serialised in call order: a slow first write cannot
 * be overtaken by a later one.
 */
export function debounceCommit<T>(
  commit: (value: T) => Promise<void>,
  delayMs = 150,
): DebouncedCommit<T> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pending: { value: T } | null = null;
  let chain: Promise<void> = Promise.resolve();

  const run = (value: T): Promise<void> => {
    chain = chain.then(() => commit(value)).catch(() => undefined);
    return chain;
  };

  const debounced = ((value: T) => {
    pending = { value };
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      const queued = pending;
      pending = null;
      if (queued) void run(queued.value);
    }, delayMs);
  }) as DebouncedCommit<T>;

  debounced.flush = () => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    const queued = pending;
    pending = null;
    if (queued) return run(queued.value);
    return chain;
  };

  debounced.cancel = () => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    pending = null;
  };

  debounced.hasPending = () => pending !== null;

  return debounced;
}
