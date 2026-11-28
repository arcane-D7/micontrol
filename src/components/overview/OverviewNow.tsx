/**
 * "Agora" — três leituras rápidas que o resumo agregava mal: ecrã, rede e IoT.
 *
 * Nada aqui abre um comando novo: o ecrã vem do `get_display_info` (já no
 * `useHardware`), a rede do `wifi_status` e o IoT do `get_iot_device_info`
 * (ambos no `useOverview`). O IoT custa ~3,5 s na ponte, por isso é lido uma
 * vez e aparece como `—` até chegar — nunca trava o resto do cartão.
 */

import { t } from '../../hooks/useI18n';
import type { DisplayInfo, IotDeviceInfo, OverviewSnapshot } from '../../types/hardware';
import type { WifiSummary } from '../../hooks/useOverview';
import { Badge } from '../ui/badge';
import { Card } from '../ui/card';
import { Icon } from '../ui/icons';
import { StatGroup, StatRow } from '../ui/stat-row';
import { EMPTY, fmtNumber, fmtResolution, fmtText } from './format';

interface Props {
  snapshot: OverviewSnapshot | null;
  display: DisplayInfo | null;
  wifi: WifiSummary | null;
  iot: IotDeviceInfo | null;
  loading: boolean;
}

export function OverviewNow({ snapshot, display, wifi, iot, loading }: Props) {
  const panel = snapshot?.display;

  return (
    <Card title={t('overview.now.title')} icon={<Icon name="bolt" size={16} />} loading={loading}>
      <div className="grid-3">
        <div>
          <h3 className="mc-group__title">{t('overview.now.screen')}</h3>
          <p className="mc-lead">
            {fmtResolution(panel?.width ?? 0, panel?.height ?? 0)}
            <span className="mc-lead__sub">
              {[panel?.name, display ? `${fmtNumber(display.refresh_rate_hz)} Hz` : null]
                .filter(Boolean)
                .join(' · ') || t('overview.now.noPanel')}
            </span>
          </p>
          <StatGroup>
            <StatRow
              label={t('overview.now.hdr')}
              value={
                display == null ? (
                  EMPTY
                ) : display.hdr_enabled ? (
                  <Badge tone="success" dot>
                    {t('overview.now.on')}
                  </Badge>
                ) : (
                  <Badge tone="neutral" dot>
                    {t('overview.now.off')}
                  </Badge>
                )
              }
              empty={display == null}
              hint={
                display && !display.hdr_supported ? t('overview.now.hdrUnsupported') : undefined
              }
            />
            <StatRow
              label={t('overview.now.brightness')}
              value={display ? fmtNumber(display.brightness) : EMPTY}
              unit={display ? '%' : undefined}
              empty={display == null}
            />
            <StatRow
              label={t('overview.now.ambient')}
              value={display?.ambient_lux != null ? fmtNumber(display.ambient_lux, 0) : EMPTY}
              unit={display?.ambient_lux != null ? 'lx' : undefined}
              empty={display?.ambient_lux == null}
            />
          </StatGroup>
        </div>

        <div>
          <h3 className="mc-group__title">{t('overview.now.network')}</h3>
          <p className="mc-lead">
            {fmtText(wifi?.ssid)}
            {wifi?.interface && <span className="mc-lead__sub">{wifi.interface}</span>}
          </p>
          <StatGroup>
            <StatRow
              label={t('overview.now.link')}
              value={
                wifi == null ? (
                  EMPTY
                ) : wifi.connected ? (
                  <Badge tone="success" dot>
                    {t('overview.now.connected')}
                  </Badge>
                ) : (
                  <Badge tone="neutral" dot>
                    {t('overview.now.disconnected')}
                  </Badge>
                )
              }
              empty={wifi == null}
            />
            <StatRow
              label={t('overview.now.signal')}
              value={wifi?.signal != null ? fmtNumber(wifi.signal) : EMPTY}
              unit={wifi?.signal != null ? '%' : undefined}
              empty={wifi?.signal == null}
            />
          </StatGroup>
        </div>

        <div>
          <h3 className="mc-group__title">{t('overview.now.iot')}</h3>
          <p className="mc-lead">
            {fmtText(iot?.model)}
            {iot?.fw_version && (
              <span className="mc-lead__sub">
                {t('overview.now.firmware')} {iot.fw_version}
              </span>
            )}
          </p>
          <StatGroup>
            <StatRow
              label={t('overview.now.pipe')}
              value={
                iot == null ? (
                  EMPTY
                ) : iot.pipe_available ? (
                  <Badge tone="success" dot>
                    {t('overview.now.online')}
                  </Badge>
                ) : (
                  <Badge tone="warning" dot>
                    {t('overview.now.offline')}
                  </Badge>
                )
              }
              empty={iot == null}
            />
            <StatRow label={t('overview.now.device')} value={fmtText(iot?.device_status)} />
            <StatRow
              label={t('overview.now.pairing')}
              value={
                iot?.bind_status == null ? (
                  EMPTY
                ) : iot.bind_status.bound ? (
                  <Badge tone="success" dot>
                    {t('overview.now.bound')}
                  </Badge>
                ) : (
                  <Badge tone="neutral" dot>
                    {t('overview.now.unbound')}
                  </Badge>
                )
              }
              empty={iot?.bind_status == null}
            />
          </StatGroup>
        </div>
      </div>
    </Card>
  );
}

export default OverviewNow;
