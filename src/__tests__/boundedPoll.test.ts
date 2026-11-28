import { describe, it, expect } from 'vitest';
import { runBounded } from '../lib/boundedPoll';

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

describe('runBounded', () => {
  it('never exceeds the concurrency limit and keeps result order', async () => {
    const gates = Array.from({ length: 5 }, () => deferred<number>());
    let active = 0;
    let maxActive = 0;

    const tasks = gates.map((gate, index) => async () => {
      active += 1;
      maxActive = Math.max(maxActive, active);
      const value = await gate.promise;
      active -= 1;
      return value + index * 0; // value identifies the leg
    });

    const run = runBounded(tasks, { limit: 2 });
    await tick();
    expect(active).toBe(2);

    // Release one at a time: the freed slot must admit the next leg, never a
    // third concurrent one.
    for (let i = 0; i < gates.length; i += 1) {
      gates[i].resolve(i);
      await tick();
      expect(active).toBeLessThanOrEqual(2);
    }

    const results = await run;
    expect(maxActive).toBe(2);
    expect(results.map((r) => (r.status === 'fulfilled' ? r.value : null))).toEqual([
      0, 1, 2, 3, 4,
    ]);
    expect(results.every((r) => r.status === 'fulfilled')).toBe(true);
  });

  it('stops starting queued legs once cancelled', async () => {
    let cancelled = false;
    let started = 0;
    const tasks = Array.from({ length: 4 }, (_, index) => async () => {
      started += 1;
      if (index === 0) cancelled = true;
      return index;
    });

    const results = await runBounded(tasks, { limit: 2, isCancelled: () => cancelled });

    expect(started).toBe(1);
    expect(results[0]).toEqual({ status: 'fulfilled', value: 0 });
    expect(results.slice(1).map((r) => r.status)).toEqual(['skipped', 'skipped', 'skipped']);
  });

  it('does not let one rejected leg abort the others', async () => {
    const results = await runBounded(
      [
        async () => {
          throw new Error('wmi wedged');
        },
        async () => 2,
      ],
      { limit: 2 },
    );

    expect(results[0].status).toBe('rejected');
    expect(results[1]).toEqual({ status: 'fulfilled', value: 2 });
  });

  it('staggered starts use the injected jitter', async () => {
    const sleeps: number[] = [];
    await runBounded([async () => 0, async () => 1, async () => 2], {
      limit: 1,
      jitterMs: 100,
      random: () => 0.5,
      sleep: async (ms) => {
        sleeps.push(ms);
      },
    });

    // The first leg is immediate; the two later ones wait 0.5 × 100 ms.
    expect(sleeps).toEqual([50, 50]);
  });
});
