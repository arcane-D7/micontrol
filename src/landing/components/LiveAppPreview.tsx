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
      'A real-time dashboard showing CPU, GPU, memory, and storage at a glance, with readings supplied by the local hardware service.',
  },
  {
    id: 'taskmgr',
    icon: '📈',
    label: 'Task Manager',
    title: 'Task Manager',
    description:
      'Inspect process CPU, GPU, NPU, memory, and network usage, then filter or sort the list when you need to find a busy process.',
  },
  {
    id: 'performance',
    icon: '⚡',
    label: 'Performance',
    title: 'Performance Monitoring',
    description:
      'Track CPU frequencies, utilization, temperatures, and power information in real time to understand the current workload.',
  },
  {
    id: 'sysopt',
    icon: '🚀',
    label: 'System Optimization',
    title: 'System Optimization',
    description:
      'Review the system optimization and debloating tools available in miControl before applying changes to Windows.',
  },
  {
    id: 'battery',
    icon: '🔋',
    label: 'Battery',
    title: 'Battery Health & Stats',
    description:
      'Monitor battery level, health information, power draw, and charging protection settings from one focused view.',
  },
  {
    id: 'display',
    icon: '🖥️',
    label: 'Display',
    title: 'Display Calibration',
    description:
      'Adjust brightness, HDR, refresh rate, adaptive refresh, and supported display options from the control center.',
  },
  {
    id: 'fan',
    icon: '💨',
    label: 'Fan Control',
    title: 'Custom Fan Curves',
    description:
      'Read fan speed and select the cooling profile that fits the current workload, with hardware-aware controls.',
  },
  {
    id: 'faceUnlock',
    icon: '😀',
    label: 'Face Unlock',
    title: 'Face Unlock',
    description:
      'Set up and manage the face unlock workflow when the required camera, Windows account, and local configuration are available.',
  },
  {
    id: 'audio',
    icon: '🎵',
    label: 'Audio',
    title: 'Audio Enhancement',
    description:
      'Keep audio controls close to the rest of your notebook settings, with the available device controls in one place.',
  },
  {
    id: 'cast',
    icon: '📺',
    label: 'Cast',
    title: 'Screen Casting',
    description:
      'Open the screen-casting controls alongside the rest of your notebook tools, without leaving the control center.',
  },
  {
    id: 'touchpad',
    icon: '🖱️',
    label: 'Touchpad',
    title: 'Touchpad Settings',
    description:
      'Configure the touchpad options exposed by your Xiaomi notebook and keep everyday input settings in one place.',
  },
  {
    id: 'iot',
    icon: '🔌',
    label: 'IoT',
    title: 'IoT Device Hub',
    description:
      'Inspect compatible IoT hardware and device status from the same control center used for your notebook.',
  },
  {
    id: 'wifi',
    icon: '📶',
    label: 'WiFi',
    title: 'WiFi Management',
    description:
      'Inspect WiFi adapter information and use the network controls available on the current Windows system.',
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
    id: 'system',
    icon: '🔧',
    label: 'System',
    title: 'System & Drivers',
    description:
      'Review hardware discovery, driver details, BIOS information, and the system controls exposed by the connected notebook.',
  },
  {
    id: 'keyboard',
    icon: '⌨️',
    label: 'Keyboard',
    title: 'Keyboard Customization',
    description:
      'Configure supported keyboard and backlight settings for a more comfortable Xiaomi notebook setup.',
  },
  {
    id: 'ai_analysis',
    icon: '🤖',
    label: 'AI Analysis',
    title: 'AI-Powered Diagnostics',
    description:
      'Review hardware information and available AI-assisted analysis workflows when the feature is configured.',
  },
  {
    id: 'security',
    icon: '🛡️',
    label: 'Security',
    title: 'Security Tools',
    description:
      'Open the local security tools and review the checks available for the current Windows installation.',
  },
  {
    id: 'crossDevice',
    icon: '📱',
    label: 'Cross Device',
    title: 'Cross Device',
    description:
      'Keep the available phone and notebook integration tools close to the hardware controls in one app.',
  },
  {
    id: 'color',
    icon: '🎨',
    label: 'Color',
    title: 'Color Controls',
    description:
      'Review the color and display profile controls exposed by the connected notebook and Windows.',
  },
  {
    id: 'cleanup',
    icon: '🧹',
    label: 'Cleanup',
    title: 'System Cleanup',
    description:
      'Review cleanup actions designed to help remove selected system clutter before applying any change.',
  },
  {
    id: 'settings',
    icon: '⚙️',
    label: 'Settings',
    title: 'App Settings',
    description:
      'Configure appearance, language, telemetry consent, notifications, and other preferences for the desktop app.',
  },
  {
    id: 'about',
    icon: 'ℹ️',
    label: 'About',
    title: 'About miControl',
    description:
      'Review version information, changelog, and credits for the Tauri 2 and React application built for Xiaomi Notebook owners.',
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
