/**
 * Proteção da bateria — blocos 6, 7 e 8 da spec §5.3.
 *
 * Resolve o RED UX-01 (clique morto): com a proteção desligada, clicar num
 * nível **liga a proteção e aplica o nível** num só gesto. Não há
 * `pointer-events:none` nem `opacity:.4` mudo — há uma razão escrita e um
 * caminho para agir.
 *
 * Resolve o RED UX-09 (feedback sem delta): o segmento marca em ≤100 ms
 * (optimistic), a linha diz "A aplicar 80%…", e o toast nomeia valor +
 * consequência. Se a escrita no EC falhar, o segmento **reverte** e aparece um
 * erro inline com "Tentar de novo" — nunca um toast que não diz o que mudou.
 */

import { useState } from 'react';
import { t } from '../hooks/useI18n';
import { useToast } from '../contexts/ToastContext';
import { getUserFriendlyMessage, parseErrorResponse, type TranslateFn } from '../types/error';
import InfoModal, { InfoRow, InfoSection } from './InfoModal';
import { Card } from './ui/card';
import { Icon } from './ui/icons';
import { IconButton } from './ui/icon-button';
import { Segmented, type SegmentedOption } from './ui/segmented';
import { Switch } from './ui/switch';

// `t` from useI18n types keys as a literal union; error.ts' TranslateFn takes
// any string — wrap it (matches the pattern used across the codebase).
const translate: TranslateFn = (key) => t(key as never);

const LEVELS = [40, 50, 60, 70, 80, 100] as const;
const RECOMMENDED = 80;

export interface ChargingProtectionProps {
  /** Limite configurado; 100 = sem limite. */
  threshold: number;
  /** Nível atual da bateria — para avisar quando o limite fica abaixo dele. */
  level: number;
  /** null = a ler `get_battery_care`. */
  care: boolean | null;
  carePending: boolean;
  /** Escreve o limite (o pai faz optimistic + revert). */
  onThresholdChange: (threshold: number) => Promise<void>;
  /** Liga a proteção **e** aplica o limite num só gesto. */
  onEnableCare: (limit: number) => Promise<void>;
  /** Desliga a proteção (o limite passa a 100). */
  onDisableCare: () => Promise<void>;
}

export default function ChargingProtection({
  threshold,
  level,
  care,
  carePending,
  onThresholdChange,
  onEnableCare,
  onDisableCare,
}: ChargingProtectionProps) {
  const [optimistic, setOptimistic] = useState<number | null>(null);
  const [applying, setApplying] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const [showInfo, setShowInfo] = useState(false);
  const { addToast } = useToast();

  const careEnabled = care === true;
  const display = optimistic ?? (careEnabled ? threshold : 100);
  const busy = applying || carePending;

  const options: SegmentedOption[] = LEVELS.map((value) => ({
    value,
    label: `${value}%`,
    badge: value === RECOMMENDED ? t('charging.recommended') : undefined,
  }));

  /** Um gesto: marca já, escreve depois, reverte se falhar. */
  const apply = async (value: number) => {
    if (busy || care === null) return;
    setOptimistic(value);
    setApplying(true);
    setFailure(null);
    try {
      if (!careEnabled) {
        await onEnableCare(value);
        addToast({
          message: t('battery.toast.careAndLimit', { limit: value }),
          type: 'success',
        });
      } else {
        await onThresholdChange(value);
        addToast({
          message: t('battery.toast.limitApplied', { limit: value }),
          type: 'success',
        });
      }
    } catch (e) {
      setOptimistic(null);
      setFailure(getUserFriendlyMessage(parseErrorResponse(e), translate));
    } finally {
      setApplying(false);
      setOptimistic(null);
    }
  };

  const toggleCare = async (next: boolean) => {
    if (busy || care === null) return;
    setFailure(null);
    try {
      if (next) {
        const limit = display < 100 ? display : RECOMMENDED;
        await onEnableCare(limit);
        addToast({
          message: t('battery.toast.careAndLimit', { limit }),
          type: 'success',
        });
      } else {
        await onDisableCare();
        addToast({ message: t('battery.toast.careOff'), type: 'info' });
      }
    } catch (e) {
      setFailure(getUserFriendlyMessage(parseErrorResponse(e), translate));
    }
  };

  const derived = (() => {
    if (failure) return null;
    if (applying)
      return { text: t('battery.applying', { limit: display }), tone: 'muted' as const };
    if (care === null) return { text: t('common.loading'), tone: 'muted' as const };
    if (!careEnabled) return { text: t('battery.phase.careOff'), tone: 'muted' as const };
    if (display >= 100) return { text: t('battery.phase.noLimit'), tone: 'muted' as const };
    if (level > display) return { text: t('battery.limitAboveLevel'), tone: 'warn' as const };
    return { text: t('battery.limitDerived', { limit: display }), tone: 'neutral' as const };
  })();

  return (
    <Card
      title={t('battery.careTitle')}
      action={
        <IconButton
          icon="info"
          label={t('charging.infoModal.title')}
          onClick={() => setShowInfo(true)}
        />
      }
    >
      <Switch
        label={t('battery.limitLabel')}
        desc={t('battery.careDesc', { limit: display < 100 ? display : RECOMMENDED })}
        checked={careEnabled}
        pending={carePending}
        disabled={care === null}
        disabledReason={care === null ? t('common.loading') : undefined}
        onChange={(next) => void toggleCare(next)}
      />

      <div className="mc-seg-wrap">
        <Segmented
          label={t('battery.limitLabel')}
          options={options}
          value={display}
          pending={applying}
          onChange={(value) => void apply(value)}
          disabledReason={
            care === null
              ? t('common.loading')
              : !careEnabled
                ? t('battery.enableToChoose')
                : undefined
          }
          onDisabledSelect={(value) => void apply(value)}
          onDisabledClick={() => void apply(RECOMMENDED)}
        />
      </div>

      {failure ? (
        <div className="mc-note mc-note--danger" role="alert">
          <Icon name="info" size={16} />
          <span>
            {t('battery.applyFailed')} {failure}
          </span>
          <span className="mc-note__spacer" />
          <button type="button" className="mc-linkbtn" onClick={() => void apply(display)}>
            {t('battery.retry')}
          </button>
        </div>
      ) : (
        derived && (
          <div
            className={`mc-hero__phase${
              derived.tone === 'warn'
                ? ' mc-hero__phase--warn'
                : derived.tone === 'muted'
                  ? ' mc-hero__phase--muted'
                  : ''
            }`}
            data-testid="limit-derived"
          >
            {derived.text}
          </div>
        )
      )}

      <InfoModal
        open={showInfo}
        onClose={() => setShowInfo(false)}
        title={t('charging.infoModal.title')}
      >
        <InfoRow label={t('charging.infoModal.functionLabel')}>
          {t('charging.infoModal.functionDesc')}
        </InfoRow>
        <InfoRow label={t('charging.infoModal.requiresLabel')}>
          {t('charging.infoModal.requiresDesc')}
        </InfoRow>
        <InfoRow label={t('charging.infoModal.behaviorLabel')}>
          {t('charging.infoModal.behaviorDesc')}
        </InfoRow>
        <InfoSection>
          <div className="mc-note mc-note--warning">
            <strong>{t('charging.infoModal.warningLabel')}</strong>{' '}
            {t('charging.infoModal.warningDesc')}
          </div>
        </InfoSection>
      </InfoModal>
    </Card>
  );
}
