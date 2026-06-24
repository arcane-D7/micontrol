import { useState, useEffect, useCallback, lazy, Suspense, memo } from 'react';
import type { ThemeMode } from '../App';
import { t } from '../hooks/useI18n';
import type { useHardware } from '../hooks/useHardware';
import TrayPopup from './TrayPopup';
import { useSettings } from '../hooks/useSettings';
import { useAnalysisLogger } from '../hooks/useAnalysisLogger';
import { ConsentDialog } from '../components/ConsentDialog';
import { MiControlIcon } from '../components/MiControlIcon';
import OnboardingWizard from '../components/OnboardingWizard';
import PrivacyPolicy from './PrivacyPolicy';

// ── Lazy-loaded tab content ──────────────────────────────────────────────────
const OverviewTab = lazy(() => import('./tabs/overview'));
const PerformanceTab = lazy(() => import('./tabs/performance'));
const BatteryTab = lazy(() => import('./tabs/battery'));
const DisplayTab = lazy(() => import('./tabs/display'));
const FanTab = lazy(() => import('./tabs/fan'));
const AudioTab = lazy(() => import('./tabs/audio'));
const CastTab = lazy(() => import('./tabs/cast'));
const IotTab = lazy(() => import('./tabs/iot'));
const WiFiTab = lazy(() => import('./tabs/wifi'));
const EcrDebugTab = lazy(() => import('./tabs/ecrdebug'));
const TouchpadTab = lazy(() => import('./tabs/touchpad'));
const StartupTab = lazy(() => import('./tabs/startup'));
const UpdatesTab = lazy(() => import('./tabs/updates'));
const KeyboardTab = lazy(() => import('./tabs/keyboard'));
const SetupTab = lazy(() => import('./tabs/setup'));
const SettingsTab = lazy(() => import('./tabs/settings'));
const AiAnalysisTab = lazy(() => import('./tabs/ai-analysis'));
const AboutTab = lazy(() => import('./tabs/about'));

interface Props {
  hardware: Hardware;
  activeTab: string;
  onTabChange: (tab: string) => void;
  themeMode: ThemeMode;
  toggleTheme: () => void;
}

const NAV_ITEMS = [
  { id: 'overview', icon: '📊', label: 'nav.overview' },
  { id: 'performance', icon: '⚡', label: 'nav.performance' },
  { id: 'battery', icon: '🔋', label: 'nav.battery' },
  { id: 'display', icon: '🖥️', label: 'nav.display' },
  { id: 'fan', icon: '💨', label: 'nav.fan' },
  { id: 'audio', icon: '🎵', label: 'nav.audio' },
  { id: 'cast', icon: '📺', label: 'nav.cast' },
  { id: 'touchpad', icon: '🖱️', label: 'nav.touchpad' },
  { id: 'iot', icon: '🔌', label: 'nav.iot' },
  { id: 'wifi', icon: '📶', label: 'nav.wifi' },
  { id: 'startup', icon: '🚀', label: 'nav.startup' },
  { id: 'updates', icon: '🔄', label: 'nav.updates' },
  { id: 'keyboard', icon: '⌨️', label: 'nav.keyboard' },
  { id: 'setup', icon: '🔍', label: 'nav.setup' },
  { id: 'ecrdebug', icon: '🔧', label: 'nav.ecrdebug' },
  { id: 'ai_analysis', icon: '🤖', label: 'nav.aiAnalysis' },
  { id: 'settings', icon: '⚙️', label: 'nav.settings' },
  { id: 'about', icon: 'ℹ️', label: 'nav.about' },
] as const;

type Hardware = ReturnType<typeof useHardware>;

interface Props {
  hardware: Hardware;
  activeTab: string;
  onTabChange: (tab: string) => void;
  themeMode: ThemeMode;
  toggleTheme: () => void;
}

function ThemeIcon({ mode }: { mode: ThemeMode }) {
  if (mode === 'light')
    return (
      <svg
        width="14"
        height="14"
        viewBox="0 0 16 16"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
      >
        <circle cx="8" cy="8" r="2.8" />
        <line x1="8" y1="1.5" x2="8" y2="3" />
        <line x1="8" y1="13" x2="8" y2="14.5" />
        <line x1="1.5" y1="8" x2="3" y2="8" />
        <line x1="13" y1="8" x2="14.5" y2="8" />
        <line x1="3.5" y1="3.5" x2="4.5" y2="4.5" />
        <line x1="11.5" y1="11.5" x2="12.5" y2="12.5" />
        <line x1="12.5" y1="3.5" x2="11.5" y2="4.5" />
        <line x1="4.5" y1="11.5" x2="3.5" y2="12.5" />
      </svg>
    );
  if (mode === 'dark')
    return (
      <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
        <path d="M7.5 2a6 6 0 1 0 6.5 8.5A5 5 0 0 1 7.5 2z" />
      </svg>
    );
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
    >
      <circle cx="8" cy="8" r="5.5" />
      <path d="M8 2.5 A5.5 5.5 0 0 1 8 13.5" fill="currentColor" stroke="none" />
    </svg>
  );
}

