import { useState, useRef, useEffect, useCallback } from 'react';
import { t } from '../hooks/useI18n';

interface Props {
  onFinish: () => void;
}

const STEPS = ['welcome', 'privacy', 'features', 'ready'] as const;

export default function OnboardingWizard({ onFinish }: Props) {
  const [step, setStep] = useState(0);
  const total = STEPS.length;
  const current = STEPS[step];

  // S24-010: Focus trap — refs and handlers (same pattern as ConsentDialog)
  const overlayRef = useRef<HTMLDivElement>(null);
  const modalRef = useRef<HTMLDivElement>(null);
  const previouslyFocused = useRef<HTMLElement | null>(null);

  // Set initial focus to first interactive element on open
  useEffect(() => {
    previouslyFocused.current = document.activeElement as HTMLElement | null;
    // Focus the modal container (neutral, not a button)
    modalRef.current?.focus();
    return () => {
      // Restore focus to trigger element on close
      previouslyFocused.current?.focus();
    };
  }, []);

  // Focus trap: Tab cycles within modal; Escape closes
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onFinish();
        return;
      }
      if (e.key !== 'Tab') return;

      const focusable = modalRef.current?.querySelectorAll<HTMLElement>(
        'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
      );
      if (!focusable || focusable.length === 0) return;

      const first = focusable[0];
      const last = focusable[focusable.length - 1];

      if (e.shiftKey) {
        if (document.activeElement === first) {
          e.preventDefault();
          last.focus();
        }
      } else {
        if (document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      }
    },
    [onFinish],
  );

  return (
    <div
      ref={overlayRef}
      role="dialog"
      aria-modal="true"
      aria-labelledby="onboarding-title"
      onKeyDown={handleKeyDown}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 9999,
        background: 'oklch(0% 0 0 / 0.6)',
        backdropFilter: 'blur(4px)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}
    >
      <div
        ref={modalRef}
        tabIndex={-1}
        style={{
          background: 'var(--surface)',
          borderRadius: 16,
          padding: '40px 48px',
          maxWidth: 520,
          width: '90%',
          boxShadow: '0 8px 40px oklch(0% 0 0 / 0.4)',
          display: 'flex',
          flexDirection: 'column',
          gap: 24,
          outline: 'none',
        }}
      >
        {/* Progress indicator */}
        <div style={{ textAlign: 'center', fontSize: 13, color: 'var(--text-dim)' }}>
          {t('onboarding.step', { current: step + 1, total })}
        </div>

        {/* Dots */}
        <div
          role="progressbar"
          aria-label="Onboarding progress"
          aria-valuenow={step + 1}
          aria-valuemin={1}
          aria-valuemax={total}
          style={{
            display: 'flex',
            justifyContent: 'center',
            gap: 8,
          }}
        >
          {STEPS.map((_, i) => (
            <div
              key={i}
              style={{
                width: 8,
                height: 8,
                borderRadius: '50%',
                background:
                  i === step ? 'var(--accent)' : 'var(--border-color, oklch(60% 0 0 / 0.3))',
                transition: 'background 0.2s',
              }}
            />
          ))}
        </div>

        {/* Step content */}
        <div style={{ minHeight: 200, display: 'flex', flexDirection: 'column', gap: 16 }}>
          {current === 'welcome' && (
            <>
              <h2 id="onboarding-title" style={{ margin: 0, fontSize: 24, textAlign: 'center' }}>
                {t('onboarding.welcome.title')}
              </h2>
              <p
                style={{
                  margin: 0,
                  textAlign: 'center',
                  color: 'var(--text-dim)',
                  lineHeight: 1.6,
                }}
              >
                {t('onboarding.welcome.description')}
              </p>
            </>
          )}

          {current === 'privacy' && (
            <>
              <h2 style={{ margin: 0, fontSize: 24, textAlign: 'center' }}>
                {t('onboarding.privacy.title')}
              </h2>
              <p
                style={{
                  margin: 0,
                  textAlign: 'center',
                  color: 'var(--text-dim)',
                  lineHeight: 1.6,
                }}
              >
                {t('onboarding.privacy.description')}
              </p>
            </>
          )}

          {current === 'features' && (
            <>
              <h2 style={{ margin: 0, fontSize: 24, textAlign: 'center' }}>
                {t('onboarding.features.title')}
              </h2>
              <ul
                style={{
                  margin: 0,
                  paddingLeft: 20,
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 10,
                  color: 'var(--text-dim)',
                  lineHeight: 1.5,
                }}
              >
                <li>{t('onboarding.features.hardwareControl')}</li>
                <li>{t('onboarding.features.driverManagement')}</li>
                <li>{t('onboarding.features.systemInfo')}</li>
                <li>{t('onboarding.features.iotService')}</li>
              </ul>
            </>
          )}

          {current === 'ready' && (
            <>
              <h2 style={{ margin: 0, fontSize: 24, textAlign: 'center' }}>
                {t('onboarding.ready.title')}
              </h2>
              <p
                style={{
                  margin: 0,
                  textAlign: 'center',
                  color: 'var(--text-dim)',
                  lineHeight: 1.6,
                }}
              >
                {t('onboarding.ready.description')}
              </p>
            </>
          )}
        </div>

        {/* Navigation buttons */}
        <div
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
            gap: 12,
          }}
        >
          <div>
            {step > 0 ? (
              <button className="btn btn-secondary" onClick={() => setStep(step - 1)}>
                {t('onboarding.back')}
              </button>
            ) : (
              <div />
            )}
          </div>

          <div style={{ display: 'flex', gap: 8 }}>
            {step < total - 1 && (
              <>
                <button className="btn btn-ghost" onClick={onFinish}>
                  {t('onboarding.skip')}
                </button>
                <button className="btn btn-primary" onClick={() => setStep(step + 1)}>
                  {current === 'welcome'
                    ? t('onboarding.welcome.getStarted')
                    : current === 'privacy'
                      ? t('onboarding.privacy.allow')
                      : t('onboarding.next')}
                </button>
                {current === 'privacy' && (
                  <button className="btn btn-secondary" onClick={onFinish}>
                    {t('onboarding.privacy.deny')}
                  </button>
                )}
              </>
            )}
            {current === 'ready' && (
              <button className="btn btn-primary" onClick={onFinish}>
                {t('onboarding.ready.finish')}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
