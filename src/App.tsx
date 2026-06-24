import { useState, useEffect } from 'react';
import MainWindow from './pages/MainWindow';
import TrayPopup from './pages/TrayPopup';
import BrightnessOsd from './components/BrightnessOsd';
import { ErrorBoundary } from './components/ErrorBoundary';
import { useHardware } from './hooks/useHardware';
import { useLanguage } from './hooks/useI18n';
import { ToastProvider } from './contexts/ToastContext';

export type ThemeMode = 'auto' | 'light' | 'dark';

function useTheme() {
  const [mode, setMode] = useState<ThemeMode>(
    () => (localStorage.getItem('micontrol_theme') as ThemeMode) ?? 'auto',
  );

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', mode);
    localStorage.setItem('micontrol_theme', mode);
  }, [mode]);

  function toggleTheme() {
    setMode((m) => (m === 'auto' ? 'light' : m === 'light' ? 'dark' : 'auto'));
  }

  return { themeMode: mode, toggleTheme };
}

// Tauri passes ?window=tray, ?window=main, or ?window=brightness-osd in the URL
const windowType = new URLSearchParams(window.location.search).get('window');
const isTrayPopup = windowType === 'tray';
const isBrightnessOsd = windowType === 'brightness-osd';

// Apply window-type setup synchronously at module level (before any render/paint)
// so the WebView2 compositor never sees an opaque background on frame 1.
if (isBrightnessOsd) {
  document.documentElement.style.background = 'transparent';
  document.body.style.background = 'transparent';
}
if (isTrayPopup) {
  document.documentElement.style.background = 'transparent';
  document.body.style.background = 'transparent';
  // Add class immediately so CSS overrides (solid surfaces, no blur) apply on frame 1.
  document.documentElement.classList.add('tray-window');
}

export default function App() {
  // The brightness OSD window needs a transparent body — no providers needed.
  if (isBrightnessOsd) {
    return <BrightnessOsd />;
  }

  const hardware = useHardware();
  const [activeTab, setActiveTab] = useState(
    () => localStorage.getItem('micontrol_active_tab') ?? 'overview',
  );

  function handleTabChange(tab: string) {
    setActiveTab(tab);
    localStorage.setItem('micontrol_active_tab', tab);
  }

  const { themeMode, toggleTheme } = useTheme();
  // Subscribe to language changes so the entire tree re-renders on locale switch
  useLanguage();

  // tray-window class is already added synchronously above; useEffect is redundant.

  if (isTrayPopup) {
    return (
      <ErrorBoundary>
        <ToastProvider>
          <TrayPopup hardware={hardware} />
        </ToastProvider>
      </ErrorBoundary>
    );
  }

  return (
    <ErrorBoundary>
      <ToastProvider>
        <MainWindow
          hardware={hardware}
          activeTab={activeTab}
          onTabChange={handleTabChange}
          themeMode={themeMode}
          toggleTheme={toggleTheme}
        />
      </ToastProvider>
    </ErrorBoundary>
  );
}
