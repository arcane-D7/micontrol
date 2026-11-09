import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import InfoModal from '../../components/InfoModal';
import ToggleSwitch from '../../components/ToggleSwitch';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import { Icon } from '../../components/ui/icons';

interface FaceStatus {
  service_installed: boolean;
  service_running: boolean;
  pipe_available: boolean;
  enrolled_profiles: number;
  camera_available: boolean;
}

interface FaceSettings {
  match_threshold: number;
  match_margin: number;
  liveness_enabled: boolean;
  antispoof_enabled: boolean;
  antispoof_threshold: number;
  antispoof_max_frames: number;
  lockout_max_fails: number;
  lockout_seconds: number;
  multi_face_protection_enabled: boolean;
  face_unlock_enabled: boolean;
  face_unlock_logon_enabled: boolean;
  face_unlock_workstation_enabled: boolean;
  renew_days: number;
  language: string;
}

interface FaceModelsStatus {
  installed: boolean;
  staged: boolean;
  installed_dir: string;
  staging_dir: string;
  url: string;
}

const DEFAULT_SETTINGS: FaceSettings = {
  match_threshold: 0.4,
  match_margin: 0.05,
  liveness_enabled: true,
  antispoof_enabled: true,
  antispoof_threshold: 0.55,
  antispoof_max_frames: 10,
  lockout_max_fails: 5,
  lockout_seconds: 30,
  multi_face_protection_enabled: false,
  face_unlock_enabled: true,
  face_unlock_logon_enabled: true,
  face_unlock_workstation_enabled: true,
  renew_days: 60,
  language: 'en',
};

/** Tauri invoke errors arrive as `{code, message}` — render the message,
 *  not "[object Object]". */
function getFriendlyErr(e: unknown): string {
  if (e && typeof e === 'object') {
    const m = (e as { message?: unknown }).message;
    if (typeof m === 'string' && m.length > 0) return m;
  }
  return String(e);
}

type EnrollStep = 'user' | 'hello' | 'password' | 'camera';

/**
 * Single-flow enrollment wizard:
 *  1. Pick the Windows account (dropdown of local users).
 *  2. Confirm identity with Windows Hello (PIN / fingerprint / face).
 *  3. Provide the Windows sign-in password ONCE (stored in LSA, elevated) —
 *     only shown after Hello passes.
 *  4. Camera capture.
 */
