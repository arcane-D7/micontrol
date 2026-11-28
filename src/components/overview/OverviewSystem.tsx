/**
 * Estado do sistema — Windows, uptime, perfil de energia, OS Turbo, serviços.
 *
 * É o "em que estado está isto agora" do resumo. O perfil de energia aparece
 * como leitura (a alteração vive na tab Desempenho, a um clique pelos atalhos
 * do fim da página): um resumo não deve ter controlos que mudam hardware.
 */

import { t, type StringKey } from '../../hooks/useI18n';
import type { OverviewSnapshot, PerformanceMode } from '../../types/hardware';
import type { OsTurboSummary, ServiceHealthSummary } from '../../hooks/useOverview';
import { Badge } from '../ui/badge';
import { Card } from '../ui/card';
import { Icon } from '../ui/icons';
import { StatGroup, StatRow } from '../ui/stat-row';
import { EMPTY, fmtText, fmtUptime } from './format';

interface Props {
  snapshot: OverviewSnapshot | null;
  perfMode: PerformanceMode;
  osTurbo: OsTurboSummary | null;
  services: ServiceHealthSummary | null;
  loading: boolean;
}

/** `performance.modes.*` é o sítio canónico dos nomes dos perfis. */
function perfLabel(mode: PerformanceMode): string {
  return t(`performance.modes.${mode}` as StringKey);
}

function servicesHealthy(services: ServiceHealthSummary | null): boolean | null {
  if (!services) return null;
  const states = [services.bridge, services.iot, services.face, services.ambientSensor]
    .filter((s): s is { state: string } => s != null)
    .map((s) => s.state);
  if (states.length === 0) return null;
  return states.every((s) => s === 'healthy');
}

export function OverviewSystem({ snapshot, perfMode, osTurbo, services, loading }: Props) {
  const os = snapshot?.os;
  const healthy = servicesHealthy(services);

  return (
    <Card
      title={t('overview.state.title')}
      icon={<Icon name="gear" size={16} />}
      loading={loading}
      meta={
        healthy == null ? undefined : healthy ? (
          <Badge tone="success" dot>
            {t('overview.state.servicesOk')}
          </Badge>
        ) : (
          <Badge tone="warning" dot>
            {t('overview.state.servicesDegraded')}
          </Badge>
        )
      }
    >
      <div className="mc-group">
        <p className="mc-lead">
          {fmtText(os?.caption)}
          {os?.build && (
            <span className="mc-lead__sub">
              {t('overview.state.build')} {os.build} · {fmtText(os?.arch)}
            </span>
          )}
        </p>
        <StatGroup>
          <StatRow
            label={t('overview.state.uptime')}
            value={fmtUptime(os?.uptime_secs)}
            hint={t('overview.state.uptimeHint')}
          />
        </StatGroup>
      </div>

      <div className="mc-group">
        <h3 className="mc-group__title">{t('overview.state.power')}</h3>
        <StatGroup>
          <StatRow label={t('overview.state.profile')} value={perfLabel(perfMode)} />
          <StatRow
            label={t('overview.state.osTurbo')}
            value={
              osTurbo == null ? (
                EMPTY
              ) : osTurbo.enabled ? (
                <Badge tone="success" dot>
                  {t('overview.state.osTurboOn')}
                </Badge>
              ) : (
                <Badge tone="neutral" dot>
                  {t('overview.state.osTurboOff')}
                </Badge>
              )
            }
            empty={osTurbo == null}
            hint={
              osTurbo?.enabled
                ? t('overview.state.osTurboCount', { count: osTurbo.throttled_processes })
                : undefined
            }
          />
          <StatRow label={t('overview.state.powerPlan')} value={fmtText(osTurbo?.power_plan)} />
        </StatGroup>
      </div>
    </Card>
  );
}

export default OverviewSystem;
