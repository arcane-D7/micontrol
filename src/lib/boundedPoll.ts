/**
 * runBounded — run N async legs with a concurrency cap, jitter and
 * cancellation.
 *
 * Why: the hardware slow poll used to fire five Tauri invokes at the same
 * instant every 15 s (`Promise.all`). Each one competes for the backend's
 * fixed blocking pool; when the battery WMI provider stalls, all five land on
 * the pool together and the registry-only legs (touchpad) time out behind
 * them. The measured effect (STAB-ROUND-1): `get_touchpad_info` timed out at
 * 5 s five times, `get_battery_info` at 15 s four times, and the Tokio runtime
 * starved.
 *
 * Rules:
 *  - at most `limit` legs in flight;
 *  - the first leg starts immediately, every later one after a random
 *    0..jitterMs delay, so a five-leg batch is spread over one poll tick
 *    instead of one instant;
 *  - `isCancelled` is checked before a queued leg starts and again after the
 *    jitter sleep (guarantees no new work after unmount);
 *  - a rejected leg does not abort the others: every task yields an outcome,
 *    with `skipped` for the ones cancellation prevented.
 *
 * Kept as a plain function (no React) so it is testable with fake timers.
 */

export type BoundedOutcome<T> =
  | { status: 'fulfilled'; value: T }
  | { status: 'rejected'; reason: unknown }
  | { status: 'skipped' };

export interface BoundedOptions {
  /** Maximum number of tasks running at once (>= 1). */
  limit: number;
  /** Random delay applied before every task except the first. */
  jitterMs?: number;
  /** When it returns true, queued tasks are skipped and no new ones start. */
  isCancelled?: () => boolean;
  /** Injection points for tests. */
  random?: () => number;
  sleep?: (ms: number) => Promise<void>;
}

const defaultSleep = (ms: number): Promise<void> =>
  new Promise((resolve) => {
    setTimeout(resolve, ms);
  });

export async function runBounded<T>(
  tasks: ReadonlyArray<() => Promise<T>>,
  options: BoundedOptions,
): Promise<BoundedOutcome<T>[]> {
  const limit = Math.max(1, Math.floor(options.limit) || 1);
  const jitterMs = Math.max(0, options.jitterMs ?? 0);
  const random = options.random ?? Math.random;
  const sleep = options.sleep ?? defaultSleep;
  const isCancelled = options.isCancelled ?? (() => false);

  const outcomes: BoundedOutcome<T>[] = tasks.map(() => ({ status: 'skipped' }));
  let next = 0;

  const worker = async () => {
    for (;;) {
      if (isCancelled()) return;
      const index = next++;
      if (index >= tasks.length) return;

      // Stagger everything after the first leg. `index > 0` (not worker id):
      // the point is one wave, not one burst per worker.
      if (index > 0 && jitterMs > 0) {
        const delay = Math.floor(random() * jitterMs);
        if (delay > 0) {
          await sleep(delay);
        }
        if (isCancelled()) return;
      }

      try {
        outcomes[index] = { status: 'fulfilled', value: await tasks[index]() };
      } catch (reason) {
        outcomes[index] = { status: 'rejected', reason };
      }
    }
  };

  const workers = Math.min(limit, tasks.length);
  await Promise.all(Array.from({ length: workers }, () => worker()));
  return outcomes;
}
