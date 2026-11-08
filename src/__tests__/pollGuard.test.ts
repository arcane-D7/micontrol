import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { createPollGuard } from '../lib/pollGuard';

/**
 * PollGuard — o remédio para o H-04 do dogfood: `get_fan_info` excedia os 6 s
 * em cada poll e o poll de 2 s empilhava chamadas (2 221 abandonadas em 74
 * min). Os testes fixam: (1) uma em voo, (2) backoff após timeout,
 * (3) reset quando a leitura volta.
 */
describe('createPollGuard', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('não emite segunda chamada enquanto a primeira está pendente', async () => {
    const guard = createPollGuard({ timeoutMs: 1000 });
    let release: (v: string) => void = () => {};
    const pending = new Promise<string>((resolve) => {
      release = resolve;
    });

    const first = guard.run(() => pending);
    const second = await guard.run(() => Promise.resolve('outra'));

    expect(second).toEqual({ status: 'skipped' });
    expect(guard.inFlight).toBe(true);

    release('primeira');
    await expect(first).resolves.toEqual({ status: 'ok', value: 'primeira' });
    expect(guard.inFlight).toBe(false);
  });

  it('abandona o resultado ao fim de timeoutMs mas mantém a chamada como em voo', async () => {
    const guard = createPollGuard({ timeoutMs: 6000 });
    const onTimeout = vi.fn();
    const withCallback = createPollGuard({ timeoutMs: 6000, onTimeout });

    const never = new Promise<string>(() => {});
    const outcome = guard.run(() => never);
    await vi.advanceTimersByTimeAsync(6000);

    await expect(outcome).resolves.toEqual({
      status: 'timeout',
      streak: 1,
      retryInMs: 6000,
    });
    // A promessa original continua viva — o guard sabe disso e não deixa
    // outra leitura empilhar-se em cima.
    expect(guard.inFlight).toBe(true);

    void withCallback.run(() => new Promise<string>(() => {}));
    await vi.advanceTimersByTimeAsync(6000);
    expect(onTimeout).toHaveBeenCalledWith({ streak: 1, retryInMs: 6000 });
  });

  it('faz backoff exponencial e volta a deixar ler quando o intervalo passa', async () => {
    const guard = createPollGuard({ timeoutMs: 100, baseBackoffMs: 100, maxBackoffMs: 400 });

    let release: (v: string) => void = () => {};
    const slow = new Promise<string>((resolve) => {
      release = resolve;
    });
    const first = guard.run(() => slow);
    await vi.advanceTimersByTimeAsync(100);
    const timedOut = await first;
    expect(timedOut.status).toBe('timeout');

    // A chamada original ainda está pendente → skipped.
    expect((await guard.run(() => Promise.resolve('x'))).status).toBe('skipped');

    // Assenta finalmente: o flag limpa-se, mas o backoff continua a mandar.
    release('tarde');
    await Promise.resolve();
    await Promise.resolve();
    expect(guard.inFlight).toBe(false);
    expect((await guard.run(() => Promise.resolve('x'))).status).toBe('skipped');

    // Passado o backoff (100 ms), a leitura volta.
    await vi.advanceTimersByTimeAsync(100);
    await expect(guard.run(() => Promise.resolve('y'))).resolves.toEqual({
      status: 'ok',
      value: 'y',
    });
  });

  it('duplica o backoff a cada timeout consecutivo e repõe-no num sucesso', async () => {
    const g2 = createPollGuard({ timeoutMs: 100, baseBackoffMs: 100, maxBackoffMs: 400 });
    const step = async (g: ReturnType<typeof createPollGuard>) => {
      let release: (v: string) => void = () => {};
      const p = new Promise<string>((resolve) => {
        release = resolve;
      });
      const run = g.run(() => p);
      await vi.advanceTimersByTimeAsync(100);
      const res = await run;
      release('tarde');
      await Promise.resolve();
      await Promise.resolve();
      await vi.advanceTimersByTimeAsync(res.status === 'timeout' ? res.retryInMs : 0);
      return res;
    };

    const a = await step(g2);
    const b = await step(g2);
    const c = await step(g2);
    expect(a).toMatchObject({ streak: 1, retryInMs: 100 });
    expect(b).toMatchObject({ streak: 2, retryInMs: 200 });
    expect(c).toMatchObject({ streak: 3, retryInMs: 400 });
  });

  it('um sucesso repõe o streak e o intervalo', async () => {
    const guard = createPollGuard({ timeoutMs: 100, baseBackoffMs: 100 });
    let release: (v: string) => void = () => {};
    const slow = new Promise<string>((resolve) => {
      release = resolve;
    });
    const p = guard.run(() => slow);
    await vi.advanceTimersByTimeAsync(100);
    await p;
    expect(guard.streak).toBe(1);

    // A chamada abandonada assenta → deixa de estar em voo, mas o backoff manda.
    release('tarde');
    await Promise.resolve();
    await Promise.resolve();
    expect(guard.inFlight).toBe(false);

    // Fora da janela de backoff, uma leitura saudável repõe tudo.
    await vi.advanceTimersByTimeAsync(100);
    await guard.run(() => Promise.resolve('ok'));
    expect(guard.streak).toBe(0);
  });
});