// @TODO: Reassess login process later — Gap between Windows Hello (PIN/fingerprint)
// auth and camera unlock: Windows does not expose Hello/NGC credentials to third-party
// lock-screen components, so camera unlock requires the account password stored once
// (sealed in LSA). Evaluate whether a PIN-based unlock (e.g. via a dedicated provider
// API) becomes feasible, or if the password-credential step can be dropped/refined.
function EnrollWizard({ onClose, onEnrolled }: { onClose: () => void; onEnrolled: () => void }) {
  const [step, setStep] = useState<EnrollStep>('user');
  const [name, setName] = useState('');
  const [label, setLabel] = useState('front');
  const [frames, setFrames] = useState(4);
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ type: 'ok' | 'err'; text: string } | null>(null);
  const [userList, setUserList] = useState<{ name: string; sid?: string | null }[]>([]);
  const [userListErr, setUserListErr] = useState<string | null>(null);
  const [helloResult, setHelloResult] = useState<{
    status: string;
    verified: boolean;
    message: string;
  } | null>(null);
  const [pwConfigured, setPwConfigured] = useState<boolean | null>(null);
  const [jpeg, setJpeg] = useState<string | null>(null);
  const [aspect, setAspect] = useState(16 / 9);
  const [camErr, setCamErr] = useState<string | null>(null);
  const [started, setStarted] = useState(false);
  const timerRef = useRef<number | null>(null);

  // Load local Windows users for the dropdown (default: current user).
  useEffect(() => {
    void invoke<{
      users: { name: string; sid?: string | null; enabled: boolean }[];
      current_user: string;
    }>('face_list_users')
      .then((r) => {
        setUserList(r.users);
        setUserListErr(null);
        const cur = r.users.find(
          (u) => u.name.toLowerCase() === (r.current_user || '').toLowerCase(),
        );
        if (cur) setName(cur.name);
        else if (r.users.length > 0) setName(r.users[0].name);
      })
      .catch((e) => setUserListErr(getFriendlyErr(e)));
  }, []);

  // ── Step 2: Windows Hello consent ───────────────────────────────────────
  const doHello = async () => {
    setBusy(true);
    setMsg(null);
    try {
      const r = await invoke<{ status: string; verified: boolean; message: string }>(
        'face_hello_verify',
      );
      setHelloResult(r);
      if (r.verified) {
        // Hello passed → offer the (optional) password step.
        try {
          const c = await invoke<{ configured: boolean; unknown?: boolean }>(
            'face_password_configured',
            { user: name },
          );
          setPwConfigured(!!c.configured && !c.unknown);
        } catch {
          setPwConfigured(null);
        }
        setStep('password');
      }
    } catch (e) {
      setMsg({ type: 'err', text: getFriendlyErr(e) });
    } finally {
      setBusy(false);
    }
  };

  // ── Step 3: store password once (elevated) ─────────────────────────────
  const savePassword = async (skip: boolean) => {
    if (skip) {
      setStep('camera');
      return;
    }
    if (!password) {
      setMsg({ type: 'err', text: t('face.enroll.enterPassword') });
      return;
    }
    setBusy(true);
    setMsg(null);
    try {
      await invoke('face_set_password', { user: name, password });
      setPwConfigured(true);
      setPassword('');
      setMsg({ type: 'ok', text: t('face.enroll.passwordStored', { name }) });
      setStep('camera');
    } catch (e) {
      setMsg({ type: 'err', text: getFriendlyErr(e) });
    } finally {
      setBusy(false);
    }
  };

  // ── Step 4: camera capture ──────────────────────────────────────────────
  const startPreview = useCallback(async () => {
    try {
      await invoke('face_camera_preview_start');
      setStarted(true);
      setCamErr(null);
    } catch (e) {
      setCamErr(getFriendlyErr(e));
    }
  }, []);

  const stopPreview = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearInterval(timerRef.current);
      timerRef.current = null;
    }
    void invoke('face_camera_preview_stop').catch(() => {});
    setStarted(false);
    setJpeg(null);
  }, []);

  useEffect(() => {
    if (step !== 'camera') return;
    void startPreview();
    return () => stopPreview();
  }, [step, startPreview, stopPreview]);

  // Poll the preview frame @ ~8 Hz.
  useEffect(() => {
    if (!started) return;
    timerRef.current = window.setInterval(async () => {
      try {
        const f = await invoke<{
          error: string | null;
          jpeg: string | null;
          width: number | null;
          height: number | null;
        }>('face_camera_preview_frame');
        if (f.error) setCamErr(f.error);
        else if (f.jpeg) {
          setCamErr(null);
          setJpeg(f.jpeg);
          if (f.width && f.height) setAspect(f.width / f.height);
        }
      } catch {
        /* keep last frame */
      }
    }, 120);
    return () => {
      if (timerRef.current !== null) window.clearInterval(timerRef.current);
    };
  }, [started]);

  const doCapture = async () => {
    if (!name.trim()) {
      setMsg({ type: 'err', text: t('face.enroll.pickAccountFirst') });
      return;
    }
    setBusy(true);
    setMsg(null);
    try {
      // Free the camera first — the preview thread holds the capture lock.
      if (timerRef.current !== null) {
        window.clearInterval(timerRef.current);
        timerRef.current = null;
      }
      await invoke('face_camera_preview_stop').catch(() => {});
      const r = await invoke<{ ok: boolean; name: string; label: string; frames: number }>(
        'face_enroll',
        { name: name.trim(), frames, label },
      );
      setMsg({
        type: 'ok',
        text: t('face.enroll.enrolled', { name: r.name, frames: r.frames, label: r.label }),
      });
      onEnrolled();
    } catch (e) {
      setMsg({ type: 'err', text: getFriendlyErr(e) });
    } finally {
      setBusy(false);
    }
  };

  const stepTitle: Record<EnrollStep, string> = {
    user: t('face.enroll.step.user'),
    hello: t('face.enroll.step.hello'),
    password: pwConfigured
      ? t('face.enroll.step.passwordSaved')
      : t('face.enroll.step.passwordOptional'),
    camera: t('face.enroll.step.camera'),
  };

  return (
    <InfoModal
      open
      onClose={() => {
        stopPreview();
        onClose();
      }}
      title={t('face.enroll.title', { step: stepTitle[step] })}
    >
      <div style={{ display: 'grid', gap: 16 }}>
        {/* ── Step indicator (clean pills, states) ─────────────────────────── */}
        <div
          style={{
            display: 'flex',
            gap: 6,
            fontSize: 11.5,
            alignItems: 'center',
            color: 'var(--text-muted)',
            flexWrap: 'wrap',
          }}
        >
          {(['user', 'hello', 'password', 'camera'] as EnrollStep[]).map((s, i) => {
            const done =
              (s === 'user' && step !== 'user') ||
              (s === 'hello' && (step === 'password' || step === 'camera')) ||
              (s === 'password' && step === 'camera' && pwConfigured === true);
            const active = step === s;
            return (
              <span
                key={s}
                style={{
                  padding: '4px 10px',
                  borderRadius: 99,
                  background: active
                    ? 'var(--accent-soft)'
                    : done
                      ? 'var(--surface-2)'
                      : 'var(--surface-2)',
                  color: active ? 'var(--accent)' : done ? 'var(--text-muted)' : 'var(--text-dim)',
                  fontWeight: active ? 600 : 500,
                  border: active ? '1px solid var(--accent-soft)' : '1px solid transparent',
                }}
              >
                {done ? <Icon name="check" size={14} /> : `${i + 1}. `}{' '}
                {s === 'password'
                  ? t('face.enroll.pill.credential')
                  : s === 'camera'
                    ? t('face.enroll.pill.capture')
                    : s === 'user'
                      ? t('face.enroll.pill.user')
                      : t('face.enroll.pill.hello')}
              </span>
            );
          })}
        </div>

        {/* ── 1 · Windows account ─────────────────────────────────────────── */}
        {step === 'user' && (
          <>
            <div
              style={{
                background: 'var(--surface-2)',
                border: '1px solid var(--border)',
                borderRadius: 'var(--r-md)',
                padding: 16,
                display: 'grid',
                gap: 12,
              }}
            >
              <p style={{ margin: 0, fontSize: 13.5, lineHeight: 1.6 }}>
                {t('face.enroll.accountQuestion')}
              </p>
              <label style={{ display: 'grid', gap: 5 }}>
                <span style={{ fontSize: 12, color: 'var(--text-muted)', fontWeight: 600 }}>
                  {t('face.enroll.account')}
                </span>
                <select
                  className="text-input"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  disabled={userList.length === 0 || busy}
                >
                  {userList.length === 0 && (
                    <option value="">
                      {userListErr
                        ? t('face.enroll.usersLoadFailed')
                        : t('face.enroll.loadingUsers')}
                    </option>
                  )}
                  {userList.map((u) => (
                    <option key={u.name} value={u.name}>
                      {u.name}
                    </option>
                  ))}
                </select>
                {userListErr && (
                  <span style={{ fontSize: 11.5, color: 'var(--warning)' }}>
                    <Icon name="alert-triangle" size={16} /> {userListErr}
                  </span>
                )}
              </label>
              <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap' }}>
                <label style={{ display: 'grid', gap: 5, flex: 1, minWidth: 140 }}>
                  <span style={{ fontSize: 12, color: 'var(--text-muted)', fontWeight: 600 }}>
                    {t('face.enroll.label')}
                  </span>
                  <input
                    className="text-input"
                    placeholder={t('face.enroll.labelPlaceholder')}
                    value={label}
                    onChange={(e) => setLabel(e.target.value)}
                  />
                </label>
                <label style={{ display: 'grid', gap: 5, width: 110 }}>
                  <span style={{ fontSize: 12, color: 'var(--text-muted)', fontWeight: 600 }}>
                    {t('face.enroll.photos')}
                  </span>
                  <input
                    className="text-input"
                    type="number"
                    min={1}
                    max={16}
                    value={frames}
                    onChange={(e) => setFrames(Number(e.target.value))}
                  />
                </label>
              </div>
              <p
                style={{
                  margin: 0,
                  fontSize: 12,
                  color: 'var(--text-muted)',
                  lineHeight: 1.5,
                  borderTop: '1px solid var(--border)',
                  paddingTop: 10,
                }}
              >
                {t('face.enroll.helloNote')}
              </p>
            </div>
            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
              <button className="btn btn-secondary" disabled={busy} onClick={onClose}>
                {t('common.cancel')}
              </button>
              <button
                className="btn btn-primary"
                disabled={busy || !name.trim()}
                onClick={() => setStep('hello')}
              >
                {t('face.enroll.continue')}
              </button>
            </div>
          </>
        )}

        {/* ── 2 · Verify with PIN / fingerprint ───────────────────────────── */}
        {step === 'hello' && (
          <>
            <div
              style={{
                background: 'var(--surface-2)',
                border: '1px solid var(--border)',
                borderRadius: 'var(--r-md)',
                padding: 16,
                display: 'grid',
                gap: 10,
              }}
            >
              <p style={{ margin: 0, fontSize: 13.5, lineHeight: 1.6 }}>
                {t('face.enroll.helloIntro')}
              </p>
              {helloResult && !helloResult.verified && (
                <p
                  style={{
                    margin: 0,
                    fontSize: 13,
                    color:
                      helloResult.status === 'canceled' ? 'var(--text-muted)' : 'var(--warning)',
                    lineHeight: 1.5,
                    background: 'var(--accent-soft)',
                    borderRadius: 'var(--r-sm)',
                    padding: '8px 10px',
                  }}
                >
                  {helloResult.message}
                </p>
              )}
              {msg && (
                <p
                  style={{
                    margin: 0,
                    fontSize: 13,
                    color: msg.type === 'ok' ? 'var(--success)' : 'var(--error)',
                    lineHeight: 1.5,
                    background: 'var(--surface-2)',
                    borderRadius: 'var(--r-sm)',
                    padding: '8px 10px',
                  }}
                >
                  {msg.text}
                </p>
              )}
            </div>
            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
              <button className="btn btn-secondary" disabled={busy} onClick={() => setStep('user')}>
                {t('face.enroll.back')}
              </button>
              <button className="btn btn-primary" disabled={busy} onClick={() => void doHello()}>
                {busy ? (
                  <>
                    <Icon name="hourglass" size={16} /> {t('face.enroll.waiting')}
                  </>
                ) : (
                  <>
                    <Icon name="key" size={16} /> {t('face.enroll.verify')}
                  </>
                )}
              </button>
            </div>
          </>
        )}

        {/* ── 3 · Unlock credential (optional) ─────────────────────────────── */}
        {step === 'password' && (
          <>
            <div
              style={{
                background: 'var(--surface-2)',
                border: '1px solid var(--border)',
                borderRadius: 'var(--r-md)',
                padding: 16,
                display: 'grid',
                gap: 12,
              }}
            >
              {pwConfigured ? (
                <>
                  <p style={{ margin: 0, fontSize: 13.5, lineHeight: 1.6 }}>
                    {t('face.enroll.storedFor', { name })}
                  </p>
                  <p
                    style={{ margin: 0, fontSize: 12, color: 'var(--text-muted)', lineHeight: 1.5 }}
                  >
                    {t('face.enroll.changeHint')}
                  </p>
                </>
              ) : (
                <>
                  <p style={{ margin: 0, fontSize: 13.5, lineHeight: 1.6 }}>
                    {t('face.enroll.optionalNote')}
                  </p>
                  <input
                    className="text-input"
                    type="password"
                    placeholder={t('face.enroll.passwordPlaceholder', { name })}
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                    autoFocus
                    style={{ width: '100%' }}
                  />
                  <p
                    style={{
                      margin: 0,
                      fontSize: 12,
                      color: 'var(--text-muted)',
                      lineHeight: 1.5,
                    }}
                  >
                    {t('face.enroll.pinReminder')}
                  </p>
                </>
              )}
              {msg && (
                <p
                  style={{
                    margin: 0,
                    fontSize: 13,
                    color: msg.type === 'ok' ? 'var(--success)' : 'var(--error)',
                    lineHeight: 1.5,
                    background: 'var(--surface-2)',
                    borderRadius: 'var(--r-sm)',
                    padding: '8px 10px',
                  }}
                >
                  {msg.text}
                </p>
              )}
            </div>
            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end', flexWrap: 'wrap' }}>
              <button
                className="btn btn-secondary"
                disabled={busy}
                onClick={() => setStep('hello')}
              >
                {t('face.enroll.back')}
              </button>
              {!pwConfigured && (
                <button
                  className="btn btn-ghost"
                  disabled={busy}
                  onClick={() => void savePassword(true)}
                >
                  {t('face.enroll.skipPinOnly')}
                </button>
              )}
              <button
                className="btn btn-primary"
                disabled={busy || (!pwConfigured && !password)}
                onClick={() => void savePassword(pwConfigured === true)}
              >
                {busy ? (
                  <>
                    <Icon name="hourglass" size={16} /> {t('face.enroll.storing')}
                  </>
                ) : pwConfigured ? (
                  t('face.enroll.continue')
                ) : (
                  <>
                    <Icon name="lock" size={16} /> {t('face.enroll.saveCredential')}
                  </>
                )}
              </button>
            </div>
          </>
        )}

        {/* 4 ── camera capture */}
        {step === 'camera' && (
          <>
            <div
              style={{
                position: 'relative',
                width: '100%',
                aspectRatio: aspect.toString(),
                background: 'var(--bg, #111)',
                borderRadius: 'var(--r-md)',
                overflow: 'hidden',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                border: '1px solid var(--border)',
              }}
            >
              {jpeg ? (
                <img
                  src={`data:image/jpeg;base64,${jpeg}`}
                  alt={t('face.enroll.cameraPreviewAlt')}
                  style={{ width: '100%', height: '100%', objectFit: 'contain' }}
                />
              ) : (
                <span style={{ color: 'var(--text-dim)', fontSize: 13 }}>
                  {camErr
                    ? t('face.enroll.cameraError', { error: camErr })
                    : t('face.enroll.startingCamera')}
                </span>
              )}
              <div
                style={{
                  position: 'absolute',
                  bottom: 8,
                  left: 0,
                  right: 0,
                  textAlign: 'center',
                  fontSize: 12,
                  color: '#fff',
                  background: 'rgba(0,0,0,.5)',
                  padding: '3px 0',
                }}
              >
                {t('face.enroll.livePreview')}
              </div>
            </div>
            {camErr && (
              <p style={{ margin: 0, color: 'var(--warning)', fontSize: 12.5 }}>
                <Icon name="alert-triangle" size={16} />{' '}
                {t('face.enroll.cameraBusy', { error: camErr })}
              </p>
            )}
            <div
              style={{
                background: 'var(--surface-2)',
                border: '1px solid var(--border)',
                borderRadius: 'var(--r-md)',
                padding: 12,
              }}
            >
              <p style={{ margin: 0, fontSize: 12.5, lineHeight: 1.6 }}>
                {t('face.enroll.captureNote', { frames, name })}
              </p>
            </div>
            {msg && (
              <p
                style={{
                  margin: 0,
                  fontSize: 13,
                  color: msg.type === 'ok' ? 'var(--success)' : 'var(--error)',
                  lineHeight: 1.5,
                  background: 'var(--surface-2)',
                  borderRadius: 'var(--r-sm)',
                  padding: '8px 10px',
                }}
              >
                {msg.text}
              </p>
            )}
            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
              <button
                className="btn btn-secondary"
                disabled={busy}
                onClick={() => {
                  stopPreview();
                  setStep('password');
                }}
              >
                {t('face.enroll.back')}
              </button>
              <button className="btn btn-primary" disabled={busy} onClick={() => void doCapture()}>
                {busy ? (
                  <>
                    <Icon name="hourglass" size={16} /> {t('face.enroll.capturing')}
                  </>
                ) : (
                  <>
                    <Icon name="camera" size={16} /> {t('face.enroll.capture', { frames })}
                  </>
                )}
              </button>
            </div>
          </>
        )}
      </div>
    </InfoModal>
  );
}

