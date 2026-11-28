/**
 * "Ir para…" — atalhos do resumo para as tabs onde cada bloco se altera.
 *
 * O resumo lê; a escrita vive nas tabs. Cada pílula leva o rótulo canónico do
 * `nav.*` para o nome do destino nunca divergir do menu lateral.
 */

import { t, type StringKey } from '../../hooks/useI18n';
import { Card } from '../ui/card';
import { Icon, type IconName } from '../ui/icons';

interface Props {
  onNavigate: (tab: string) => void;
}

interface Jump {
  tab: string;
  icon: IconName;
  label: StringKey;
}

/** Ordem = ordem por que os blocos aparecem no resumo. */
const JUMPS: Jump[] = [
  { tab: 'battery', icon: 'battery', label: 'nav.battery' },
  { tab: 'performance', icon: 'bolt', label: 'nav.performance' },
  { tab: 'fan', icon: 'wind', label: 'nav.fan' },
  { tab: 'display', icon: 'monitor', label: 'nav.display' },
  { tab: 'sysopt', icon: 'rocket', label: 'nav.sysOpt' },
  { tab: 'iot', icon: 'plug', label: 'nav.iot' },
  { tab: 'wifi', icon: 'wifi', label: 'nav.wifi' },
  { tab: 'system', icon: 'wrench', label: 'nav.system' },
  { tab: 'security', icon: 'shield', label: 'nav.security' },
  { tab: 'taskmgr', icon: 'chart-line', label: 'nav.taskmgr' },
];

export function OverviewJump({ onNavigate }: Props) {
  return (
    <Card title={t('overview.jump.title')} icon={<Icon name="link" size={16} />}>
      <p className="mc-group__note">{t('overview.jump.hint')}</p>
      <nav className="mc-jump" aria-label={t('overview.jump.title')}>
        {JUMPS.map((jump) => (
          <button
            key={jump.tab}
            type="button"
            className="mc-jump__btn"
            onClick={() => onNavigate(jump.tab)}
          >
            <Icon name={jump.icon} size={14} />
            {t(jump.label)}
          </button>
        ))}
      </nav>
    </Card>
  );
}

export default OverviewJump;
