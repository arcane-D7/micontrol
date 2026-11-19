/**
 * System log — o sítio próprio dos erros (a sidebar NÃO é um log).
 *
 * Substitui o banner vermelho permanente da sidebar: o botão discreto do
 * rodapé da sidebar (`IconButton` 32 px, glifo `alert-triangle`, rótulo
 * "System log") abre este painel com as últimas entradas de erro — o
 * `hardware.error` vivo do hook de hardware + o `errors.log` real escrito
 * pelo backend, lido pelo comando existente `read_error_log` (formato
 * `{rfc3339} [{target}] ERROR: {message}`, retenção de 7 dias).
 *
 * Cada linha tem timestamp, severidade e um botão Copy. Sem emojis, sem
 * texto de erro permanente na navegação — o estado na sidebar é um botão de
 * 32 px em tinta neutra que só ganha a tinta de aviso quando há um erro vivo.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getLanguage, t } from '../hooks/useI18n';
import { Dialog } from './ui/dialog';
import { Icon } from './ui/icons';
import { IconButton } from './ui/icon-button';
import { Badge } from './ui/badge';

interface LogEntry {
  /** RFC-3339 do backend; `null` na entrada viva (é "agora"). */
  ts: string | null;
  severity: string;
  message: string;
}

export interface SystemLogPanelProps {
  open: boolean;
  onClose: () => void;
  /** `hardware.error` — o erro vivo do hook, sempre a entrada mais recente. */
  currentError?: string | null;
}

/** `{rfc3339} [{target}] ERROR: {message}` — ver `util/error_log.rs`. */
const LINE_RE = /^(\S+)\s+\[([^\]]+)\]\s+([A-Z]+):\s(.*)$/;
const MAX_LINES = 100;

/** Parse do texto do `errors.log`; linhas irreconhecíveis entram como texto cru. */
export function parseErrorLog(raw: string): LogEntry[] {
  return raw
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const match = line.match(LINE_RE);
      if (!match) return { ts: null, severity: 'ERROR', message: line };
      return { ts: match[1], severity: match[3], message: match[4] };
    })
    .reverse(); // mais recente primeiro
}

/** O erro vivo entra no topo — a menos que o log já o tenha (sem duplicado). */
export function mergeCurrentError(entries: LogEntry[], currentError: string | null): LogEntry[] {
  if (!currentError) return entries;
  if (entries.some((e) => e.message === currentError)) return entries;
  return [{ ts: null, severity: 'ERROR', message: currentError }, ...entries];
}

async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // Fallback para contextos sem Clipboard API (o fallback também pode
    // falhar — o estado "Copiado" só aparece quando a cópia não rejeitou).
    const area = document.createElement('textarea');
    area.value = text;
    area.setAttribute('readonly', '');
    area.style.position = 'fixed';
    area.style.opacity = '0';
    document.body.appendChild(area);
    area.select();
    try {
      document.execCommand('copy');
    } finally {
      area.remove();
    }
  }
}

/** RFC-3339 → data+hora no locale activo; sem timestamp → "Agora". */
function fmtEntryTime(ts: string | null): string {
  if (!ts) return t('systemLog.now');
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return ts;
  try {
    return new Intl.DateTimeFormat(getLanguage(), {
      dateStyle: 'short',
      timeStyle: 'medium',
    }).format(date);
  } catch {
    return ts;
  }
}

function severityTone(severity: string): 'danger' | 'warning' | 'neutral' {
  if (severity === 'ERROR') return 'danger';
  if (severity === 'WARN') return 'warning';
  return 'neutral';
}

export function SystemLogPanel({ open, onClose, currentError }: SystemLogPanelProps) {
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null);
  const copiedTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Lê o log real quando o painel abre (e re Lê se o erro vivo mudar).
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    void (async () => {
      let parsed: LogEntry[] = [];
      try {
        const raw = await invoke<string>('read_error_log', { maxLines: MAX_LINES });
        parsed = parseErrorLog(raw ?? '');
      } catch (e) {
        // O comando pode falhar (log indisponível); o erro vivo continua visível.
        console.warn('[systemLog] read_error_log failed:', e);
      }
      if (!cancelled) setEntries(mergeCurrentError(parsed, currentError ?? null));
    })();
    return () => {
      cancelled = true;
    };
  }, [open, currentError]);

  // Sem marcação pendente depois do unmount.
  useEffect(
    () => () => {
      if (copiedTimer.current !== null) clearTimeout(copiedTimer.current);
    },
    [],
  );

  const handleCopy = useCallback((entry: LogEntry, index: number) => {
    const text = entry.ts ? `${entry.ts} [${entry.severity}] ${entry.message}` : entry.message;
    void copyText(text).then(() => {
      setCopiedIndex(index);
      if (copiedTimer.current !== null) clearTimeout(copiedTimer.current);
      copiedTimer.current = setTimeout(() => setCopiedIndex(null), 1500);
    });
  }, []);

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={
        <>
          <Icon name="alert-triangle" size={16} /> {t('systemLog.title')}
        </>
      }
      size="lg"
      closeLabel={t('common.close')}
    >
      {entries.length === 0 ? (
        <p className="mc-note" role="status">
          {t('systemLog.empty')}
        </p>
      ) : (
        <ul className="mc-loglist">
          {entries.map((entry, index) => (
            <li className="mc-logrow" key={`${entry.ts ?? 'now'}-${index}`}>
              <span className="mc-logrow__time">{fmtEntryTime(entry.ts)}</span>
              <Badge tone={severityTone(entry.severity)}>{entry.severity}</Badge>
              <span className="mc-logrow__msg">{entry.message}</span>
              <span className="mc-logrow__action">
                <IconButton
                  icon="clipboard"
                  label={copiedIndex === index ? t('systemLog.copied') : t('systemLog.copy')}
                  onClick={() => handleCopy(entry, index)}
                />
              </span>
            </li>
          ))}
        </ul>
      )}
    </Dialog>
  );
}

export default SystemLogPanel;
