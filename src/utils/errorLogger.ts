/**
 * Frontend error logging utility.
 *
 * Sends error messages to the Rust backend's error log system,
 * which writes them to `%LOCALAPPDATA%\MiControl\logs\errors.log`
 * with 7-day retention.
 *
 * Usage:
 *   import { logError } from '../utils/errorLogger';
 *   try { ... } catch (e) { logError('myComponent', e); }
 */

import { invoke } from '@tauri-apps/api/core';

/**
 * Log an error to the backend error log file.
 * This is a fire-and-forget call — it never throws.
 */
export function logError(target: string, error: unknown): void {
  const message =
    error instanceof Error
      ? error.message
      : typeof error === 'string'
        ? error
        : JSON.stringify(error);

  // Fire and forget — don't await, don't catch
  invoke('log_frontend_error', { target, message }).catch(() => {
    // Silently ignore — error logging is best-effort
  });
}

/**
 * Log an error with additional context.
 */
export function logErrorWithContext(
  target: string,
  error: unknown,
  context: Record<string, unknown>,
): void {
  const errorMsg = error instanceof Error ? error.message : String(error);
  const contextStr = Object.entries(context)
    .map(([k, v]) => `${k}=${JSON.stringify(v)}`)
    .join(' ');
  const message = `${errorMsg} [${contextStr}]`;

  invoke('log_frontend_error', { target, message }).catch(() => {});
}
