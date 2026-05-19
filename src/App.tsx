import { useState, useEffect } from "react";
import MainWindow from "./pages/MainWindow";
import TrayPopup from "./pages/TrayPopup";
import BrightnessOsd from "./components/BrightnessOsd";
import { useHardware } from "./hooks/useHardware";
import { useLanguage } from "./hooks/useI18n";
import { ToastProvider } from "./contexts/ToastContext";

export type ThemeMode = "auto" | "light" | "dark";

function useTheme() {
  const [mode, setMode] = useState<ThemeMode>(
    () => (localStorage.getItem("micontrol_theme") as ThemeMode) ?? "auto"
  );

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", mode);
    localStorage.setItem("micontrol_theme", mode);
  }, [mode]);

  function toggleTheme() {
    setMode((m) => (m === "auto" ? "light" : m === "light" ? "dark" : "auto"));
  }

  return { themeMode: mode, toggleTheme };
}

// Tauri passes ?window=tray, ?window=main, or ?window=brightness-osd in the URL
const windowType = new URLSearchParams(window.location.search).get("window");
const isTrayPopup     = windowType === "tray";
const isBrightnessOsd = windowType === "brightness-osd";

export default function App() {
  // The brightness OSD window needs a transparent body — no providers needed.
  if (isBrightnessOsd) {
    // Apply transparent background so the glass card floats over the desktop.
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";
    return <BrightnessOsd />;
  }

  const hardware = useHardware();
  const [activeTab, setActiveTab] = useState(
    () => localStorage.getItem("micontrol_active_tab") ?? "overview"
  );

  function handleTabChange(tab: string) {
    setActiveTab(tab);
    localStorage.setItem("micontrol_active_tab", tab);
  }

  const { themeMode, toggleTheme } = useTheme();
  // Subscribe to language changes so the entire tree re-renders on locale switch
  useLanguage();

  useEffect(() => {
    if (isTrayPopup) document.documentElement.classList.add("tray-window");
  }, []);

  if (isTrayPopup) {
    return (
      <ToastProvider>
        <TrayPopup hardware={hardware} />
      </ToastProvider>
    );
  }

  return (
    <ToastProvider>
      <MainWindow
        hardware={hardware}
        activeTab={activeTab}
        onTabChange={handleTabChange}
        themeMode={themeMode}
        toggleTheme={toggleTheme}
      />
    </ToastProvider>
  );
}
