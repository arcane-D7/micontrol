import { useState } from 'react';
import { t } from '../hooks/useI18n';
import type { useHardware } from '../hooks/useHardware';
import type { useSettings as UseSettings } from '../hooks/useSettings';

type Hardware = ReturnType<typeof useHardware>;
type Settings = ReturnType<typeof UseSettings>;

interface Props {
  hw: Hardware;
  ai: Settings;
  /** Called when user clicks "Configure API" link */
  onOpenSettings: () => void;
}

/** Converts markdown-ish bullet text to simple JSX */
function RenderAnalysis({ text }: { text: string }) {
  const lines = text.split('\n').filter((l) => l.trim());
  return (
    <div style={{ lineHeight: 1.7, fontSize: 13 }}>
      {lines.map((line, i) => {
        const clean = line.replace(/^[-*•]\s*/, '').trim();
        const isBullet = /^[-*•]/.test(line.trim()) || /^\d+\./.test(line.trim());
        const isHeading = line.trim().startsWith('**') || /^#{1,3}\s/.test(line.trim());
        const cleanHeading = clean.replace(/\*\*/g, '').replace(/^#+\s*/, '');
        if (isHeading) {
          return (
            <div key={i} style={{ fontWeight: 700, marginTop: i > 0 ? 12 : 0, marginBottom: 2 }}>
              {cleanHeading}
            </div>
          );
        }
        if (isBullet) {
          return (
            <div key={i} style={{ display: 'flex', gap: 8, marginLeft: 4 }}>
              <span style={{ color: 'var(--color-accent)', flexShrink: 0 }}>›</span>
              <span>{clean.replace(/\*\*/g, '')}</span>
            </div>
          );
        }
        return (
          <div key={i} style={{ marginTop: 4 }}>
            {clean.replace(/\*\*/g, '')}
          </div>
        );
      })}
    </div>
  );
}

export default function AiAdvisor({ hw, ai, onOpenSettings }: Props) {
  const [status, setStatus] = useState<'idle' | 'loading' | 'done' | 'error'>('idle');
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(true);

  async function handleAnalyze() {
    setStatus('loading');
    setResult(null);
    setError(null);
    try {
      const text = await ai.analyzeSystem({
        deviceModel: hw.hardwareProfile?.device_model ?? null,
        systemInfo: hw.systemInfo,
        battery: hw.battery,
        performanceMode: hw.performanceMode,
        fan: hw.fan,
        display: hw.display,
        capabilities: hw.hardwareProfile?.capabilities ?? null,
      });
      setResult(text);
      setStatus('done');
    } catch (e) {
      const msg = String(e).replace(/^Error:\s*/, '');
      setError(msg === 'api_key_missing' ? t('settings.apiKeyMissing') : msg);
      setStatus('error');
    }
  }

  return (
    <div className="card">
      {/* Header */}
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          cursor: 'pointer',
        }}
        onClick={() => setExpanded((v) => !v)}
      >
        <div
          className="card-title"
          style={{ marginBottom: 0, display: 'flex', alignItems: 'center', gap: 8 }}
        >
          🤖 {t('ai.title')}
        </div>
        <span style={{ color: 'var(--color-text-muted)', fontSize: 12 }}>
          {expanded ? '▲' : '▼'}
        </span>
      </div>

      {expanded && (
        <div style={{ marginTop: 12 }}>
          {!ai.isConfigured ? (
            /* Not configured yet */
            <div style={{ fontSize: 13, color: 'var(--color-text-muted)' }}>
              {t('ai.notConfigured')}{' '}
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  onOpenSettings();
                }}
                className="link-btn"
              >
                {t('ai.configureLink')}
              </button>
            </div>
          ) : (
            <>
              {/* Subtitle */}
              <p
                style={{
                  fontSize: 12,
                  color: 'var(--color-text-muted)',
                  marginBottom: 14,
                  marginTop: 0,
                }}
              >
                {t('ai.subtitle')}
              </p>

              {/* Analyze button */}
              <button
                className="btn-primary"
                disabled={status === 'loading'}
                onClick={() => void handleAnalyze()}
              >
                {status === 'loading' ? <>⏳ {t('ai.analyzing')}</> : <>🔍 {t('ai.analyzeBtn')}</>}
              </button>

              {/* Loading spinner */}
              {status === 'loading' && (
                <div style={{ fontSize: 12, color: 'var(--color-text-muted)', marginTop: 4 }}>
                  {t('ai.analyzingHint')}
                </div>
              )}

              {/* Error */}
              {status === 'error' && error && (
                <div
                  style={{
                    padding: '8px 12px',
                    borderRadius: 6,
                    fontSize: 12,
                    background: 'rgba(248,113,113,0.1)',
                    color: 'var(--color-danger, #f87171)',
                    border: '1px solid rgba(248,113,113,0.3)',
                  }}
                >
                  ✗ {error}
                </div>
              )}

              {/* Result */}
              {status === 'done' && result && (
                <div
                  style={{
                    marginTop: 4,
                    padding: '12px 14px',
                    borderRadius: 8,
                    background: 'var(--color-surface-raised, rgba(255,255,255,0.04))',
                    border: '1px solid var(--color-border)',
                  }}
                >
                  <div
                    style={{
                      fontSize: 11,
                      color: 'var(--color-text-muted)',
                      marginBottom: 8,
                      display: 'flex',
                      justifyContent: 'space-between',
                    }}
                  >
                    <span>🤖 {ai.settings.openai_model}</span>
                    <button
                      className="btn-inline"
                      onClick={() => {
                        setStatus('idle');
                        setResult(null);
                      }}
                    >
                      ✕ {t('ai.clear')}
                    </button>
                  </div>
                  <RenderAnalysis text={result} />
                </div>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}
