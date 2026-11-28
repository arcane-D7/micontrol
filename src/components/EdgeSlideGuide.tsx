import { useId } from 'react';
import { t } from '../hooks/useI18n';
import { Accordion } from './ui/accordion';

/**
 * EdgeSlideGuide — o "onde tocar e como" do Edge Slide.
 *
 * O toggle sozinho não ensina nada: a banda tem 16 % da largura, o pré-rolo
 * de 8 mm não dá feedback nenhum e o cursor fica preso a meio do gesto. Quem
 * não sabe disto conclui que "não funciona". Isto é a resposta, escrita a
 * partir do temporizador real (`hw/touchpad.rs`): banda de arranque,
 * `EDGE_ACTIVATION_PERMILLE`, `ClipCursor`, tick háptico por passo e as duas
 * chaves de registo que afinam a banda.
 *
 * O diagrama usa só `currentColor` e os tokens (`--accent-soft`, `--hairline-2`,
 * `--surface-2`): nada de cores literais, por isso segue o tema claro/escuro
 * sem uma única media query.
 */
export default function EdgeSlideGuide() {
  const clipId = useId();
  const padX = 8;
  const padY = 8;
  const padW = 344;
  const padH = 196;
  const bandW = Math.round((padW * 16) / 100); // 16 % de x_max — a banda real de arranque
  const leftCx = padX + bandW / 2;
  const rightCx = padX + padW - bandW / 2;

  const swipe = (cx: number) =>
    `M${cx} 154 V 58 M${cx - 7} 68 L${cx} 58 L${cx + 7} 68 M${cx - 7} 144 L${cx} 154 L${cx + 7} 144`;

  return (
    <Accordion summary={t('touchpadHelp.summary')} className="mc-edgeslide-guide">
      <svg
        className="mc-touchdiagram"
        viewBox="0 0 360 232"
        role="img"
        aria-label={t('touchpadHelp.diagramAlt')}
      >
        <defs>
          <clipPath id={clipId}>
            <rect x={padX} y={padY} width={padW} height={padH} rx={14} />
          </clipPath>
        </defs>

        <rect
          className="mc-touchdiagram__pad"
          x={padX}
          y={padY}
          width={padW}
          height={padH}
          rx={14}
        />

        <g clipPath={`url(#${clipId})`}>
          <rect className="mc-touchdiagram__band" x={padX} y={padY} width={bandW} height={padH} />
          <rect
            className="mc-touchdiagram__band"
            x={padX + padW - bandW}
            y={padY}
            width={bandW}
            height={padH}
          />
        </g>

        <path className="mc-touchdiagram__arrow" d={swipe(leftCx)} />
        <path className="mc-touchdiagram__arrow" d={swipe(rightCx)} />

        <text className="mc-touchdiagram__note" x={180} y={padY + padH / 2 + 3} textAnchor="middle">
          {t('touchpadHelp.diagramFinger')}
        </text>

        <text className="mc-touchdiagram__cap" x={leftCx} y={padY + padH + 20} textAnchor="middle">
          {t('touchpadHelp.diagramLeft')}
        </text>
        <text className="mc-touchdiagram__cap" x={rightCx} y={padY + padH + 20} textAnchor="middle">
          {t('touchpadHelp.diagramRight')}
        </text>
      </svg>

      <h4>{t('touchpadHelp.howTitle')}</h4>
      <ol>
        <li>{t('touchpadHelp.step1')}</li>
        <li>{t('touchpadHelp.step2')}</li>
        <li>{t('touchpadHelp.step3')}</li>
        <li>{t('touchpadHelp.step4')}</li>
        <li>{t('touchpadHelp.step5')}</li>
      </ol>

      <h4>{t('touchpadHelp.notTitle')}</h4>
      <ul>
        <li>{t('touchpadHelp.notScrollbar')}</li>
        <li>{t('touchpadHelp.notTwoFingers')}</li>
        <li>{t('touchpadHelp.notCornerClick')}</li>
      </ul>

      <h4>{t('touchpadHelp.bandTitle')}</h4>
      <p>{t('touchpadHelp.bandBody')}</p>

      <h4>{t('touchpadHelp.troubleTitle')}</h4>
      <ol>
        <li>{t('touchpadHelp.trouble1')}</li>
        <li>{t('touchpadHelp.trouble2')}</li>
        <li>{t('touchpadHelp.trouble3')}</li>
      </ol>
    </Accordion>
  );
}
