import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import { useToast } from '../../contexts/ToastContext';
import { Badge } from '../../components/ui/badge';
import { Button } from '../../components/ui/button';
import { Card } from '../../components/ui/card';
import { Hairline } from '../../components/ui/hairline';
import { Icon } from '../../components/ui/icons';
import { InfoHint } from '../../components/ui/info-hint';
import { Slider } from '../../components/ui/slider';
import { StatRow } from '../../components/ui/stat-row';
import { Switch } from '../../components/ui/switch';
import './color.css';

// ── Types matching Rust structs ──────────────────────────────────────────────

/** EDID identity of the panel behind a display. Absent when Windows could not
 *  join the CCD monitor path to a `WmiMonitorID` row — never guessed. */
interface PanelInfo {
  panel_id: string;
  manufacturer: string;
  product_code: string;
  name: string;
  model_token: string;
  year: number | null;
}

type CatalogKind = 'local' | 'download';
type MatchQuality = 'exact' | 'generic';

interface CatalogEntry {
  id: string;
  display_name: string;
  description: string;
  source: string;
  source_url: string | null;
  license_note: string;
  kind: CatalogKind;
  color_space: string;
  sha256: string | null;
  sha256_reference: string | null;
  bytes: number | null;
  file_name: string;
  available_locally: boolean;
  match_quality: MatchQuality;
  compatible: boolean;
}

interface ColorProfileInfo {
  device_name: string;
  label?: string;
  current_profile: string | null;
  installed_profiles: string[];
  /** The display exposes Windows' advanced-color (HDR / WCG) pipeline. */
  advanced_color_supported?: boolean;
  /** HDR is on for this display — calibration gammas are ignored meanwhile. */
  hdr_enabled?: boolean;
  /** EDID panel, when the display could be joined to one monitor. */
  panel?: PanelInfo | null;
  /** Curated profiles applicable to this panel. */
  catalog?: CatalogEntry[];
}

/** Stable reason codes returned by the backend for a disabled wizard. */
type WizardBlock = 'no-display' | 'hdr-enabled' | 'already-open';

interface ColorCalibrationStatus {
  displays: ColorProfileInfo[];
  eye_protection_active: boolean;
  gamma_intensity: number;
  /** Display dccw.exe calibrates (the GDI primary). */
  wizard_target?: string | null;
  /** Set when the wizard must stay disabled. */
  wizard_block?: WizardBlock | null;
  /** Date the bundled catalogue was last verified against its sources. */
  catalog_updated?: string;
}

interface EyeProtectionStatus {
  enabled: boolean;
  intensity: number;
}

interface DownloadResult {
  entry_id: string;
  file_name: string;
  staging_path: string;
  sha256: string;
  bytes: number;
  profile_class: string;
  color_space: string;
}

// ── Helpers ──────────────────────────────────────────────────────────────────

function fileNameOf(path: string | null | undefined): string {
  if (!path) return '';
  return path.split('\\').pop() || path;
}

function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null) return '—';
  if (bytes < 1024) return `${bytes} B`;
  return `${(bytes / 1024).toFixed(1)} kB`;
}

function shortHash(hash: string | null | undefined): string {
  if (!hash) return '—';
  return `${hash.slice(0, 16)}…`;
}

function errorMessage(e: unknown): string {
  if (typeof e === 'object' && e !== null && 'message' in e) {
    return String((e as { message: unknown }).message);
  }
  return String(e);
}

// ── Component ────────────────────────────────────────────────────────────────

