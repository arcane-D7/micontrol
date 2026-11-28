import React, { createContext, useCallback, useContext, useRef, useState } from 'react';
import { t as i18n } from '../hooks/useI18n';
import { Icon } from '../components/ui/icons';

// ── Types ─────────────────────────────────────────────────────────────────────

export type ToastType = 'error' | 'success' | 'info';

export interface ToastOptions {
  message: string;
  type?: ToastType;
  onRetry?: () => void;
  duration?: number;
}

interface ToastItem {
  id: number;
  message: string;
  type: ToastType;
  onRetry?: () => void;
}

interface ToastContextValue {
  addToast: (message: string | ToastOptions, type?: ToastType) => void;
}

// ── Context ───────────────────────────────────────────────────────────────────

const ToastContext = createContext<ToastContextValue>({ addToast: () => {} });

export function useToast(): ToastContextValue {
  return useContext(ToastContext);
}

// ── Provider + renderer ───────────────────────────────────────────────────────

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const nextId = useRef(0);

  const addToast = useCallback(
    (messageOrOpts: string | ToastOptions, type: ToastType = 'error') => {
      let message: string;
      let toastType: ToastType = type;
      let onRetry: (() => void) | undefined;
      let duration = 5000;

      if (typeof messageOrOpts === 'object') {
        message = messageOrOpts.message;
        toastType = messageOrOpts.type ?? type;
        onRetry = messageOrOpts.onRetry;
        duration = messageOrOpts.duration ?? 5000;
      } else {
        message = messageOrOpts;
      }

      const id = ++nextId.current;
      setToasts((prev) => [...prev, { id, message, type: toastType, onRetry }]);
      setTimeout(() => {
        setToasts((prev) => prev.filter((t) => t.id !== id));
      }, duration);
    },
    [],
  );

  const dismiss = useCallback((id: number) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  return (
    <ToastContext.Provider value={{ addToast }}>
      {children}

      {/* Toast container — fixed bottom-right */}
      <div className="toast-container" role="log" aria-live="polite">
        {toasts.map((t) => (
          <div key={t.id} className={`toast toast--${t.type}`}>
            <span className="toast__icon">
              {t.type === 'error' ? (
                <Icon name="alert-triangle" size={16} />
              ) : t.type === 'success' ? (
                <Icon name="check" size={16} />
              ) : (
                <Icon name="info" size={16} />
              )}
            </span>
            <span className="toast__message">{t.message}</span>
            {t.onRetry && (
              <button
                className="toast-retry-btn"
                onClick={() => {
                  t.onRetry!();
                  dismiss(t.id);
                }}
              >
                {i18n('common.retry')}
              </button>
            )}
            <button
              className="toast__close"
              onClick={() => dismiss(t.id)}
              aria-label={i18n('common.dismiss')}
            >
              ×
            </button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}
