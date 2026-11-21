/**
 * Hardware — identidade da máquina + CPU + GPU + NPU.
 *
 * É o bloco que faltava: a app sabia o uso de CPU mas não o modelo do
 * processador, o cache, o driver gráfico nem que existe uma NPU. Tudo aqui vem
 * de `get_overview_snapshot` (leitura única, cache em `OnceLock`).
 *
 * A NPU é a peça "sempre esquecida": o Windows 11 expõe-na como classe de
 * dispositivo `ComputeAccelerator`, e é essa a fonte. Os **TOPS não são
 * observáveis** por API nenhuma — ficam em `—` com a razão, nunca com o número
 * do datasheet, que mudaria com o modelo e mentiria sem aviso.
 */

import { t, type StringKey } from '../../hooks/useI18n';
import type { OverviewSnapshot } from '../../types/hardware';
import { Badge } from '../ui/badge';
import { Card } from '../ui/card';
import { Icon } from '../ui/icons';
import { StatGroup, StatRow } from '../ui/stat-row';
import { EMPTY, fmtIsoDate, fmtNumber, fmtText, fmtUnit } from './format';

interface Props {
  snapshot: OverviewSnapshot | null;
  loading: boolean;
}

/** `MSFT_PhysicalDisk.HealthStatus` chega como rótulo inglês; traduz-se aqui. */
const HEALTH_KEY: Record<string, StringKey> = {
  Healthy: 'overview.storage.healthHealthy',
  Warning: 'overview.storage.healthWarning',
  Unhealthy: 'overview.storage.healthUnhealthy',
  Unknown: 'overview.storage.healthUnknown',
};

const HEALTH_TONE: Record<string, 'success' | 'warning' | 'danger' | 'neutral'> = {
  Healthy: 'success',
  Warning: 'warning',
  Unhealthy: 'danger',
  Unknown: 'neutral',
};

export function OverviewHardware({ snapshot, loading }: Props) {
  const device = snapshot?.device;
  const cpu = snapshot?.cpu;
  const gpu = snapshot?.gpu;
  const npu = snapshot?.npu;

  return (
    <Card
      title={t('overview.hardware.title')}
      icon={<Icon name="monitor" size={16} />}
      loading={loading}
      meta={device?.family ? <Badge tone="neutral">{device.family}</Badge> : undefined}
    >
      <div className="mc-group">
        <h3 className="mc-group__title">{t('overview.hardware.machine')}</h3>
        <p className="mc-lead">
          {fmtText(device?.manufacturer)} · {fmtText(device?.model)}
          {device?.sku && <span className="mc-lead__sub">{device.sku}</span>}
        </p>
        <StatGroup>
          <StatRow label={t('overview.hardware.bios')} value={fmtText(device?.bios_version)} />
        </StatGroup>
      </div>

      <div className="mc-group">
        <h3 className="mc-group__title">{t('overview.hardware.cpu')}</h3>
        <p className="mc-lead">{fmtText(cpu?.name)}</p>
        <StatGroup>
          <StatRow
            label={t('overview.hardware.cores')}
            value={cpu?.cores ? fmtNumber(cpu.cores) : EMPTY}
          />
          <StatRow
            label={t('overview.hardware.threads')}
            value={cpu?.threads ? fmtNumber(cpu.threads) : EMPTY}
          />
          <StatRow
            label={t('overview.hardware.baseClock')}
            value={fmtNumber(cpu?.base_clock_mhz)}
            unit={cpu?.base_clock_mhz ? 'MHz' : undefined}
          />
          <StatRow
            label={t('overview.hardware.l3')}
            value={fmtNumber(cpu?.l3_cache_mb)}
            unit={cpu?.l3_cache_mb ? 'MB' : undefined}
          />
        </StatGroup>
      </div>

      <div className="mc-group">
        <h3 className="mc-group__title">{t('overview.hardware.gpu')}</h3>
        <p className="mc-lead">{fmtText(gpu?.name)}</p>
        <StatGroup>
          <StatRow
            label={t('overview.hardware.driver')}
            value={fmtText(gpu?.driver_version)}
            mono
          />
        </StatGroup>
      </div>

      <div className="mc-group">
        <h3 className="mc-group__title">{t('overview.hardware.npu')}</h3>
        {npu ? (
          <>
            <p className="mc-lead">
              {fmtText(npu.name)}
              {npu.vendor && <span className="mc-lead__sub">{npu.vendor}</span>}
            </p>
            <StatGroup>
              <StatRow
                label={t('overview.hardware.npuState')}
                value={
                  npu.status === 'ok' ? (
                    <Badge tone="success" dot>
                      {t('overview.hardware.npuOk')}
                    </Badge>
                  ) : (
                    <Badge tone="warning" dot>
                      {t('overview.hardware.npuError')}
                    </Badge>
                  )
                }
              />
              <StatRow
                label={t('overview.hardware.driver')}
                value={fmtText(npu.driver_version)}
                mono
              />
              <StatRow
                label={t('overview.hardware.driverDate')}
                value={fmtIsoDate(npu.driver_date)}
              />
              <StatRow
                label={t('overview.hardware.tops')}
                value={EMPTY}
                empty
                hint={t('overview.hardware.topsHint')}
              />
            </StatGroup>
          </>
        ) : (
          <p className="mc-group__note">
            {loading ? t('overview.hardware.npuReading') : t('overview.hardware.npuNone')}
          </p>
        )}
      </div>
    </Card>
  );
}

