import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t } from "../hooks/useI18n";

interface Props {
  autostart: boolean;
}

export default function StartupManager({ autostart }: Props) {
  const [enabled, setEnabled] = useState(autostart);
  const [saving, setSaving] = useState(false);

  const handleToggle = async (value: boolean) => {
    setSaving(true);
    try {
      await invoke("set_autostart", { enabled: value });
      setEnabled(value);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="card">
      <div className="card-title">{t("startup.title")}</div>

      <div className="toggle" onClick={() => !saving && void handleToggle(!enabled)}>
        <div className="toggle-info">
          <div className="toggle-name">{t("startup.runAtStartup")}</div>
          <div className="toggle-desc">{t("startup.description")}</div>
        </div>
        <label className="toggle-switch" onClick={(e) => e.stopPropagation()}>
          <input
            type="checkbox"
            checked={enabled}
            disabled={saving}
            onChange={(e) => void handleToggle(e.target.checked)}
          />
          <span className="toggle-track" />
          <span className="toggle-knob" />
        </label>
      </div>
    </div>
  );
}