export default function ColorTab() {
  const [status, setStatus] = useState<ColorCalibrationStatus | null>(null);
  const [eyeProtection, setEyeProtection] = useState<EyeProtectionStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [selectedProfiles, setSelectedProfiles] = useState<Record<string, string>>({});
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [eyeToggling, setEyeToggling] = useState(false);
  /** entry id → validated staging result, per session. */
  const [downloads, setDownloads] = useState<Record<string, DownloadResult>>({});
  /** entry id currently downloading/applying. */
  const [busyEntry, setBusyEntry] = useState<string | null>(null);
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
      // The command only succeeds when the WCS readback confirms the new
      // default profile, so re-fetching afterwards really shows it.
      await invoke('load_icc_profile', { display, profilePath: profile });
      setErrorMsg(null);
      await fetchStatus();
      addToast({ message: t('color.applied'), type: 'success' });
    } catch (e) {
      const msg = errorMessage(e);
      setErrorMsg(`${t('color.loadProfile')}: ${msg}`);
      addToast({ message: `${t('color.loadProfile')}: ${msg}`, type: 'error' });
    }
  };

  const handleUnloadProfile = async (display: string) => {
    try {
      // Reset to sRGB: same readback contract, the sRGB profile must be
      // reported as current before success is toasted.
      await invoke('unload_icc_profile', { display });
      setErrorMsg(null);
      await fetchStatus();
      addToast({ message: t('color.resetDone'), type: 'success' });
    } catch (e) {
      const msg = errorMessage(e);
      setErrorMsg(`${t('color.unloadProfile')}: ${msg}`);
      addToast({ message: `${t('color.unloadProfile')}: ${msg}`, type: 'error' });
    }
  };

  const handleDownload = async (entry: CatalogEntry) => {
    setBusyEntry(entry.id);
    setErrorMsg(null);
    try {
      const result = await invoke<DownloadResult>('download_color_profile', {
        entryId: entry.id,
      });
      setDownloads((prev) => ({ ...prev, [entry.id]: result }));
      addToast({
        message: t('color.downloaded', { bytes: String(result.bytes) }),
        type: 'success',
      });
    } catch (e) {
      const msg = errorMessage(e);
      setErrorMsg(`${t('color.downloadAndValidate')}: ${msg}`);
      addToast({ message: msg, type: 'error' });
    }
    setBusyEntry(null);
  };

  const handleApply = async (display: string, entry: CatalogEntry) => {
    setBusyEntry(entry.id);
    setErrorMsg(null);
    try {
      // apply_color_profile only returns success when the backend read the
      // default profile back (WCS); the banner the user sees therefore
      // reflects the system state, not an unchecked API return value.
      await invoke('apply_color_profile', { display, entryId: entry.id });
      addToast({ message: t('color.applied'), type: 'success' });
      await fetchStatus();
    } catch (e) {
      const msg = errorMessage(e);
      setErrorMsg(`${t('color.applyProfile')}: ${msg}`);
      addToast({ message: msg, type: 'error' });
    }
    setBusyEntry(null);
  };

  const handleSearch = async (kind: 'notebookcheck' | 'panelook', query: string) => {
    if (!query) return;
    try {
      await invoke('open_color_profile_search', { kind, query });
    } catch (e) {
      setErrorMsg(`${t('color.searchOnline')}: ${errorMessage(e)}`);
    }
  };

  const handleLaunchWizard = async () => {
    try {
      await invoke('launch_color_calibration_wizard');
      addToast({ message: t('color.launchWizard'), type: 'info' });
    } catch (e) {
      const msg = errorMessage(e);
      setErrorMsg(`${t('color.launchWizard')}: ${msg}`);
      addToast({ message: msg, type: 'error' });
    }
  };

  const handleOpenSettings = async () => {
    try {
      await invoke('open_color_management_settings');
    } catch (e) {
      setErrorMsg(errorMessage(e));
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
      const msg = errorMessage(e);
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
      setErrorMsg(`${t('color.eyeProtection')}: ${errorMessage(e)}`);
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

  // The wizard is only offered when it can actually do something: dccw.exe
  // calibrates the primary display, and Windows discards the calibration gamma
  // while HDR is on. The backend returns a stable code so the reason is
  // translated here instead of echoing an English error.
  const wizardBlock = status?.wizard_block ?? null;
  const wizardBlockReason = (block: WizardBlock) =>
    block === 'no-display'
      ? t('color.wizardBlockedNoDisplay')
      : block === 'hdr-enabled'
        ? t('color.wizardBlockedHdr')
        : t('color.wizardBlockedOpen');

  return (
    <div className="mc-stack">
      <PageHeader title={t('color.title')} subtitle={t('color.subtitle')} />

      {errorMsg && (
        <div className="mc-note mc-note--danger" role="alert">
          <Icon name="alert-triangle" size={16} /> {errorMsg}
        </div>
      )}

      <Card title={t('color.actions')} icon={<Icon name="palette" size={16} />}>
        <div className="color-actions">
          <Button
            variant="primary"
            icon="palette"
            onClick={handleLaunchWizard}
            disabled={wizardBlock != null}
            title={wizardBlock ? wizardBlockReason(wizardBlock) : undefined}
          >
            {t('color.launchWizard')}
          </Button>
          <Button variant="secondary" icon="gear" onClick={handleOpenSettings}>
            {t('color.openSettings')}
          </Button>
        </div>
        <p className="color-note">
          {wizardBlock
            ? wizardBlockReason(wizardBlock)
            : status?.wizard_target
              ? `${t('color.launchWizardDesc')} — ${t('color.wizardTargets', {
                  device: status.wizard_target,
                })}`
              : t('color.launchWizardDesc')}
        </p>
      </Card>

      <Card
        title={t('color.eyeProtection')}
        icon={<Icon name="eye" size={16} />}
        meta={
          eyeProtection?.enabled ? (
            <Badge tone="success" dot>
              {t('color.eyeProtectionActive')}
            </Badge>
          ) : (
            <Badge tone="neutral">{t('color.eyeProtectionInactive')}</Badge>
          )
        }
      >
        <Switch
          checked={eyeProtection?.enabled ?? false}
          onChange={handleToggleEyeProtection}
          pending={eyeToggling}
          disabled={!eyeProtection}
          label={t('color.eyeProtection')}
          desc={t('color.eyeProtectionDesc')}
        />
        {eyeProtection?.enabled && (
          <div className="color-slider">
            <Slider
              label={t('color.gammaIntensity')}
              min={0}
              max={100}
              value={eyeProtection.intensity}
              onChange={handleIntensityChange}
              unit="%"
              ariaLabel={t('color.gammaIntensity')}
            />
          </div>
        )}
      </Card>

      {status?.displays.map((display, idx) => {
        const panel = display.panel ?? null;
        const catalog = display.catalog ?? [];
        const searchQuery = panel?.model_token || display.label || '';
        return (
          <Card
            key={display.device_name}
            title={display.label || panel?.name || `${t('color.displays')} ${idx + 1}`}
            icon={<Icon name="monitor" size={16} />}
            meta={<span className="color-device">{display.device_name}</span>}
          >
            <div className="color-stats">
              <StatRow
                label={t('color.detectedPanel')}
                value={panel ? panel.panel_id : '—'}
                mono
                dim={!panel}
                hint={panel && panel.name !== panel.panel_id ? panel.name : undefined}
              />
              <StatRow
                label={t('color.currentProfile')}
                value={
                  display.current_profile
                    ? fileNameOf(display.current_profile)
                    : t('color.noProfile')
                }
                dim={!display.current_profile}
              />
              <StatRow
                label={t('color.advancedColor')}
                value={
                  display.advanced_color_supported
                    ? display.hdr_enabled
                      ? t('color.hdrOn')
                      : t('color.hdrAvailable')
                    : t('color.notSupported')
                }
                dim={!display.advanced_color_supported}
              />
            </div>

            {!panel && (
              <div className="mc-note">
                <Icon name="info" size={14} />
                {t('color.panelUnknownHint')}
              </div>
            )}

            {display.hdr_enabled && (
              <div className="mc-note mc-note--warning">
                <Icon name="alert-triangle" size={14} />
                {t('color.hdrIgnoresProfile')}
              </div>
            )}

            <Hairline />

            <div className="color-section-head">
              <h3 className="color-section-title">{t('color.catalogTitle')}</h3>
              {status?.catalog_updated && (
                <span className="color-meta">
                  {t('color.catalogVerified', { date: status.catalog_updated })}
                </span>
              )}
            </div>

            {catalog.length === 0 && <p className="color-note">{t('color.noCatalog')}</p>}

            <div className="color-catalog">
              {catalog.map((entry) => {
                const staged = downloads[entry.id];
                const busy = busyEntry === entry.id;
                const applyDisabled = Boolean(display.hdr_enabled) || busy;
                return (
                  <div className="color-entry" key={entry.id}>
                    <div className="color-entry__head">
                      <span className="color-entry__name">{entry.display_name}</span>
                      {entry.compatible ? (
                        <Badge tone="success" dot>
                          {t('color.compatible')}
                        </Badge>
                      ) : (
                        <Badge tone="neutral">{t('color.genericProfile')}</Badge>
                      )}
                      <Badge tone={entry.kind === 'download' ? 'info' : 'neutral'}>
                        {entry.kind === 'download' ? t('color.downloadable') : t('color.builtIn')}
                      </Badge>
                      <InfoHint
                        label={`${entry.description} — ${t('color.licenseNote')}: ${entry.license_note}`}
                      />
                    </div>

                    <div className="color-entry__meta">
                      <span>{entry.source}</span>
                      <span>{entry.color_space}</span>
                      <span
                        className="color-mono"
                        title={entry.sha256 || entry.sha256_reference || ''}
                      >
                        {entry.sha256 ? t('color.sha256') : t('color.sha256Reference')}{' '}
                        {shortHash(entry.sha256 || entry.sha256_reference)}
                      </span>
                      <span>{formatBytes(entry.bytes)}</span>
                      <span className={entry.available_locally ? '' : 'color-warn'}>
                        {entry.available_locally
                          ? t('color.presentLocally')
                          : t('color.notPresent')}
                      </span>
                    </div>

                    <div className="color-entry__actions">
                      {entry.kind === 'local' || entry.available_locally ? (
                        <Button
                          variant="secondary"
                          onClick={() => handleApply(display.device_name, entry)}
                          disabled={!entry.available_locally || applyDisabled}
                          pending={busy}
                          title={display.hdr_enabled ? t('color.hdrIgnoresProfile') : undefined}
                        >
                          {t('color.applyProfile')}
                        </Button>
                      ) : staged ? (
                        <Button
                          variant="primary"
                          onClick={() => handleApply(display.device_name, entry)}
                          disabled={applyDisabled}
                          pending={busy}
                          title={display.hdr_enabled ? t('color.hdrIgnoresProfile') : undefined}
                        >
                          {t('color.installAndApply')}
                        </Button>
                      ) : (
                        <Button
                          variant="secondary"
                          onClick={() => handleDownload(entry)}
                          pending={busy}
                        >
                          {t('color.downloadAndValidate')}
                        </Button>
                      )}
                      {staged && (
                        <span className="color-meta color-mono" title={staged.sha256}>
                          {t('color.sha256')} {shortHash(staged.sha256)}
                        </span>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>

            <Hairline />

            <div className="color-actions">
              <Button
                variant="ghost"
                icon="refresh"
                onClick={() => handleUnloadProfile(display.device_name)}
                disabled={!display.current_profile}
              >
                {t('color.unloadProfile')}
              </Button>
              {searchQuery && (
                <>
                  <Button
                    variant="ghost"
                    icon="search"
                    onClick={() => handleSearch('notebookcheck', searchQuery)}
                  >
                    {t('color.searchNotebookcheck')}
                  </Button>
                  <Button
                    variant="ghost"
                    icon="search"
                    onClick={() => handleSearch('panelook', panel?.model_token || searchQuery)}
                  >
                    {t('color.searchPanelook')}
                  </Button>
                </>
              )}
            </div>

            <p className="color-note color-note--small">{t('color.searchHint')}</p>

            {display.installed_profiles.length > 0 && (
              <div className="color-installed">
                <div className="color-section-head">
                  <h3 className="color-section-title">{t('color.installedProfiles')}</h3>
                </div>
                <div className="color-installed__row">
                  <select
                    className="select-input"
                    aria-label={t('color.selectProfile')}
                    value={selectedProfiles[display.device_name] || ''}
                    onChange={(e) =>
                      setSelectedProfiles((prev) => ({
                        ...prev,
                        [display.device_name]: e.target.value,
                      }))
                    }
                  >
                    <option value="">{t('color.selectProfile')}</option>
                    {display.installed_profiles.map((p) => (
                      <option key={p} value={p}>
                        {fileNameOf(p)}
                      </option>
                    ))}
                  </select>
                  <Button
                    variant="secondary"
                    onClick={() => handleLoadProfile(display.device_name)}
                    disabled={!selectedProfiles[display.device_name]}
                  >
                    {t('color.loadProfile')}
                  </Button>
                </div>
              </div>
            )}

            {display.installed_profiles.length === 0 && (
              <p className="color-note">{t('color.noProfiles')}</p>
            )}
          </Card>
        );
      })}

      {status != null && status.displays.length === 0 && (
        <Card>
          <p className="color-note">{t('color.noDisplays')}</p>
        </Card>
      )}

      <p className="color-disclaimer">
        <Icon name="info" size={14} /> {t('color.disclaimer')}
      </p>
    </div>
  );
}
