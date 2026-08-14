import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { PageHeader } from './PageHeader';
import ToggleSwitch from '../../components/ToggleSwitch';
import { t } from '../../hooks/useI18n';
import { useToast } from '../../contexts/ToastContext';

// ── Types matching Rust structs ──────────────────────────────────────────────

interface ColorProfileInfo {
  device_name: string;
  current_profile: string | null;
  installed_profiles: string[];
  hardware_calibration: boolean;
}

interface ColorCalibrationStatus {
  displays: ColorProfileInfo[];
  eye_protection_active: boolean;
  gamma_intensity: number;
}

interface EyeProtectionStatus {
  enabled: boolean;
  intensity: number;
}

// ── Component ────────────────────────────────────────────────────────────────

export default function ColorTab() {
  const [status, setStatus] = useState<ColorCalibrationStatus | null>(null);
  const [eyeProtection, setEyeProtection] = useState<EyeProtectionStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [selectedProfiles, setSelectedProfiles] = useState<Record<string, string>>({});
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [eyeToggling, setEyeToggling] = useState(false);
  const { addToast } = useToast();

  const fetchStatus = useCallback(async () => {
    try {
      const s = await invoke<ColorCalibrationStatus>('get_color_status');
      setStatus(s);
      // Initialize selected profiles
      const init: Record<string, string> = {};
      for (const d of s.displays) {
        init[d.device_name] = d.current_profile || '';
      }
      setSelectedProfiles(init);
    } catch (e) {
      setStatus(null);
      setErrorMsg(String(e));
    }
    // Fetch eye protection status
    try {
      const ep = await invoke<EyeProtectionStatus>('get_eye_protection');
      setEyeProtection(ep);
    } catch {
      setEyeProtection(null);
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    void fetchStatus();
  }, [fetchStatus]);

  const handleLoadProfile = async (display: string) => {
    const profile = selectedProfiles[display];
    if (!profile) return;
    try {
      await invoke('load_icc_profile', { display, profilePath: profile });
      void fetchStatus();
    } catch (e) {
      const msg =
        typeof e === 'object' && e !== null && 'message' in e
          ? String((e as { message: unknown }).message)
          : String(e);
      setErrorMsg(`${t('color.loadProfile')}: ${msg}`);
    }
  };

  const handleUnloadProfile = async (display: string) => {
    try {
      await invoke('unload_icc_profile', { display });
      void fetchStatus();
    } catch (e) {
      const msg =
        typeof e === 'object' && e !== null && 'message' in e
          ? String((e as { message: unknown }).message)
          : String(e);
      setErrorMsg(`${t('color.unloadProfile')}: ${msg}`);
    }
  };

  const handleLaunchWizard = async () => {
    try {
      await invoke('launch_color_calibration_wizard');
      addToast({ message: t('color.launchWizard'), type: 'info' });
    } catch (e) {
      const msg =
        typeof e === 'object' && e !== null && 'message' in e
          ? String((e as { message: unknown }).message)
          : String(e);
      setErrorMsg(`${t('color.launchWizard')}: ${msg}`);
      addToast({ message: msg, type: 'error' });
    }
  };

  const handleOpenSettings = async () => {
    try {
      await invoke('open_color_management_settings');
    } catch (e) {
      const msg =
        typeof e === 'object' && e !== null && 'message' in e
          ? String((e as { message: unknown }).message)
          : String(e);
      setErrorMsg(msg);
    }
  };

  const handleToggleEyeProtection = async () => {
    if (!eyeProtection) return;
    setEyeToggling(true);
    setErrorMsg(null);
    try {
      await invoke('set_eye_protection', {
        enabled: !eyeProtection.enabled,
        intensity: null,
      });
      const ep = await invoke<EyeProtectionStatus>('get_eye_protection');
      setEyeProtection(ep);
      addToast({
        message: ep.enabled ? t('color.eyeProtectionActive') : t('color.eyeProtectionInactive'),
        type: 'success',
      });
    } catch (e) {
      const msg =
        typeof e === 'object' && e !== null && 'message' in e
          ? String((e as { message: unknown }).message)
          : String(e);
      setErrorMsg(`${t('color.eyeProtection')}: ${msg}`);
      addToast({ message: `${t('color.eyeProtection')}: ${msg}`, type: 'error' });
    }
    setEyeToggling(false);
  };

  const handleIntensityChange = async (intensity: number) => {
    if (!eyeProtection) return;
    try {
      await invoke('set_eye_protection', {
        enabled: eyeProtection.enabled,
        intensity,
      });
      setEyeProtection({ ...eyeProtection, intensity });
    } catch (e) {
      const msg =
        typeof e === 'object' && e !== null && 'message' in e
          ? String((e as { message: unknown }).message)
          : String(e);
      setErrorMsg(`${t('color.eyeProtection')}: ${msg}`);
    }
  };

  if (loading) {
    return (
      <>
        <PageHeader title={t('color.title')} subtitle={t('color.subtitle')} />
        <div className="loading-spinner">{t('common.loading')}</div>
      </>
    );
  }

  return (
    <>
      <PageHeader title={t('color.title')} subtitle={t('color.subtitle')} />

      {/* Error message */}
      {errorMsg && (
        <div className="alert alert-error" style={{ marginBottom: 16 }}>
          ⚠ {errorMsg}
        </div>
      )}

      {/* Calibration Actions Card */}
      <div className="card" style={{ marginBottom: 16 }}>
        <h3>{t('color.actions')}</h3>
        <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap' }}>
          <button className="btn btn-primary" onClick={handleLaunchWizard}>
            🎨 {t('color.launchWizard')}
          </button>
          <button className="btn btn-secondary" onClick={handleOpenSettings}>
            ⚙️ {t('color.openSettings')}
          </button>
        </div>
        <p className="text-muted" style={{ marginTop: 8, fontSize: 12 }}>
          {t('color.launchWizardDesc')}
        </p>
      </div>

      {/* Eye Protection Card */}
      <div className="card" style={{ marginBottom: 16 }}>
        <h3>👁️ {t('color.eyeProtection')}</h3>
        <p className="text-muted" style={{ fontSize: 12, marginBottom: 12 }}>
          {t('color.eyeProtectionDesc')}
        </p>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 12 }}>
          <label className="toggle-row" style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <ToggleSwitch
              checked={eyeProtection?.enabled ?? false}
              onChange={handleToggleEyeProtection}
              disabled={eyeToggling || !eyeProtection}
              ariaLabel={t('color.eyeProtection')}
            />
            <span>
              {eyeProtection?.enabled
                ? t('color.eyeProtectionActive')
                : t('color.eyeProtectionInactive')}
            </span>
          </label>
        </div>
        {eyeProtection?.enabled && (
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <span className="text-muted" style={{ fontSize: 12, minWidth: 80 }}>
              {t('color.gammaIntensity')}
            </span>
            <input
              type="range"
              min={0}
              max={100}
              value={eyeProtection.intensity}
              onChange={(e) => handleIntensityChange(Number(e.target.value))}
              style={{ flex: 1 }}
            />
            <span style={{ minWidth: 40, textAlign: 'right' }}>{eyeProtection.intensity}%</span>
          </div>
        )}
      </div>

      {/* Display Profiles */}
      {status?.displays.map((display, idx) => (
        <div className="card" key={display.device_name} style={{ marginBottom: 16 }}>
          <h3>
            {t('color.displays')} {idx + 1}
            <span className="text-muted" style={{ fontSize: 12, marginLeft: 8 }}>
              {display.device_name}
            </span>
          </h3>

          <div className="info-grid" style={{ marginBottom: 12 }}>
            <div className="info-row">
              <span className="info-label">{t('color.currentProfile')}</span>
              <span className="info-value">
                {display.current_profile ? (
                  <span className="status-ok">{display.current_profile}</span>
                ) : (
                  <span className="text-muted">{t('color.noProfile')}</span>
                )}
              </span>
            </div>
            <div className="info-row">
              <span className="info-label">{t('color.hardwareCalibration')}</span>
              <span
                className={`info-value ${display.hardware_calibration ? 'status-ok' : 'text-muted'}`}
              >
                {display.hardware_calibration ? t('color.supported') : t('color.notSupported')}
              </span>
            </div>
          </div>

          {/* Profile selector */}
          {display.installed_profiles.length > 0 && (
            <div style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
              <select
                className="select-input"
                value={selectedProfiles[display.device_name] || ''}
                onChange={(e) =>
                  setSelectedProfiles((prev) => ({
                    ...prev,
                    [display.device_name]: e.target.value,
                  }))
                }
                style={{ flex: 1, minWidth: 200 }}
              >
                <option value="">{t('color.selectProfile')}</option>
                {display.installed_profiles.map((p) => {
                  const name = p.split('\\').pop() || p;
                  return (
                    <option key={p} value={p}>
                      {name}
                    </option>
                  );
                })}
              </select>
              <button
                className="btn btn-primary"
                onClick={() => handleLoadProfile(display.device_name)}
                disabled={!selectedProfiles[display.device_name]}
              >
                {t('color.loadProfile')}
              </button>
              <button
                className="btn btn-secondary"
                onClick={() => handleUnloadProfile(display.device_name)}
                disabled={!display.current_profile}
              >
                {t('color.unloadProfile')}
              </button>
            </div>
          )}

          {display.installed_profiles.length === 0 && (
            <p className="text-muted" style={{ fontSize: 12 }}>
              {t('color.noProfiles')}
            </p>
          )}
        </div>
      ))}
    </>
  );
}
