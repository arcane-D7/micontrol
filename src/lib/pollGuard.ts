/**
 * PollGuard — "skip-if-pending" + backoff exponencial para uma leitura lenta.
 *
 * Porque existe: `get_fan_info` pode demorar mais do que o intervalo do poll
 * (2 s). O `Promise.race` antigo abandonava o *resultado* mas não a chamada —
 * a cada 2 s nascia outra leitura que ficava pendente. O dogfood mediu 2 221
 * chamadas abandonadas em 74 min (e 2 221 avisos no log).
 *
 * Regras:
 *  - **uma em voo**: enquanto a chamada anterior não assentar (mesmo depois de
 *    abandonada), `run()` devolve `skipped` e não emite nada;
 *  - **backoff**: um timeout multiplica o intervalo até `maxBackoffMs`;
 *    um resultado (mesmo `null` vindo de um erro do comando) repõe o intervalo;
 *  - **sem timers pendentes**: o timer do timeout é sempre limpo.
 *
 * É uma fábrica de estado (não um hook), para ser testável com fake timers.
 */

export type PollOutcome<T> =
  | { status: 'ok'; value: T }
  | { status: 'skipped' }
  | { status: 'timeout'; streak: number; retryInMs: number };

export interface PollGuardOptions {
  /** Tempo máximo à espera de uma leitura antes de a abandonar. */
  timeoutMs: number;
  baseBackoffMs?: number;
  maxBackoffMs?: number;
  /** Chamado uma vez por timeout — logging do chamador, não do guard. */
  onTimeout?: (info: { streak: number; retryInMs: number }) => void;
  /** Injeção de relógio (testes). */
  now?: () => number;
}

export interface PollGuard {
  run<T>(task: () => Promise<T>): Promise<PollOutcome<T>>;
  /** Verdadeiro enquanto a última chamada real ainda não assentou. */
  readonly inFlight: boolean;
  /** Nº de timeouts consecutivos. */
  readonly streak: number;
}

export function createPollGuard(options: PollGuardOptions): PollGuard {
  const {
    timeoutMs,
    baseBackoffMs = timeoutMs,
    maxBackoffMs = 60_000,
    onTimeout,
    now = () => Date.now(),
  } = options;

  let inFlight = false;
  let streak = 0;
  let nextAttemptAt = 0;

  const backoffFor = (s: number) => Math.min(maxBackoffMs, baseBackoffMs * 2 ** (s - 1));

  return {
    get inFlight() {
      return inFlight;
    },
    get streak() {
      return streak;
    },

    async run<T>(task: () => Promise<T>): Promise<PollOutcome<T>> {
      if (inFlight) return { status: 'skipped' };
      if (now() < nextAttemptAt) return { status: 'skipped' };

      inFlight = true;
      let timer: ReturnType<typeof setTimeout> | undefined;

      // A chamada real limpa o flag quando *ela* assentar — não quando a
      // corrida acaba. É esta diferença que impede o empilhamento.
      const call = task().then(
        (value) => {
          inFlight = false;
          return { value };
        },
        () => {
          inFlight = false;
          return { value: null as unknown as T };
        },
      );

      const timeout = new Promise<null>((resolve) => {
        timer = setTimeout(() => resolve(null), timeoutMs);
      });

      const raced = await Promise.race([call, timeout]);
      if (timer !== undefined) clearTimeout(timer);

      if (raced === null) {
        streak += 1;
        const retryInMs = backoffFor(streak);
        nextAttemptAt = now() + retryInMs;
        onTimeout?.({ streak, retryInMs });
        return { status: 'timeout', streak, retryInMs };
      }

      streak = 0;
      nextAttemptAt = 0;
      return { status: 'ok', value: raced.value };
    },
  };
}
