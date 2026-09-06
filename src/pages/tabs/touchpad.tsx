import { memo, useCallback } from 'react';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import TouchpadSettings from '../../components/TouchpadSettings';
import { useToast } from '../../contexts/ToastContext';
import type { Hardware } from './shared';

interface Props {
  hw: Hardware;
}

/**
 * S53: wraps each touchpad setter with success/failure toasts.
 *
 * The backend setters throw on hardware failure (HID report rejection), but
 * the tab previously swallowed the result — the user toggled and got zero
 * feedback, reinforcing the "does nothing" perception. Now: green toast on
 * success, red toast with the error message on failure.
 */
function TouchpadTab({ hw }: Props) {
  const toast = useToast();

  const wrap = useCallback(
    (
      action: (v: never) => Promise<void>,
      successKey: Parameters<typeof t>[0],
      label: string,
    ): ((v: never) => Promise<void>) => {
      return async (value: never) => {
        try {
          await action(value);
          toast.addToast(`${t(successKey)} — ${label}`, 'success');
        } catch (e) {
          const msg =
            typeof e === 'object' && e !== null && 'message' in e
              ? String((e as { message: unknown }).message)
              : String(e);
          toast.addToast(`${t('touchpad.applyFailed')}: ${msg}`, 'error');
        }
      };
    },
    [toast],
  );

  const onSensitivity = useCallback(
    (v: 'low' | 'medium' | 'high' | 'very_high') =>
      wrap(
        hw.setTouchpadSensitivity,
        'touchpad.toastSensitivity',
        t(`touchpad.levels.${v}`),
      )(v as never),
    [hw, wrap],
  );
  const onHaptics = useCallback(
    (v: boolean) =>
      wrap(hw.setTouchpadHaptics, 'touchpad.toastHaptics', v ? 'ON' : 'OFF')(v as never),
    [hw, wrap],
  );
  const onHapticsIntensity = useCallback(
    (v: 'low' | 'medium' | 'high') =>
      wrap(
        hw.setTouchpadHapticsIntensity,
        'touchpad.toastHapticsIntensity',
        t(`touchpad.levels.${v}`),
      )(v as never),
    [hw, wrap],
  );
  const onGestureScreenshot = useCallback(
    (v: boolean) =>
      wrap(
        hw.setTouchpadGestureScreenshot,
        'touchpad.toastGestureScreenshot',
        v ? 'ON' : 'OFF',
      )(v as never),
    [hw, wrap],
  );
  const onRepress = useCallback(
    (v: boolean) =>
      wrap(hw.setTouchpadRepress, 'touchpad.toastRepress', v ? 'ON' : 'OFF')(v as never),
    [hw, wrap],
  );
  const onEdgeSlide = useCallback(
    (v: boolean) =>
      wrap(hw.setTouchpadEdgeSlide, 'touchpad.toastEdgeSlide', v ? 'ON' : 'OFF')(v as never),
    [hw, wrap],
  );

  return (
    <>
      <PageHeader title={t('touchpad.title')} />
      <TouchpadSettings
        touchpad={hw.touchpad}
        capabilities={hw.hardwareProfile?.capabilities}
        onSensitivityChange={onSensitivity}
        onHapticsChange={onHaptics}
        onHapticsIntensityChange={onHapticsIntensity}
        onGestureScreenshotChange={onGestureScreenshot}
        onRepressChange={onRepress}
        onEdgeSlideChange={onEdgeSlide}
      />
    </>
  );
}

export default memo(TouchpadTab);