function getThemeLabel(mode: ThemeMode): string {
  return t(`theme.${mode}` as Parameters<typeof t>[0]);
}

interface SidebarProps {
  activeTab: string;
  onTabChange: (tab: string) => void;
  themeMode: ThemeMode;
  toggleTheme: () => void;
  hardware: Hardware;
  setShowTrayPreview: (v: boolean) => void;
}

const Sidebar = memo(function Sidebar({
  activeTab,
  onTabChange,
  themeMode,
  toggleTheme,
  hardware,
  setShowTrayPreview,
}: SidebarProps) {
  return (
    <nav aria-label="Main navigation" className="sidebar">
      <div className="sidebar-logo">
        <MiControlIcon size={22} />
        MiControl
      </div>
      {NAV_ITEMS.map((item) => (
        <button
          key={item.id}
          className={`sidebar-item ${activeTab === item.id ? 'active' : ''}`}
          onClick={() => onTabChange(item.id)}
          aria-label={t(item.label as Parameters<typeof t>[0])}
        >
          <span className="sidebar-icon" aria-hidden="true">
            {item.icon}
          </span>
          {t(item.label as Parameters<typeof t>[0])}
        </button>
      ))}

      <div className="sidebar-footer">
        {hardware.error && (
          <div
            style={{
              padding: '4px 8px',
              fontSize: 11,
              color: 'var(--error)',
              wordBreak: 'break-word',
            }}
          >
            ⚠️ {hardware.error}
          </div>
        )}
        {hardware.loading && (
          <div style={{ padding: '4px 8px', fontSize: 11, color: 'var(--text-dim)' }}>
            {t('common.loading')}
          </div>
        )}
        <div
          style={{
            padding: '4px 8px',
            fontSize: 10,
            color: 'var(--color-text-muted)',
            opacity: 0.6,
            textAlign: 'center',
          }}
        >
          {t('shortcuts.tabSwitch')}
        </div>
        <button className="theme-toggle" onClick={toggleTheme} title={`Theme: ${themeMode}`}>
          <ThemeIcon mode={themeMode} />
          <span>{getThemeLabel(themeMode)}</span>
        </button>
        {import.meta.env.DEV && (
          <button
            className="theme-toggle"
            onClick={() => setShowTrayPreview(true)}
            title="Preview tray popup"
          >
            <svg
              width="14"
              height="14"
              viewBox="0 0 16 16"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <rect x="1" y="10" width="14" height="5" rx="1" />
              <rect x="1" y="1" width="14" height="7" rx="1" />
              <line x1="4" y1="12.5" x2="4" y2="12.5" strokeWidth="2" />
              <line x1="7" y1="12.5" x2="7" y2="12.5" strokeWidth="2" />
            </svg>
            <span>Tray</span>
          </button>
        )}
      </div>
    </nav>
  );
});

