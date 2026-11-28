/**
 * Temperatura — CPU, GPU, SSD e ambiente, com a fonte à vista.
 *
 * As temperaturas de CPU/GPU vêm do `get_fan_info` (ESIF/DPTF via ponte
 * elevada), que já é lido a cada 2 s pelo `useHardware` — este cartão não abre
 * nenhuma leitura nova. O cartão mostra a **fonte** em vez de a esconder: uma
 * leitura ESIF e uma leitura ACPI não valem o mesmo, e o utilizador tem o
 * direito de saber qual está a ver.
 *
 * O SSD não é legível sem elevação: `MSFT_StorageReliabilityCounter` responde
 * `0x80041003` a um processo normal (medido). Fica em `—` com a razão, porque
 * uma temperatura de disco inventada é pior do que a sua ausência.
 */

import { t } from '../../hooks/useI18n';
import type { FanInfo } from '../../types/hardware';
import { Badge } from '../ui/badge';
import { Card } from '../ui/card';
import { Icon } from '../ui/icons';
import { StatGroup, StatRow } from '../ui/stat-row';
import { EMPTY, fmtNumber } from './format';

interface Props {
  fan: FanInfo | null;
  loading: boolean;
}

function sourceLabel(source: FanInfo['temp_source'] | undefined): string | null {
  switch (source) {
    case 'esif':
      return t('overview.thermal.sourceEsif');
    case 'acpi':
      return t('overview.thermal.sourceAcpi');
    case 'ecram':
      return t('overview.thermal.sourceEcram');
    case 'unavailable':
      return null;
    default:
      return null;
  }
}

export function OverviewThermal({ fan, loading }: Props) {
  const cpu = fan?.cpu_temp_celsius ?? null;
  const gpu = fan?.gpu_temp_celsius ?? null;
  const ambient = fan?.ambient_temp_celsius ?? null;
  const tdp = fan?.tdp_watts ?? null;
  const source = sourceLabel(fan?.temp_source);
  const hasAny = cpu != null || gpu != null;

  return (
    <Card
      title={t('overview.thermal.title')}
      icon={<Icon name="thermometer" size={16} />}
      loading={loading}
      meta={
        source ? (
          <Badge tone="neutral">{source}</Badge>
        ) : (
          <Badge tone="warning" dot>
            {t('overview.thermal.noSource')}
          </Badge>
        )
      }
    >
      <StatGroup>
        <StatRow
          label={t('overview.thermal.cpu')}
          value={cpu != null ? fmtNumber(cpu, 1) : EMPTY}
          unit={cpu != null ? '°C' : undefined}
          empty={cpu == null}
          mono
        />
        <StatRow
          label={t('overview.thermal.gpu')}
          value={gpu != null ? fmtNumber(gpu, 1) : EMPTY}
          unit={gpu != null ? '°C' : undefined}
          empty={gpu == null}
          mono
        />
        <StatRow
          label={t('overview.thermal.ssd')}
          value={EMPTY}
          empty
          hint={t('overview.thermal.ssdHint')}
        />
        <StatRow
          label={t('overview.thermal.ambient')}
          value={ambient != null ? fmtNumber(ambient, 1) : EMPTY}
          unit={ambient != null ? '°C' : undefined}
          empty={ambient == null}
          hint={t('overview.thermal.ambientHint')}
        />
        <StatRow
          label={t('overview.thermal.power')}
          value={tdp != null ? fmtNumber(tdp, 1) : EMPTY}
          unit={tdp != null ? 'W' : undefined}
          empty={tdp == null}
          mono
        />
      </StatGroup>

      {!hasAny && (
        <p className="mc-note mc-note--warning" role="status">
          <Icon name="alert-triangle" size={14} />
          {t('overview.thermal.unavailable')}
        </p>
      )}
    </Card>
  );
}

export default OverviewThermal;
