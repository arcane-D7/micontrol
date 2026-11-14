import { useState, useEffect, useCallback, useRef, lazy, Suspense, memo } from 'react';
import type { ThemeMode } from '../App';
import { t } from '../hooks/useI18n';
import type { useHardware } from '../hooks/useHardware';
import TrayPopup from './TrayPopup';
import { useSettings } from '../hooks/useSettings';
import { usePowerProfiles } from '../hooks/usePowerProfiles';
import { useAnalysisLogger } from '../hooks/useAnalysisLogger';
import { useAutoUpdate } from '../hooks/useAutoUpdate';
import { ConsentDialog } from '../components/ConsentDialog';
import { ErrorBoundary } from '../components/ErrorBoundary';
import { MiControlIcon } from '../components/MiControlIcon';
import OnboardingWizard from '../components/OnboardingWizard';
import PrivacyPolicy from './PrivacyPolicy';
import { Icon, type IconName } from '../components/ui/icons';

// ── Lazy-loaded tab content ──────────────────────────────────────────────────
const OverviewTab = lazy(() => import('./tabs/overview'));
const TaskManagerTab = lazy(() => import('./tabs/taskmanager'));
const PerformanceTab = lazy(() => import('./tabs/performance'));
const SysOptTab = lazy(() => import('./tabs/sys_opt'));
const BatteryTab = lazy(() => import('./tabs/battery'));
const DisplayTab = lazy(() => import('./tabs/display'));
const FanTab = lazy(() => import('./tabs/fan'));
const AudioTab = lazy(() => import('./tabs/audio'));
const CastTab = lazy(() => import('./tabs/cast'));
const IotTab = lazy(() => import('./tabs/iot'));
const FaceUnlockTab = lazy(() => import('./tabs/face'));
const WiFiTab = lazy(() => import('./tabs/wifi'));
const EcrDebugTab = lazy(() => import('./tabs/ecrdebug'));
const TouchpadTab = lazy(() => import('./tabs/touchpad'));
const StartupTab = lazy(() => import('./tabs/startup'));
const SystemTab = lazy(() => import('./tabs/system'));
const KeyboardTab = lazy(() => import('./tabs/keyboard'));
const SettingsTab = lazy(() => import('./tabs/settings'));
const AiAnalysisTab = lazy(() => import('./tabs/ai-analysis'));
const SecurityTab = lazy(() => import('./tabs/security'));
const CrossDeviceTab = lazy(() => import('./tabs/crossdevice'));
const ColorTab = lazy(() => import('./tabs/color'));
const CleanupTab = lazy(() => import('./tabs/cleanup'));
const AboutTab = lazy(() => import('./tabs/about'));

type Hardware = ReturnType<typeof useHardware>;

interface Props {
  hardware: Hardware;
  activeTab: string;
  onTabChange: (tab: string) => void;
  themeMode: ThemeMode;
  toggleTheme: () => void;
}

type NavKey = Parameters<typeof t>[0];

interface NavItem {
  id: string;
  icon: IconName;
  label: NavKey;
}

interface NavGroup {
  id: string;
  label: NavKey;
  items: NavItem[];
}

/**
 * Navigation grouped by task domain (audit 2026-11-07, finding #3: 23 flat
 * destinations cost 23 binary decisions and pushed `Definições` below the
 * fold). Five groups, each labelled; the last one is pinned outside the
 * scroll area so Settings is always reachable at 950×660 and at the 600×560
 * minimum.
 */
const NAV_GROUPS: NavGroup[] = [
  {
    id: 'system',
    label: 'nav.groups.system',
    items: [
      { id: 'overview', icon: 'chart-bar', label: 'nav.overview' },
      { id: 'taskmgr', icon: 'chart-line', label: 'nav.taskmgr' },
      { id: 'system', icon: 'wrench', label: 'nav.system' },
      { id: 'startup', icon: 'rocket', label: 'nav.startup' },
      { id: 'security', icon: 'shield', label: 'nav.security' },
      { id: 'cleanup', icon: 'broom', label: 'nav.cleanup' },
      { id: 'ai_analysis', icon: 'bot', label: 'nav.aiAnalysis' },
    ],
  },
  {
    id: 'energy',
    label: 'nav.groups.energy',
    items: [
      { id: 'battery', icon: 'battery', label: 'nav.battery' },
      { id: 'performance', icon: 'bolt', label: 'nav.performance' },
      { id: 'fan', icon: 'wind', label: 'nav.fan' },
      { id: 'sysopt', icon: 'rocket', label: 'nav.sysOpt' },
    ],
  },
  {
    id: 'screen',
    label: 'nav.groups.screen',
    items: [
      { id: 'display', icon: 'monitor', label: 'nav.display' },
      { id: 'color', icon: 'palette', label: 'nav.color' },
      { id: 'audio', icon: 'music', label: 'nav.audio' },
      { id: 'cast', icon: 'tv', label: 'nav.cast' },
    ],
  },
  {
    id: 'devices',
    label: 'nav.groups.devices',
    items: [
      { id: 'crossDevice', icon: 'smartphone', label: 'nav.crossDevice' },
      { id: 'iot', icon: 'plug', label: 'nav.iot' },
      { id: 'wifi', icon: 'wifi', label: 'nav.wifi' },
      { id: 'keyboard', icon: 'keyboard', label: 'nav.keyboard' },
      { id: 'touchpad', icon: 'mouse', label: 'nav.touchpad' },
      { id: 'faceUnlock', icon: 'smile', label: 'nav.faceUnlock' },
    ],
  },
];