export default function FaceUnlockTab() {
  const [status, setStatus] = useState<FaceStatus | null>(null);
  const [settings, setSettings] = useState<FaceSettings>(DEFAULT_SETTINGS);
  const [modelsStatus, setModelsStatus] = useState<FaceModelsStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ type: 'ok' | 'err'; text: string } | null>(null);
  const [diagnostics, setDiagnostics] = useState<Record<string, unknown> | null>(null);
  const [diagnosticsError, setDiagnosticsError] = useState<string | null>(null);
  const [enrollOpen, setEnrollOpen] = useState(false);
  const [downloadProgress, setDownloadProgress] = useState<number | null>(null);
  const [downloading, setDownloading] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [showMaintenance, setShowMaintenance] = useState(false);
  const [dirty, setDirty] = useState(false);

  const load = useCallback(async () => {
    try {
      const st = await invoke<FaceStatus>('face_status');
      setStatus(st);
      const s = await invoke<FaceSettings>('face_get_settings');
      setSettings({ ...DEFAULT_SETTINGS, ...s });
    } catch (e) {
      setMsg({ type: 'err', text: t('face.loadError', { error: String(e) }) });
    }
  }, []);

  const loadModels = useCallback(async (): Promise<FaceModelsStatus | null> => {
    try {
      const m = await invoke<FaceModelsStatus>('face_models_status');
      setModelsStatus(m);
      return m;
    } catch {
      setModelsStatus(null);
      return null;
    }
  }, []);

  useEffect(() => {
    void load();
    void loadModels();
    // If the setup wizard (or another path) removes the models, rescan so the
    // download button re-enables immediately.
    const un = listen('face-models-removed', () => {
      void loadModels();
    });
    const unProg = listen<number>('face-model-progress', (e) => {
      setDownloadProgress(e.payload);
    });
    return () => {
      void un.then((f) => f());
      void unProg.then((f) => f());
    };
  }, [load, loadModels]);

  const show = (type: 'ok' | 'err', text: string) => {
    setMsg({ type, text });
    setTimeout(() => setMsg(null), 6000);
  };

  const saveSettings = async () => {
    setBusy(true);
    try {
      await invoke('face_set_settings', { settings });
      setDirty(false);
      show('ok', t('face.settings.saved'));
    } catch (e) {
      show('err', t('face.settings.error', { error: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  const runDiagnostics = async () => {
    setBusy(true);
    setDiagnosticsError(null);
    try {
      const d = await invoke<Record<string, unknown>>('face_diagnostics');
      setDiagnostics(d);
    } catch (e) {
      setDiagnostics(null);
      setDiagnosticsError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const removeAllModels = async () => {
    // Confirm before deleting downloaded models (~250 MB).
    if (!window.confirm(t('face.maint.confirmRemove'))) {
      return;
    }
    setBusy(true);
    try {
      const r = await invoke<{
        ok: boolean;
        removed: string[];
        warnings: string[];
        status: { installed: boolean; staged: boolean };
      }>('face_models_remove_all');
      show(
        r.warnings.length > 0 ? 'err' : 'ok',
        r.removed.length > 0
          ? t('face.maint.removed', { count: r.removed.length }) +
              (r.warnings.length
                ? ` ${t('face.maint.warnings', { warnings: r.warnings.join('; ') })}`
                : '')
          : t('face.maint.nothingRemoved'),
      );
      await loadModels();
    } catch (e) {
      show('err', t('face.maint.removeError', { error: getFriendlyErr(e) }));
    } finally {
      setBusy(false);
    }
  };

  const rescanModels = async () => {
    setBusy(true);
    try {
      const m = await loadModels();
      const present = !!m && (m.installed || m.staged);
      show('ok', present ? t('face.maint.modelsPresent') : t('face.maint.modelsMissing'));
    } catch (e) {
      show('err', t('face.maint.rescanError', { error: getFriendlyErr(e) }));
    } finally {
      setBusy(false);
    }
  };

  // True once the ONNX module files are present (either staged under
  // ProgramData or installed under Program Files). Used to disable the
  // "Download & install module" button so the 250 MB download is not
  // re-triggered after setup, and to gate enrollment on the models existing.
  const modelsInstalled = !!modelsStatus && (modelsStatus.installed || modelsStatus.staged);
  const modelsKnown = modelsStatus !== null;

  // Setup journey phase — drives the hero card.
  const enabled = settings.face_unlock_enabled;
  const svcInstalled = !!status?.service_installed;
  const svcRunning = !!status?.service_running;
  const hasFaces = (status?.enrolled_profiles ?? 0) > 0;
  const phase: 'off' | 'models' | 'service' | 'enroll' | 'ready' = !enabled
    ? 'off'
    : !modelsInstalled
      ? 'models'
      : !svcRunning
        ? 'service'
        : !hasFaces
          ? 'enroll'
          : 'ready';

  const enrolledCount = status?.enrolled_profiles ?? 0;
  const phaseMeta: Record<string, { dot: string; title: string; subtitle: string }> = {
    off: {
      dot: 'var(--text-dim)',
      title: t('face.phase.off.title'),
      subtitle: t('face.phase.off.subtitle'),
    },
    models: {
      dot: 'var(--warning)',
      title: t('face.phase.models.title'),
      subtitle: t('face.phase.models.subtitle'),
    },
    service: {
      dot: 'var(--warning)',
      title: t('face.phase.service.title'),
      subtitle: t('face.phase.service.subtitle'),
    },
    enroll: {
      dot: 'var(--accent)',
      title: t('face.phase.enroll.title'),
      subtitle: t('face.phase.enroll.subtitle'),
    },
    ready: {
      dot: 'var(--success)',
      title: t('face.phase.ready.title', { count: enrolledCount }),
      subtitle: t('face.phase.ready.subtitle'),
    },
  };
  const meta = phaseMeta[phase];

  const set = (patch: Partial<FaceSettings>) => {
    setSettings((s) => ({ ...s, ...patch }));
    setDirty(true);
  };

  // Step 1 — download the ONNX models inline (no modal), with progress.
  const downloadModels = async () => {
    setDownloading(true);
    setDownloadProgress(0);
    setMsg(null);
    try {
      await invoke('face_download_models');
      try {
        await invoke('face_install_models');
      } catch {
        // staged copy is enough; the installer copies it on reinstall
      }
      await loadModels();
      show('ok', t('face.downloadReady'));
    } catch (e) {
      show('err', t('face.downloadError', { error: getFriendlyErr(e) }));
    } finally {
      setDownloading(false);
      setDownloadProgress(null);
    }
  };

  // Step 2 — install the service if missing, otherwise self-heal (sc failure +
  // restart) after a crash. No UAC prompt through the bridge.
  const doServiceAction = async () => {
    setBusy(true);
    setMsg(null);
    try {
      if (!svcInstalled) {
        const r = await invoke<{ ok: boolean; stdout: string; stderr: string }>(
          'face_service_install',
        );
        show(
          r.ok ? 'ok' : 'err',
          r.ok
            ? t('face.maint.serviceInstalled')
            : t('face.maint.installFailed', { error: r.stderr }),
        );
      } else {
        const r = await invoke<{
          service_installed: boolean;
          service_running: boolean;
          state?: string;
          action?: string;
          failure_actions_configured?: boolean;
        }>('face_service_ensure');
        show(
          'ok',
          r.action === 'already_running'
            ? t('face.maint.serviceRunning')
            : r.action === 'started'
              ? t('face.maint.serviceStarted')
              : t('face.maint.serviceStartSent'),
        );
      }
      await load();
    } catch (e) {
      show('err', t('face.maint.serviceError', { error: getFriendlyErr(e) }));
    } finally {
      setBusy(false);
    }
  };

  const toggleMaster = async (enabled: boolean) => {
    setBusy(true);
    try {
      await invoke('face_set_settings', {
        settings: { ...settings, face_unlock_enabled: enabled },
      });
      setSettings((s) => ({ ...s, face_unlock_enabled: enabled }));
      show('ok', enabled ? t('face.toggle.enabled') : t('face.toggle.disabled'));
    } catch (e) {
      show('err', t('face.toggle.error', { error: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="page">
      <PageHeader title={t('face.title')} subtitle={t('face.subtitle')} />

      {/* Hero — single source of truth for where the user is in the journey */}
      <div className="card" style={{ padding: '24px 26px' }}>
        <div style={{ display: 'flex', gap: 18, alignItems: 'flex-start', flexWrap: 'wrap' }}>
          <span
            aria-hidden
            style={{
              width: 12,
              height: 12,
              borderRadius: 999,
              background: meta.dot,
              marginTop: 7,
              flexShrink: 0,
              boxShadow: `0 0 0 4px ${meta.dot.replace(')', ' / 0.18)')}`,
            }}
          />
          <div style={{ flex: 1, minWidth: 260 }}>
            <div
              style={{
                fontSize: '1.125rem',
                fontWeight: 700,
                letterSpacing: '-0.3px',
                color: 'var(--text)',
              }}
            >
              {meta.title}
            </div>
            <p className="page-subtitle" style={{ maxWidth: '68ch', marginBottom: 16 }}>
              {meta.subtitle}
            </p>
            {phase === 'ready' ? (
              <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
                {[t('face.badge.models'), t('face.badge.service'), t('face.badge.enrolled')].map(
                  (label) => (
                    <span
                      key={label}
                      style={{
                        display: 'inline-flex',
                        alignItems: 'center',
                        gap: 6,
                        fontSize: 12,
                        fontWeight: 600,
                        color: 'var(--success)',
                        background: 'var(--surface-2)',
                        border: '1px solid var(--border)',
                        borderRadius: 999,
                        padding: '4px 12px',
                      }}
                    >
                      <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden>
                        <circle cx="6" cy="6" r="5" fill="currentColor" opacity="0.22" />
                        <path
                          d="M3.6 6.2l1.8 1.8 3-3.6"
                          stroke="currentColor"
                          strokeWidth="1.4"
                          strokeLinecap="round"
                          strokeLinejoin="round"
                        />
                      </svg>
                      {label}
                    </span>
                  ),
                )}
              </div>
            ) : (
              <div style={{ display: 'flex', gap: 10, flexDirection: 'column', maxWidth: 480 }}>
                {phase === 'off' && (
                  <>
                    <div style={{ fontSize: 14, color: 'var(--text-muted)' }}>
                      {t('face.off.enableNote')}
                    </div>
                    <button
                      className="btn btn-primary"
                      style={{ alignSelf: 'flex-start' }}
                      onClick={() => void toggleMaster(true)}
                      disabled={busy}
                    >
                      {t('face.off.enable')}
                    </button>
                  </>
                )}
                {phase === 'models' && (
                  <button
                    className="btn btn-primary"
                    style={{ alignSelf: 'flex-start', minWidth: 240 }}
                    onClick={() => void downloadModels()}
                    disabled={busy || downloading || modelsInstalled}
                  >
                    {downloading
                      ? downloadProgress !== null
                        ? t('face.downloading', {
                            percent: `${Math.round(downloadProgress)}%`,
                          })
                        : t('face.downloadingNoPct')
                      : t('face.download')}
                  </button>
                )}
                {phase === 'service' && (
                  <button
                    className="btn btn-primary"
                    style={{ alignSelf: 'flex-start' }}
                    onClick={() => void doServiceAction()}
                    disabled={busy}
                  >
                    {svcInstalled ? t('face.service.restart') : t('face.service.install')}
                  </button>
                )}
                {phase === 'enroll' && (
                  <button
                    className="btn btn-primary"
                    style={{ alignSelf: 'flex-start' }}
                    onClick={() => setEnrollOpen(true)}
                    disabled={busy}
                  >
                    {t('face.addMyFace')}
                  </button>
                )}
                {(downloading || downloadProgress !== null) && (
                  <div style={{ width: '100%', maxWidth: 360 }}>
                    <div
                      style={{
                        height: 6,
                        borderRadius: 999,
                        background: 'var(--surface-2)',
                        border: '1px solid var(--border)',
                        overflow: 'hidden',
                      }}
                    >
                      <div
                        style={{
                          height: '100%',
                          width: `${downloadProgress ?? 0}%`,
                          background: 'var(--accent)',
                          borderRadius: 999,
                          transition: 'width 220ms var(--ease)',
                        }}
                      />
                    </div>
                  </div>
                )}
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Journey checklist — 3 compact steps with current state */}
      <div
        className="card"
        style={{
          display: 'grid',
          gap: 14,
          gridTemplateColumns: 'repeat(auto-fit, minmax(200px, 1fr))',
          padding: '16px 22px',
        }}
      >
        {[
          {
            label: t('face.badge.models'),
            done: modelsInstalled,
            active: phase === 'models',
            note: modelsInstalled
              ? t('face.checklist.modelsDone')
              : downloading
                ? t('face.downloadingNoPct')
                : t('face.checklist.modelsNote'),
          },
          {
            label: t('face.badge.service'),
            done: svcRunning,
            active: phase === 'service',
            note: svcRunning
              ? t('face.checklist.serviceRunning')
              : svcInstalled
                ? t('face.checklist.serviceInstalled')
                : t('face.checklist.serviceNote'),
          },
          {
            label: t('face.badge.enrolled'),
            done: hasFaces,
            active: phase === 'enroll',
            note: hasFaces
              ? t('face.checklist.faceCount', { count: enrolledCount })
              : t('face.checklist.faceNote'),
          },
        ].map((step) => (
          <div
            key={step.label}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 10,
              padding: '10px 12px',
              borderRadius: 'var(--r-sm)',
              background: step.active ? 'var(--accent-soft)' : 'var(--surface-2)',
              border: `1px solid ${step.active ? 'var(--accent)' : 'var(--border)'}`,
              transition: 'background 190ms var(--ease), border-color 190ms var(--ease)',
            }}
          >
            <span style={{ flexShrink: 0 }}>
              {step.done ? (
                <svg width="18" height="18" viewBox="0 0 18 18" fill="none" aria-hidden>
                  <circle cx="9" cy="9" r="8" fill="var(--success)" opacity="0.18" />
                  <path
                    d="M5.5 9.3l2.3 2.3 4.7-5"
                    stroke="var(--success)"
                    strokeWidth="1.6"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
              ) : step.active ? (
                <svg width="18" height="18" viewBox="0 0 18 18" fill="none" aria-hidden>
                  <circle cx="9" cy="9" r="8" fill="var(--accent)" opacity="0.2" />
                  <circle
                    cx="9"
                    cy="9"
                    r="8"
                    stroke="var(--accent)"
                    strokeWidth="1.6"
                    strokeDasharray="3 4"
                  />
                </svg>
              ) : (
                <svg width="18" height="18" viewBox="0 0 18 18" fill="none" aria-hidden>
                  <circle cx="9" cy="9" r="8" stroke="var(--text-dim)" strokeWidth="1.4" />
                </svg>
              )}
            </span>
            <span style={{ minWidth: 0 }}>
              <span
                style={{
                  display: 'block',
                  fontSize: 13,
                  fontWeight: 600,
                  color: step.done ? 'var(--text-muted)' : 'var(--text)',
                }}
              >
                {step.label}
              </span>
              <span style={{ display: 'block', fontSize: 12, color: 'var(--text-dim)' }}>
                {step.note}
              </span>
            </span>
          </div>
        ))}
      </div>

      {phase === 'off' && (
        <div className="card" style={{ display: 'flex', gap: 12, alignItems: 'flex-start' }}>
          <svg
            width="16"
            height="16"
            viewBox="0 0 16 16"
            fill="none"
            style={{ marginTop: 2, flexShrink: 0 }}
            aria-hidden
          >
            <circle cx="8" cy="8" r="7" stroke="var(--warning)" strokeWidth="1.3" />
            <path d="M8 4.6v4" stroke="var(--warning)" strokeWidth="1.3" strokeLinecap="round" />
            <circle cx="8" cy="11.1" r="0.9" fill="var(--warning)" />
          </svg>
          <p className="page-subtitle" style={{ margin: 0, maxWidth: '78ch' }}>
            {t('face.risk')}
          </p>
        </div>
      )}

      {diagnostics && (
        <div className="card">
          <h2 className="card-title">{t('face.diagnostics')}</h2>
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fit,minmax(220px,1fr))',
              gap: 10,
            }}
          >
            {Object.entries(diagnostics)
              .filter(([k]) => !k.endsWith('_dir') && k !== 'models_dir' && k !== 'data_dir')
              .map(([key, value]) => {
                const okState =
                  value === true ||
                  String(value) === 'true' ||
                  (typeof value === 'number' && value > 0);
                const isBool = typeof value === 'boolean';
                return (
                  <div
                    key={key}
                    className="stat"
                    style={{
                      background: 'var(--surface-2)',
                      border: '1px solid var(--border)',
                      borderRadius: 'var(--r-sm)',
                      padding: 10,
                    }}
                  >
                    <span
                      className="stat-label"
                      style={{ textTransform: 'none', fontSize: 12, color: 'var(--text-muted)' }}
                    >
                      {key.replace(/_/g, ' ')}
                    </span>
                    <div
                      className="stat-value"
                      style={{
                        fontSize: 14,
                        color: isBool
                          ? okState
                            ? 'var(--success)'
                            : 'var(--error)'
                          : 'var(--text)',
                        fontFamily: isBool ? undefined : 'var(--font-mono)',
                      }}
                    >
                      {isBool
                        ? okState
                          ? t('face.diag.ok')
                          : t('face.diag.failed')
                        : String(value)}
                    </div>
                  </div>
                );
              })}
          </div>
          <div style={{ marginTop: 10, fontSize: 11, color: 'var(--text-muted)', lineHeight: 1.6 }}>
            <div>
              {t('face.diag.modelsDir')} <code>{String(diagnostics.models_dir ?? '')}</code>
            </div>
            <div>
              {t('face.diag.dataDir')} <code>{String(diagnostics.data_dir ?? '')}</code>
            </div>
          </div>
        </div>
      )}
      {diagnosticsError && (
        <div className="card">
          <h2 className="card-title">{t('face.diagnostics_error')}</h2>
          <p style={{ margin: 0, fontSize: 13, color: 'var(--error)' }}>{diagnosticsError}</p>
        </div>
      )}

      {/* Settings — grouped, ready-first */}
      <div className="card">
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            gap: 12,
          }}
        >
          <div>
            <h2 className="card-title" style={{ marginBottom: 4 }}>
              {t('common.settings')}
            </h2>
            <p className="page-subtitle" style={{ margin: 0 }}>
              {t('face.settings.subtitle')}
            </p>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            {dirty && (
              <span style={{ fontSize: 12, color: 'var(--warning)' }}>
                {t('face.unsaved_changes')}
              </span>
            )}
            <button
              className="btn btn-primary"
              onClick={() => void saveSettings()}
              disabled={busy || !dirty}
              style={{ minWidth: 96 }}
            >
              {busy ? t('face.settings.saving') : t('common.save')}
            </button>
          </div>
        </div>

        <div
          style={{
            display: 'grid',
            gap: 18,
            marginTop: 14,
            gridTemplateColumns: 'repeat(auto-fit, minmax(280px, 1fr))',
          }}
        >
          {/* Matching */}
          <div>
            <div
              style={{
                fontSize: 12,
                fontWeight: 600,
                color: 'var(--text-dim)',
                textTransform: 'uppercase',
                letterSpacing: '0.06em',
                marginBottom: 10,
              }}
            >
              {t('face.settings.group.matching')}
            </div>
            <div style={{ display: 'grid', gap: 12 }}>
              <label style={{ display: 'grid', gap: 5 }}>
                <span style={{ fontSize: 13, color: 'var(--text)' }}>
                  {t('face.similarity_threshold')}
                </span>
                <input
                  type="range"
                  min={0.4}
                  max={0.8}
                  step={0.01}
                  value={settings.match_threshold}
                  onChange={(e) => set({ match_threshold: Number(e.target.value) })}
                  style={{ width: '100%', accentColor: 'var(--accent)' }}
                />
                <span style={{ fontSize: 11.5, color: 'var(--text-dim)' }}>
                  {settings.match_threshold < 0.55
                    ? t('face.settings.thresholdLoose')
                    : settings.match_threshold > 0.7
                      ? t('face.settings.thresholdStrict')
                      : t('face.settings.thresholdBalanced')}
                  {' · '}
                  {settings.match_threshold.toFixed(2)}
                </span>
              </label>
              <label
                style={{
                  display: 'flex',
                  justifyContent: 'space-between',
                  alignItems: 'center',
                  gap: 12,
                }}
              >
                <span style={{ fontSize: 13, color: 'var(--text)' }}>
                  {t('face.settings.multiFace')}
                </span>
                <ToggleSwitch
                  checked={settings.multi_face_protection_enabled}
                  onChange={(v) => set({ multi_face_protection_enabled: v })}
                  ariaLabel={t('face.settings.multiFace')}
                />
              </label>
              <label
                style={{
                  display: 'flex',
                  justifyContent: 'space-between',
                  alignItems: 'center',
                  gap: 12,
                }}
              >
                <span style={{ fontSize: 13, color: 'var(--text)' }}>
                  {t('face.settings.liveness')}
                </span>
                <ToggleSwitch
                  checked={settings.liveness_enabled}
                  onChange={(v) => set({ liveness_enabled: v })}
                  ariaLabel={t('face.settings.livenessAria')}
                />
              </label>
              <label
                style={{
                  display: 'flex',
                  justifyContent: 'space-between',
                  alignItems: 'center',
                  gap: 12,
                }}
              >
                <span style={{ fontSize: 13, color: 'var(--text)' }}>
                  {t('face.settings.antispoof')}
                </span>
                <ToggleSwitch
                  checked={settings.antispoof_enabled}
                  onChange={(v) => set({ antispoof_enabled: v })}
                  ariaLabel={t('face.settings.antispoofAria')}
                />
              </label>
            </div>
          </div>

          {/* Where it appears */}
          <div>
            <div
              style={{
                fontSize: 12,
                fontWeight: 600,
                color: 'var(--text-dim)',
                textTransform: 'uppercase',
                letterSpacing: '0.06em',
                marginBottom: 10,
              }}
            >
              {t('face.settings.group.signIn')}
            </div>
            <div style={{ display: 'grid', gap: 12 }}>
              <label
                style={{
                  display: 'flex',
                  justifyContent: 'space-between',
                  alignItems: 'center',
                  gap: 12,
                }}
              >
                <span style={{ fontSize: 13, color: 'var(--text)' }}>
                  {t('face.show_tile_at_sign_in')}
                </span>
                <ToggleSwitch
                  checked={settings.face_unlock_logon_enabled}
                  onChange={(v) => set({ face_unlock_logon_enabled: v })}
                  ariaLabel={t('face.show_tile_at_sign_in')}
                />
              </label>
              <label
                style={{
                  display: 'flex',
                  justifyContent: 'space-between',
                  alignItems: 'center',
                  gap: 12,
                }}
              >
                <span style={{ fontSize: 13, color: 'var(--text)' }}>
                  {t('face.settings.showTileLock')}
                </span>
                <ToggleSwitch
                  checked={settings.face_unlock_workstation_enabled}
                  onChange={(v) => set({ face_unlock_workstation_enabled: v })}
                  ariaLabel={t('face.settings.showTileLockAria')}
                />
              </label>
              <label style={{ display: 'grid', gap: 5 }}>
                <span style={{ fontSize: 13, color: 'var(--text)' }}>
                  {t('face.re_enrollment_reminder')}
                </span>
                <input
                  type="range"
                  min={0}
                  max={365}
                  step={7}
                  value={Math.min(settings.renew_days, 365)}
                  onChange={(e) => set({ renew_days: Number(e.target.value) })}
                  style={{ width: '100%', accentColor: 'var(--accent)' }}
                />
                <span style={{ fontSize: 11.5, color: 'var(--text-dim)' }}>
                  {settings.renew_days === 0
                    ? t('face.settings.renewOff')
                    : t('face.settings.renewEvery', { count: settings.renew_days })}
                </span>
              </label>
              <label style={{ display: 'grid', gap: 5 }}>
                <span style={{ fontSize: 13, color: 'var(--text)' }}>
                  {t('face.settings.lockout')}
                </span>
                <input
                  type="range"
                  min={1}
                  max={10}
                  step={1}
                  value={Math.min(Math.max(settings.lockout_max_fails, 1), 10)}
                  onChange={(e) => set({ lockout_max_fails: Number(e.target.value) })}
                  style={{ width: '100%', accentColor: 'var(--accent)' }}
                />
                <span style={{ fontSize: 11.5, color: 'var(--text-dim)' }}>
                  {t('face.settings.lockoutNote', { count: settings.lockout_max_fails })}
                </span>
              </label>
            </div>
          </div>
        </div>

        {/* Advanced — collapsed by default, stays out of the way */}
        <button
          className="btn btn-ghost"
          onClick={() => setShowAdvanced((v) => !v)}
          style={{ marginTop: 16, fontSize: 13 }}
          aria-expanded={showAdvanced}
        >
          {showAdvanced ? t('face.settings.advancedHide') : t('face.settings.advanced')}
          <svg
            width="14"
            height="14"
            viewBox="0 0 14 14"
            fill="none"
            style={{
              marginLeft: 6,
              transform: showAdvanced ? 'rotate(180deg)' : 'none',
              transition: 'transform 190ms var(--ease)',
            }}
            aria-hidden
          >
            <path
              d="M3.5 5l3.5 3.5L10.5 5"
              stroke="currentColor"
              strokeWidth="1.4"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>
        {showAdvanced && (
          <div style={{ display: 'grid', gap: 12, marginTop: 12, maxWidth: 560 }}>
            <label
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                alignItems: 'center',
                gap: 12,
              }}
            >
              <span style={{ fontSize: 13, color: 'var(--text)' }}>
                {t('face.settings.matchMargin')}
              </span>
              <input
                className="text-input"
                type="number"
                min={0}
                max={1}
                step={0.01}
                value={settings.match_margin}
                onChange={(e) => set({ match_margin: Number(e.target.value) })}
                style={{ width: 90 }}
              />
            </label>
            <label
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                alignItems: 'center',
                gap: 12,
              }}
            >
              <span style={{ fontSize: 13, color: 'var(--text)' }}>
                {t('face.anti_spoof_threshold')}
              </span>
              <input
                className="text-input"
                type="number"
                min={0}
                max={1}
                step={0.05}
                value={settings.antispoof_threshold}
                onChange={(e) => set({ antispoof_threshold: Number(e.target.value) })}
                style={{ width: 90 }}
              />
            </label>
            <span style={{ fontSize: 12, color: 'var(--text-dim)' }}>
              {t('face.settings.advancedHint')}
            </span>
          </div>
        )}
      </div>

      {/* Master toggle + password + templates together under "Manage" */}
      <div className="card">
        <h2
          className="card-title"
          style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}
        >
          <span>{t('face.face_unlock_power')}</span>
          <ToggleSwitch
            checked={settings.face_unlock_enabled}
            onChange={(v) => void toggleMaster(v)}
            disabled={busy}
            ariaLabel={t('face.face_unlock_power')}
          />
        </h2>
        {phase === 'off' ? (
          <p className="page-subtitle" style={{ margin: 0 }}>
            {t('face.powerOffNote')}
          </p>
        ) : (
          <div
            style={{
              display: 'grid',
              gap: 18,
              marginTop: 2,
              gridTemplateColumns: 'repeat(auto-fit, minmax(280px, 1fr))',
            }}
          >
            <div>
              <span
                style={{
                  display: 'block',
                  fontSize: 13,
                  fontWeight: 600,
                  color: 'var(--text)',
                  marginBottom: 6,
                }}
              >
                {t('face.enrolledFaces')}
              </span>
              <TemplateList compact />
              <button
                className="btn btn-secondary"
                style={{ marginTop: 8 }}
                onClick={() => setEnrollOpen(true)}
                disabled={busy || (modelsKnown && !modelsInstalled)}
                title={modelsKnown && !modelsInstalled ? t('face.modelsFirstTitle') : undefined}
              >
                {t('face.addAnother')}
              </button>
            </div>
            <div>
              <span
                style={{
                  display: 'block',
                  fontSize: 13,
                  fontWeight: 600,
                  color: 'var(--text)',
                  marginBottom: 6,
                }}
              >
                {t('face.signInPassword')}
              </span>
              <p className="page-subtitle" style={{ margin: 0 }}>
                {t('face.signInPasswordNote')}
              </p>
              <button
                className="btn btn-secondary"
                style={{ marginTop: 8 }}
                onClick={() => setEnrollOpen(true)}
                disabled={busy || (modelsKnown && !modelsInstalled)}
              >
                {t('face.managePassword')}
              </button>
            </div>
          </div>
        )}
      </div>

      {/* Maintenance — tucked away */}
      <button
        className="btn btn-ghost"
        onClick={() => setShowMaintenance((v) => !v)}
        style={{ fontSize: 13, marginBottom: showMaintenance ? 10 : 0 }}
        aria-expanded={showMaintenance}
      >
        {showMaintenance ? t('face.maintenanceHide') : t('face.maintenance')}
        <svg
          width="14"
          height="14"
          viewBox="0 0 14 14"
          fill="none"
          style={{
            marginLeft: 6,
            transform: showMaintenance ? 'rotate(180deg)' : 'none',
            transition: 'transform 190ms var(--ease)',
          }}
          aria-hidden
        >
          <path
            d="M3.5 5l3.5 3.5L10.5 5"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </button>
      {showMaintenance && (
        <div className="card" style={{ display: 'grid', gap: 10 }}>
          <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', alignItems: 'center' }}>
            <button
              className="btn btn-secondary"
              style={{ marginRight: 0 }}
              onClick={() => void doServiceAction()}
              disabled={busy}
              title={t('face.maint.reinstallTitle')}
            >
              {t('face.maint.reinstall')}
            </button>
            <button
              className="btn btn-secondary"
              onClick={() => void rescanModels()}
              disabled={busy}
              title={t('face.maint.rescanTitle')}
            >
              {t('face.maint.rescan')}
            </button>
            <button
              className="btn btn-secondary"
              onClick={() => void runDiagnostics()}
              disabled={busy}
            >
              {t('face.maint.diagnostics')}
            </button>
          </div>
          <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', alignItems: 'center' }}>
            <button
              className="btn btn-danger"
              onClick={() => void removeAllModels()}
              disabled={busy}
              title={t('face.maint.removeTitle')}
            >
              {t('face.maint.remove')}
            </button>
            <span style={{ fontSize: 12, color: 'var(--text-dim)' }}>
              {t('face.maint.removeNote')}
            </span>
          </div>
        </div>
      )}

      {msg && (
        <div
          role="status"
          style={{
            marginTop: 12,
            padding: '10px 14px',
            borderRadius: 'var(--r-sm)',
            background: msg.type === 'ok' ? 'var(--success)' : 'var(--error)',
            color: 'var(--surface-solid)',
            fontSize: 13,
            fontWeight: 600,
            opacity: 0.95,
          }}
        >
          {msg.text}
        </div>
      )}

      {/* Enroll wizard modal — single modal left in the flow */}
      {enrollOpen && (
        <EnrollWizard
          onClose={() => setEnrollOpen(false)}
          onEnrolled={() => {
            setEnrollOpen(false);
            void load();
            void loadModels();
          }}
        />
      )}
    </div>
  );
}