export default function MainWindow({
  hardware,
  activeTab,
  onTabChange,
  themeMode,
  toggleTheme,
}: Props) {
  const aiSettings = useSettings();
  const { onboardingCompleted } = aiSettings.settings;
  const [showTrayPreview, setShowTrayPreview] = useState(false);
  const [showConsentDialog, setShowConsentDialog] = useState(false);
  const [consentChecked, setConsentChecked] = useState(false);

  useEffect(() => {
    if (!consentChecked) {
      aiSettings
        .getTelemetryConsent()
        .then((consent) => {
          setConsentChecked(true);
          if (consent === null) setShowConsentDialog(true);
        })
        .catch(() => setConsentChecked(true));
    }
  }, [aiSettings, consentChecked]);

  const handleConsentAllow = useCallback(async () => {
    await aiSettings.setTelemetryConsent('granted');
    setShowConsentDialog(false);
  }, [aiSettings]);

  const handleConsentDeny = useCallback(async () => {
    await aiSettings.setTelemetryConsent('denied');
    setShowConsentDialog(false);
  }, [aiSettings]);

  const handleOpenPrivacy = useCallback(() => {
    setShowConsentDialog(false);
    onTabChange('privacy');
  }, [onTabChange]);

  const handleFinishOnboarding = useCallback(() => {
    aiSettings.setOnboardingCompleted(true);
  }, [aiSettings]);

  useAnalysisLogger(hardware, aiSettings);

  // ── Keyboard shortcuts: Alt+1..9 to switch tabs ──────────────────────
  useEffect(() => {
    const tabIds = NAV_ITEMS.slice(0, 9).map((item) => item.id);
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.altKey && e.key >= '1' && e.key <= '9') {
        e.preventDefault();
        const tabIndex = parseInt(e.key, 10) - 1;
        if (tabIndex < tabIds.length) {
          onTabChange(tabIds[tabIndex]);
        }
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [onTabChange]);

  function renderTab() {
    switch (activeTab) {
      case 'overview':
        return (
          <OverviewTab
            hw={hardware}
            ai={aiSettings}
            onOpenSettings={() => onTabChange('settings')}
          />
        );
      case 'performance':
        return (
          <PerformanceTab
            hw={hardware}
            ai={aiSettings}
            onOpenSettings={() => onTabChange('settings')}
          />
        );
      case 'battery':
        return <BatteryTab hw={hardware} />;
      case 'display':
        return <DisplayTab hw={hardware} />;
      case 'fan':
        return <FanTab hw={hardware} />;
      case 'audio':
        return <AudioTab hw={hardware} />;
      case 'cast':
        return <CastTab />;
      case 'iot':
        return <IotTab />;
      case 'wifi':
        return <WiFiTab />;
      case 'ecrdebug':
        return <EcrDebugTab />;
      case 'touchpad':
        return <TouchpadTab hw={hardware} />;
      case 'startup':
        return <StartupTab />;
      case 'updates':
        return <UpdatesTab hw={hardware} />;
      case 'keyboard':
        return <KeyboardTab />;
      case 'setup':
        return <SetupTab hw={hardware} />;
      case 'ai_analysis':
        return (
          <AiAnalysisTab
            hw={hardware}
            ai={aiSettings}
            onOpenSettings={() => onTabChange('settings')}
          />
        );
      case 'settings':
        return <SettingsTab ai={aiSettings} onTabChange={onTabChange} />;
      case 'privacy':
        return <PrivacyPolicy />;
      case 'about':
        return <AboutTab />;
      default:
        return (
          <OverviewTab
            hw={hardware}
            ai={aiSettings}
            onOpenSettings={() => onTabChange('settings')}
          />
        );
    }
  }

  return (
    <div className="app-layout">
      <Sidebar
        activeTab={activeTab}
        onTabChange={onTabChange}
        themeMode={themeMode}
        toggleTheme={toggleTheme}
        hardware={hardware}
        setShowTrayPreview={setShowTrayPreview}
      />

      <main className="content-area">
        <div className="tab-content" key={activeTab}>
          <Suspense
            fallback={
              <div className="loading-spinner" role="status" aria-live="polite">
                {t('common.loading')}
              </div>
            }
          >
            {renderTab()}
          </Suspense>
        </div>

        {/* Watermark */}
        <div
          style={{
            position: 'fixed',
            bottom: 10,
            right: 14,
            fontSize: 10,
            color: 'var(--color-text-muted, oklch(50% 0 0))',
            opacity: 0.55,
            userSelect: 'none',
            pointerEvents: 'none',
            display: 'flex',
            alignItems: 'center',
            gap: 4,
            fontFamily: 'var(--font-mono, monospace)',
          }}
        >
          <span>By: Marcos Freitas</span>
          <a
            href="https://github.com/Freitas-MA"
            target="_blank"
            rel="noopener noreferrer"
            title="github.com/Freitas-MA"
            style={{
              color: 'inherit',
              textDecoration: 'none',
              pointerEvents: 'auto',
              display: 'flex',
              alignItems: 'center',
            }}
          >
            <svg width="11" height="11" viewBox="0 0 16 16" fill="currentColor" aria-label="GitHub">
              <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8z" />
            </svg>
          </a>
        </div>
      </main>

      {/* Tray popup preview overlay */}
      {showTrayPreview && (
        <div
          style={{
            position: 'fixed',
            inset: 0,
            zIndex: 999,
            background: 'oklch(0% 0 0 / 0.35)',
            backdropFilter: 'blur(2px)',
          }}
          onClick={() => setShowTrayPreview(false)}
        >
          <div
            style={{
              position: 'absolute',
              bottom: 48,
              right: 16,
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'flex-end',
              gap: 6,
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <div style={{ fontSize: 10, color: 'oklch(90% 0 0 / 0.6)' }}>
              Preview — click outside to close
            </div>
            <TrayPopup hardware={hardware} />
            <div
              style={{
                width: 40,
                height: 40,
                borderRadius: '50%',
                background: 'var(--accent)',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                cursor: 'pointer',
                boxShadow: '0 2px 12px var(--accent-glow)',
                fontSize: 18,
                alignSelf: 'flex-end',
              }}
              title="MiControl tray icon"
            >
              🖥️
            </div>
          </div>
        </div>
      )}

      {/* First-run onboarding wizard */}
      {!onboardingCompleted && <OnboardingWizard onFinish={handleFinishOnboarding} />}

      {/* Telemetry consent dialog */}
      {showConsentDialog && (
        <ConsentDialog
          onAllow={handleConsentAllow}
          onDeny={handleConsentDeny}
          onOpenPrivacy={handleOpenPrivacy}
        />
      )}
    </div>
  );
}
