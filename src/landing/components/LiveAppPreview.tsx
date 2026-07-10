import { useState, useCallback, useEffect } from 'react';
import MainWindow from '../../pages/MainWindow';
import { useHardware } from '../../hooks/useHardware';
import { ToastProvider } from '../../contexts/ToastContext';
import { ErrorBoundary } from '../../components/ErrorBoundary';

// ── Pre-set localStorage so onboarding/consent dialogs don't appear ──────────
// This runs at module load time, BEFORE any component renders, so useSettings
// will read the correct values on its first render. We also rely on the
// mocked Tauri credential store (src/mocks/tauri-api.ts) which returns
// 'granted' for telemetry_consent, so the consent panel shows as accepted.
try {
  const key = 'micontrol_settings_v2';
  const raw = localStorage.getItem(key);
  const settings = raw ? JSON.parse(raw) : {};
  settings.onboardingCompleted = true;
  localStorage.setItem(key, JSON.stringify(settings));
} catch {
  /* ignore */
}

// ── Tab definitions (mirrors MainWindow NAV_ITEMS, minus dev-only ecrdebug) ──

export interface PreviewTab {
  id: string;
  icon: string;
  label: string;
  title: string;
  description: string;
}

export const PREVIEW_TABS: PreviewTab[] = [
  {
    id: 'overview',
    icon: '📊',
    label: 'Overview',
    title: 'Full System Overview',
    description:
      'A real-time dashboard showing CPU, GPU, memory, and storage at a glance. Hardware sensors stream live data through the Rust backend with sub-millisecond latency.',
  },
  {
    id: 'performance',
    icon: '⚡',
    label: 'Performance',
    title: 'Performance Monitoring',
    description:
      'Track CPU frequencies, core utilization, and thermal throttling in real time. Identify bottlenecks and optimize your workflow with detailed per-core metrics.',
  },
  {
    id: 'battery',
    icon: '🔋',
    label: 'Battery',
    title: 'Battery Health & Stats',
    description:
      'Monitor charge cycles, wear level, and power draw. Smart charging thresholds extend your battery lifespan with configurable start and stop limits.',
  },
  {
    id: 'display',
    icon: '🖥️',
    label: 'Display',
    title: 'Display Calibration',
    description:
      'Fine-tune brightness, color temperature, and refresh rate. Night light scheduling and per-profile display presets adapt your screen to any environment.',
  },
  {
    id: 'fan',
    icon: '💨',
    label: 'Fan Control',
    title: 'Custom Fan Curves',
    description:
      'Create custom fan curves with temperature-triggered speed profiles. Silent, balanced, or performance modes — your Xiaomi Notebook stays cool under any load.',
  },
  {
    id: 'audio',
    icon: '🎵',
    label: 'Audio',
    title: 'Audio Enhancement',
    description:
      'Equalizer presets, spatial audio tuning, and device-specific output profiles. Enhance your listening experience with real-time audio processing.',
  },
  {
    id: 'cast',
    icon: '📺',
    label: 'Cast',
    title: 'Screen Casting',
    description:
      'Wireless display casting with Miracast support. Configure resolution, latency mode, and multi-monitor setups directly from the app.',
  },
  {
    id: 'touchpad',
    icon: '🖱️',
    label: 'Touchpad',
    title: 'Touchpad Settings',
    description:
      'Adjust sensitivity, palm rejection, and gesture mappings. Multi-finger swipe and tap configurations tailored to your Xiaomi touchpad hardware.',
  },
  {
    id: 'iot',
    icon: '🔌',
    label: 'IoT',
    title: 'IoT Device Hub',
    description:
      'Connect and manage smart home devices. Monitor sensors, control actuators, and automate routines — all integrated into your notebook control center.',
  },
  {
    id: 'wifi',
    icon: '📶',
    label: 'WiFi',
    title: 'WiFi Management',
    description:
      'Scan networks, analyze signal strength, and manage saved profiles. Advanced diagnostics help you find the best channel and optimize your connection.',
  },
  {
    id: 'startup',
    icon: '🚀',
    label: 'Startup',
    title: 'Startup Manager',
    description:
      'Control which apps launch at boot. Enable, disable, or delay startup entries to reduce boot time and optimize system resource allocation.',
  },
  {
    id: 'updates',
    icon: '🔄',
    label: 'Updates',
    title: 'System Updates',
    description:
      'Check for driver updates, BIOS releases, and app patches. Automated update detection keeps your Xiaomi Notebook running the latest firmware.',
  },
  {
    id: 'keyboard',
    icon: '⌨️',
    label: 'Keyboard',
    title: 'Keyboard Customization',
    description:
      'Remap keys, configure backlight brightness, and set up macro profiles. Per-application keyboard layouts switch automatically as you work.',
  },
  {
    id: 'setup',
    icon: '🔍',
    label: 'Setup',
    title: 'Initial Setup Wizard',
    description:
      'Guided first-run configuration for new installations. Hardware detection, driver verification, and optimal preset selection in a single streamlined flow.',
  },
  {
    id: 'ai_analysis',
    icon: '🤖',
    label: 'AI Analysis',
    title: 'AI-Powered Diagnostics',
    description:
      'Machine learning models analyze hardware patterns to predict failures and suggest optimizations. Proactive alerts keep your system healthy.',
  },
  {
    id: 'settings',
    icon: '⚙️',
    label: 'Settings',
    title: 'App Settings',
    description:
      'Configure telemetry, appearance, language, and notifications. Export and import profiles to sync your preferences across devices.',
  },
  {
    id: 'about',
    icon: 'ℹ️',
    label: 'About',
    title: 'About miControl',
    description:
      'Version info, changelog, and credits. Built with Tauri 2 and React — open source and community-driven for Xiaomi Notebook owners.',
  },
];

// ── Theme (fixed dark for the landing preview) ───────────────────────────────

type ThemeMode = 'auto' | 'light' | 'dark';

function useFixedDarkTheme() {
  const [mode] = useState<ThemeMode>('dark');
  useEffect(() => {
    document.documentElement.setAttribute('data-theme', 'dark');
  }, []);
  const toggleTheme = useCallback(() => {}, []);
  return { themeMode: mode, toggleTheme };
}

// ── Main component ────────────────────────────────────────────────────────────

interface LiveAppPreviewProps {
  activeTab: string;
  onTabChange: (tab: string) => void;
}

function LiveAppPreviewInner({ activeTab, onTabChange }: LiveAppPreviewProps) {
  const hardware = useHardware();
  const { themeMode, toggleTheme } = useFixedDarkTheme();

  return (
    <MainWindow
      hardware={hardware}
      activeTab={activeTab}
      onTabChange={onTabChange}
      themeMode={themeMode}
      toggleTheme={toggleTheme}
    />
  );
}

export function LiveAppPreview(props: LiveAppPreviewProps) {
  return (
    <ErrorBoundary>
      <ToastProvider>
        <LiveAppPreviewInner {...props} />
      </ToastProvider>
    </ErrorBoundary>
  );
}