/** Memória e armazenamento — o que o resumo ainda não dizia sobre 32 GB e o NVMe. */
export function OverviewMemoryStorage({ snapshot, loading }: Props) {
  const memory = snapshot?.memory;
  const storage = snapshot?.storage;

  return (
    <Card
      title={t('overview.storage.title')}
      icon={<Icon name="package" size={16} />}
      loading={loading}
    >
      <div className="mc-group">
        <h3 className="mc-group__title">{t('overview.storage.memory')}</h3>
        <p className="mc-lead">
          {fmtUnit(memory?.total_gb, 'GB', 0)}
          {memory?.memory_type && (
            <span className="mc-lead__sub">
              {memory.memory_type}
              {memory.speed_mt_s ? ` · ${fmtNumber(memory.speed_mt_s)} MT/s` : ''}
            </span>
          )}
        </p>
        <StatGroup>
          <StatRow
            label={t('overview.storage.modules')}
            value={memory?.module_count ? fmtNumber(memory.module_count) : EMPTY}
            hint={memory?.module_count ? t('overview.storage.modulesHint') : undefined}
          />
          <StatRow
            label={t('overview.storage.speed')}
            value={fmtNumber(memory?.speed_mt_s)}
            unit={memory?.speed_mt_s ? 'MT/s' : undefined}
          />
          <StatRow label={t('overview.storage.vendor')} value={fmtText(memory?.manufacturer)} />
        </StatGroup>
      </div>

      <div className="mc-group">
        <h3 className="mc-group__title">{t('overview.storage.disk')}</h3>
        <p className="mc-lead">{fmtText(storage?.model)}</p>
        <StatGroup>
          <StatRow
            label={t('overview.storage.capacity')}
            value={fmtUnit(storage?.capacity_gb, 'GB', 0)}
          />
          <StatRow
            label={t('overview.storage.free')}
            value={fmtUnit(storage?.free_gb, 'GB', 0)}
            hint={
              storage && storage.capacity_gb > 0
                ? t('overview.storage.freeHint', {
                    pct: Math.round((storage.free_gb / storage.capacity_gb) * 100),
                  })
                : undefined
            }
          />
          <StatRow
            label={t('overview.storage.bus')}
            value={
              storage?.bus
                ? [storage.bus, storage.media].filter(Boolean).join(' · ')
                : fmtText(storage?.media)
            }
          />
          <StatRow
            label={t('overview.storage.health')}
            value={
              storage?.health ? (
                <Badge tone={HEALTH_TONE[storage.health] ?? 'neutral'} dot>
                  {t(HEALTH_KEY[storage.health] ?? 'overview.storage.healthUnknown')}
                </Badge>
              ) : (
                EMPTY
              )
            }
            empty={!storage?.health}
          />
        </StatGroup>
      </div>
    </Card>
  );
}

export default OverviewHardware;
