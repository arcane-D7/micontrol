import { useState, useEffect } from "react";
import MainWindow from "./pages/MainWindow";
import TrayPopup from "./pages/TrayPopup";
import { useHardware } from "./hooks/useHardware";
import { useLanguage } from "./hooks/useI18n";

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

// Tauri passes ?window=tray or ?window=main in the URL
const windowType = new URLSearchParams(window.location.search).get("window");
const isTrayPopup = windowType === "tray";

export default function App() {
  const hardware = useHardware();
  const [activeTab, setActiveTab] = useState("overview");
  const { themeMode, toggleTheme } = useTheme();
  // Subscribe to language changes so the entire tree re-renders on locale switch
  useLanguage();

  if (isTrayPopup) {
    return <TrayPopup hardware={hardware} />;
  }

  return (
    <MainWindow
      hardware={hardware}
      activeTab={activeTab}
      onTabChange={setActiveTab}
      themeMode={themeMode}
      toggleTheme={toggleTheme}
    />
  );
}