const PINNED_GROUP: NavGroup = {
  id: 'settings',
  label: 'nav.groups.settings',
  items: [
    { id: 'settings', icon: 'gear', label: 'nav.settings' },
    { id: 'about', icon: 'info', label: 'nav.about' },
    ...(import.meta.env.DEV
      ? [{ id: 'ecrdebug', icon: 'wrench' as IconName, label: 'nav.ecrdebug' as NavKey }]
      : []),
  ],
};

const NAV_ITEMS: NavItem[] = [...NAV_GROUPS.flatMap((g) => g.items), ...PINNED_GROUP.items];

/** Alt+1..9 keeps the pre-grouping order — muscle memory must survive the
 *  reorganisation (audit §4.5: memorability is the weakest axis). */
const SHORTCUT_TABS = [
  'overview',
  'taskmgr',
  'performance',
  'sysopt',
  'battery',
  'display',
  'fan',
  'audio',
  'cast',
];

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
  const [query, setQuery] = useState('');
  const activeRef = useRef<HTMLButtonElement | null>(null);
  const q = query.trim().toLowerCase();

  // Keep the active destination in view: the list scrolls internally, so a
  // tab change from a shortcut or the tray must not leave the marker offscreen
  // (audit §4.3 #7 — the active item used to stay below the fold).
  useEffect(() => {
    if (!q) activeRef.current?.scrollIntoView({ block: 'nearest' });
  }, [activeTab, q]);

  const renderItem = (item: NavItem) => {
    const active = activeTab === item.id;
    return (
      <button
        key={item.id}
        ref={active ? activeRef : undefined}
        className={`sidebar-item ${active ? 'active' : ''}`}
        onClick={() => onTabChange(item.id)}
        aria-current={active ? 'page' : undefined}
        title={t(item.label)}
      >
        <span className="sidebar-icon" aria-hidden="true">
          <Icon name={item.icon} size={18} />
        </span>
        <span className="sidebar-item-label">{t(item.label)}</span>
      </button>
    );
  };

  const matches = (item: NavItem) => !q || t(item.label).toLowerCase().includes(q);
  const results = q ? NAV_ITEMS.filter(matches) : null;
  // While filtering, every match (pinned ones included) is rendered in the
  // single result list — keeping the pinned group would duplicate Settings.
  const pinnedItems = q ? [] : PINNED_GROUP.items;

  return (
    <nav aria-label={t('nav.label')} className="sidebar">
      <div className="sidebar-logo">
        <MiControlIcon size={16} />
        MiControl
      </div>

      <div className="sidebar-search">
        <svg
          className="sidebar-search-icon"
          width="12"
          height="12"
          viewBox="0 0 16 16"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinecap="round"
          aria-hidden="true"
        >
          <circle cx="7" cy="7" r="4.5" />
          <line x1="10.5" y1="10.5" x2="14" y2="14" />
        </svg>
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') setQuery('');
          }}
          placeholder={t('nav.search')}
          aria-label={t('nav.search')}
          spellCheck={false}
        />
      </div>

      <div className="sidebar-scroll">
        {results ? (
          results.length > 0 ? (
            results.map(renderItem)
          ) : (
            <div className="sidebar-empty" role="status">
              {t('nav.noResults', { query: query.trim() })}
            </div>
          )
        ) : (
          NAV_GROUPS.map((group) => (
            <div key={group.id} className="sidebar-group" role="group" aria-label={t(group.label)}>
              <div className="sidebar-group-label" aria-hidden="true">
                {t(group.label)}
              </div>
              {group.items.map(renderItem)}
            </div>
          ))
        )}
      </div>

      {pinnedItems.length > 0 && (
        <div className="sidebar-pinned">
          <div className="sidebar-group" role="group" aria-label={t(PINNED_GROUP.label)}>
            <div className="sidebar-group-label" aria-hidden="true">
              {t(PINNED_GROUP.label)}
            </div>
            {pinnedItems.map(renderItem)}
          </div>
        </div>
      )}

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
            <Icon name="alert-triangle" size={16} /> {hardware.error}
          </div>
        )}
        {hardware.loading && (
          <div style={{ padding: '4px 8px', fontSize: 11, color: 'var(--text-muted)' }}>
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
        <button
          className="theme-toggle"
          onClick={toggleTheme}
          title={t('main.themeTitle', { theme: themeMode })}
        >
          <ThemeIcon mode={themeMode} />
          <span>{getThemeLabel(themeMode)}</span>
        </button>
        {import.meta.env.DEV && (
          <button
            className="theme-toggle"
            onClick={() => setShowTrayPreview(true)}
            title={t('main.previewTray')}
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
            <span>{t('main.tray')}</span>
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
  // P2-6: the Power Profiles toggle must outlive tab switches. The hook lives
  // in the shell (mounted once), not in the Performance tab, and the actual
  // AC/DC application happens in Rust (`hw::scenario_rules`) — the UI only
  // mirrors config + subscribes to status.
  const powerProfiles = usePowerProfiles(aiSettings.settings, aiSettings.updateKey);
  const appUpdate = useAutoUpdate();
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
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.altKey && e.key >= '1' && e.key <= '9') {
        e.preventDefault();
        const tabIndex = parseInt(e.key, 10) - 1;
        const tabId = SHORTCUT_TABS[tabIndex];
        if (tabId) onTabChange(tabId);
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
            onNavigate={onTabChange}
          />
        );
      case 'taskmgr':
        return <TaskManagerTab hw={hardware} />;
      case 'performance':
        return (
          <PerformanceTab
            hw={hardware}
            ai={aiSettings}
            pp={powerProfiles}
            onOpenSettings={() => onTabChange('settings')}
          />
        );
      case 'sysopt':
        return <SysOptTab />;
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
      case 'faceUnlock':
        return <FaceUnlockTab />;
      case 'wifi':
        return <WiFiTab />;
      case 'ecrdebug':
        return <EcrDebugTab />;
      case 'touchpad':
        return <TouchpadTab hw={hardware} />;
      case 'startup':
        return <StartupTab />;
      case 'system':
        return <SystemTab hw={hardware} />;
      case 'keyboard':
        return <KeyboardTab />;
      case 'security':
        return <SecurityTab />;
      case 'crossDevice':
        return <CrossDeviceTab />;
      case 'color':
        return <ColorTab />;
      case 'cleanup':
        return <CleanupTab />;
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
        return (
          <AboutTab
            appUpdateState={appUpdate.state}
            appUpdateInfo={appUpdate.updateInfo}
            appUpdateProgress={appUpdate.progress}
            appUpdateError={appUpdate.errorMsg}
            onCheckAppUpdate={appUpdate.checkForUpdate}
            onInstallAppUpdate={appUpdate.downloadAndInstall}
            onDismissAppUpdate={appUpdate.dismiss}
          />
        );
      default:
        return (
          <OverviewTab
            hw={hardware}
            ai={aiSettings}
            onOpenSettings={() => onTabChange('settings')}
            onNavigate={onTabChange}
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
          <ErrorBoundary compact>
            <Suspense
              fallback={
                <div className="loading-spinner" role="status" aria-live="polite">
                  {t('common.loading')}
                </div>
              }
            >
              {renderTab()}
            </Suspense>
          </ErrorBoundary>
        </div>

        {/* Watermark */}
        <div
          style={{
            position: 'fixed',
            bottom: 10,
            right: 14,
            fontSize: 10,
            color: 'var(--color-text-muted, oklch(50% 0 0))',
            opacity: 0.75,
            userSelect: 'none',
            pointerEvents: 'none',
            display: 'flex',
            alignItems: 'center',
            gap: 4,
            fontFamily: 'var(--font-mono, monospace)',
          }}
        >
          <span>{t('main.by', { name: 'Marcos Freitas' })}</span>
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
              {t('main.previewHint')}
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
              title={t('main.trayIconTitle')}
            >
              <Icon name="monitor" size={16} />
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