function TemplateList({ compact = false }: { compact?: boolean }) {
  const [profiles, setProfiles] = useState<{ name: string; templates: number; labels: string[] }[]>(
    [],
  );
  const [busy, setBusy] = useState(false);

  const loadTemplates = useCallback(async () => {
    try {
      const r = await invoke<{ profiles: { name: string; templates: number; labels: string[] }[] }>(
        'face_list_templates',
      );
      setProfiles(r.profiles);
    } catch {
      /* camera/models not built — leave empty */
    }
  }, []);

  useEffect(() => {
    void loadTemplates();
  }, [loadTemplates]);

  const deleteAll = async (name: string) => {
    setBusy(true);
    try {
      // Delete templates from last to first.
      const p = profiles.find((x) => x.name === name);
      if (p) {
        for (let i = p.templates - 1; i >= 0; i--) {
          await invoke('face_delete_template', { name, index: i });
        }
      }
      await loadTemplates();
    } catch (e) {
      console.warn('delete error:', e);
    } finally {
      setBusy(false);
    }
  };

  if (profiles.length === 0) {
    return (
      <p className="page-subtitle" style={{ color: 'var(--text-dim)' }}>
        {t('face.templates.empty')}
      </p>
    );
  }

  return (
    <ul style={{ listStyle: 'none', padding: 0, margin: 0 }}>
      {profiles.map((p) => (
        <li
          key={p.name}
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
            gap: 10,
            padding: compact ? '6px 0' : 8,
            borderBottom: '1px solid var(--border)',
          }}
        >
          <div style={{ minWidth: 0 }}>
            <b>{p.name}</b>{' '}
            <span style={{ color: 'var(--text-dim)' }}>
              {t('face.templates.count', { count: p.templates })}
            </span>
            {p.labels.length > 0 && (
              <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>{p.labels.join(', ')}</div>
            )}
          </div>
          <button
            className="btn btn-secondary btn-sm"
            onClick={() => void deleteAll(p.name)}
            disabled={busy}
            style={{ fontSize: 12, padding: '4px 10px', flexShrink: 0 }}
          >
            {t('face.templates.delete')}
          </button>
        </li>
      ))}
    </ul>
  );
}
