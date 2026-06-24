import { Component, ErrorInfo, ReactNode } from 'react';
import en from '../i18n/en.json';

const APP_VERSION = '1.0.0';

interface ErrorBoundaryProps {
  children: ReactNode;
}

interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
}

function getNestedValue(obj: Record<string, unknown>, path: string): string {
  const parts = path.split('.');
  let current: unknown = obj;
  for (const part of parts) {
    if (current == null || typeof current !== 'object') return '';
    current = (current as Record<string, unknown>)[part];
  }
  return typeof current === 'string' ? current : '';
}

function getLocaleStrings() {
  const strings = en as Record<string, unknown>;
  return {
    title: getNestedValue(strings, 'error.boundary.title') || 'Something went wrong',
    message:
      getNestedValue(strings, 'error.boundary.message') ||
      'An unexpected error occurred. Try reloading the application.',
    reload: getNestedValue(strings, 'error.boundary.reload') || 'Reload',
    report: getNestedValue(strings, 'error.boundary.report') || 'Report Issue',
  };
}

export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo): void {
    console.error('ErrorBoundary caught an error:', error, errorInfo);
  }

  handleReload = (): void => {
    window.location.reload();
  };

  handleReportIssue = (): void => {
    const { error } = this.state;
    const title = encodeURIComponent(`[Bug] Application crash: ${error?.name || 'Unknown'}`);
    const body = encodeURIComponent(
      `## Error\n\n\`\`\`\n${error?.stack || error?.message || 'Unknown error'}\n\`\`\`\n\n` +
        `## Environment\n\n- Version: ${APP_VERSION}\n- OS: ${navigator.userAgent}\n- Timestamp: ${new Date().toISOString()}\n`,
    );
    window.open(
      `https://github.com/Freitas-MA/miPC/issues/new?title=${title}&body=${body}`,
      '_blank',
    );
  };

  render(): ReactNode {
    if (this.state.hasError) {
      const t = getLocaleStrings();
      return (
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            justifyContent: 'center',
            minHeight: '100vh',
            padding: '32px',
            background: 'var(--color-bg, #181825)',
            color: 'var(--color-text, #cdd6f4)',
            fontFamily: 'system-ui, -apple-system, sans-serif',
          }}
        >
          <h1 style={{ fontSize: 20, fontWeight: 700, marginBottom: 12 }}>{t.title}</h1>
          <p
            style={{
              fontSize: 14,
              color: 'var(--color-text-muted, #a6adc8)',
              marginBottom: 20,
              textAlign: 'center',
            }}
          >
            {t.message}
          </p>
          {this.state.error && (
            <pre
              role="alert"
              style={{
                fontSize: 12,
                color: 'var(--color-text-muted, #a6adc8)',
                background: 'var(--color-surface, #1e1e2e)',
                padding: 12,
                borderRadius: 8,
                maxWidth: 600,
                overflow: 'auto',
                marginBottom: 20,
              }}
            >
              {this.state.error.message}
            </pre>
          )}
          <button
            type="button"
            onClick={this.handleReload}
            style={{
              padding: '10px 24px',
              fontSize: 14,
              fontWeight: 600,
              border: 'none',
              borderRadius: 8,
              cursor: 'pointer',
              background: 'var(--color-accent, #6c8cff)',
              color: '#fff',
            }}
          >
            {t.reload}
          </button>
          <button
            type="button"
            onClick={this.handleReportIssue}
            style={{
              padding: '10px 24px',
              fontSize: 14,
              fontWeight: 600,
              border: '1px solid var(--color-border, #3a3a4e)',
              borderRadius: 8,
              cursor: 'pointer',
              background: 'transparent',
              color: 'var(--color-text, #cdd6f4)',
            }}
          >
            {t.report}
          </button>
        </div>
      );
    }

    return this.props.children;
  }
}
