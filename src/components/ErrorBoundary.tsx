import { Component, ErrorInfo, ReactNode } from 'react';

interface ErrorBoundaryProps {
  children: ReactNode;
}

interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
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

  render(): ReactNode {
    if (this.state.hasError) {
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
          <h1 style={{ fontSize: 20, fontWeight: 700, marginBottom: 12 }}>Something went wrong</h1>
          <p
            style={{
              fontSize: 14,
              color: 'var(--color-text-muted, #a6adc8)',
              marginBottom: 20,
              textAlign: 'center',
            }}
          >
            An unexpected error occurred. Try reloading the application.
          </p>
          {this.state.error && (
            <pre
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
            Reload
          </button>
        </div>
      );
    }

    return this.props.children;
  }
}
