import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { t } from '../hooks/useI18n';

export interface AiUsageStats {
  total_requests: number;
  total_input_tokens: number;
  total_output_tokens: number;
  estimated_cost_usd: number;
}

/**
 * AI usage counters.
 *
 * The numbers are real (they are written by `analyze_system`), but with no API
 * key and no analysis ever run they are a wall of honest zeros that reads like
 * placeholder data — and a "Reset" button for counters that never moved. Show
 * the state instead of the numbers until there is something to report.
 */
export default function AiUsagePanel({
  configured,
  onOpenSettings,
}: {
  configured: boolean;
  onOpenSettings: () => void;
}) {
  const [usage, setUsage] = useState<AiUsageStats | null>(null);

  useEffect(() => {
    if (!configured) {
      setUsage(null);
      return;
    }
    invoke<AiUsageStats>('get_ai_usage')
      .then(setUsage)
      .catch(() => {
        /* ignore */
      });
  }, [configured]);

  const handleReset = async () => {
    await invoke('reset_ai_usage');
    const updated = await invoke<AiUsageStats>('get_ai_usage');
    setUsage(updated);
  };

  if (!configured) {
    return (
      <div className="card ai-usage-panel">
        <div className="card-title ai-usage-panel__title">{t('ai.usage.title')}</div>
        <p className="text-muted" style={{ margin: '4px 0' }}>
          {t('ai.notConfigured')} {t('ai.configureLink')}
        </p>
        <button onClick={onOpenSettings} className="ai-usage-panel__reset">
          {t('ai.usage.openSettings')}
        </button>
      </div>
    );
  }

  if (!usage) return null;

  const hasRequests = usage.total_requests > 0;

  return (
    <div className="card ai-usage-panel">
      <div className="ai-usage-panel__header">
        <div>
          <div className="card-title ai-usage-panel__title">{t('ai.usage.title')}</div>
          {hasRequests ? (
            <div className="ai-usage-panel__stats">
              {t('ai.usage.requests')}: <strong>{usage.total_requests}</strong>
              {' · '}
              {t('ai.usage.tokens')}:{' '}
              <strong>{usage.total_input_tokens + usage.total_output_tokens}</strong>
              {' · '}
              {t('ai.usage.estimatedCost')}: <strong>${usage.estimated_cost_usd.toFixed(4)}</strong>
            </div>
          ) : (
            <p className="text-muted" style={{ margin: 0 }}>
              {t('ai.usage.noRequests')}
            </p>
          )}
        </div>
        {hasRequests && (
          <button onClick={handleReset} className="ai-usage-panel__reset">
            {t('ai.usage.reset')}
          </button>
        )}
      </div>
    </div>
  );
}
