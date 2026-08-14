import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import InfoModal from '../../components/InfoModal';
import ToggleSwitch from '../../components/ToggleSwitch';
import { PageHeader } from './PageHeader';

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

type WizardStep = 'experimental' | 'models' | 'service' | 'done';

/**
 * Setup wizard modals:
 *  1. ⚠️ experimental warning (mandatory acceptance)
 *  2. models download (progress bar via `face-model-progress` event)
 *  3. auth-service install
 *  4. done → hand-off to the camera enroll modal
 */
function SetupWizard({ onClose, onDone }: { onClose: () => void; onDone: () => void }) {
  const [step, setStep] = useState<WizardStep>('experimental');
  const [accepted, setAccepted] = useState(false);
  const [progress, setProgress] = useState<number | null>(null);
  const [installMsg, setInstallMsg] = useState<string>('');
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    // Live download progress.
    const un = listen<number>('face-model-progress', (e) => {
      setProgress(e.payload);
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  const finishError = (e: unknown): string => getFriendlyErr(e);

  const startDownload = async () => {
    setBusy(true);
    setErr(null);
    setProgress(0);
    try {
      await invoke('face_download_models');
      // Optional copy into Program Files (needs admin; silently skips if not).
      try {
        await invoke('face_install_models');
      } catch {
        // Models are staged — the auth service can read the staging dir too.
        // Program Files copy will be attempted by the installer on reinstall.
      }
      setStep('service');
    } catch (e) {
      setErr(finishError(e));
    } finally {
      setBusy(false);
    }
  };

  const installSvc = async () => {
    setBusy(true);
    setErr(null);
    setInstallMsg('');
    try {
      const r = await invoke<{ ok: boolean; stdout: string; stderr: string }>(
        'face_service_install',
      );
      if (!r.ok) {
        setErr(`install failed: ${r.stderr}`);
      } else {
        setInstallMsg(r.stdout || 'Service installed & started.');
        setStep('done');
      }
    } catch (e) {
      setErr(finishError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <InfoModal open onClose={onClose} title="Face Unlock — Setup wizard">
      {/* 1. Experimental warning */}
      {step === 'experimental' && (
        <div style={{ display: 'grid', gap: 14 }}>
          <div
            style={{
              padding: 12,
              borderRadius: 8,
              background: 'rgba(230,162,60,.12)',
              borderLeft: '4px solid #e6a23c',
            }}
          >
            <b style={{ color: '#e6a23c' }}>⚠️ Experimental system</b>
          </div>
          <p style={{ margin: 0, fontSize: 13.5, lineHeight: 1.6 }}>
            This is an <b>experimental</b> face-recognition system. It does <b>not</b> provide the
            same level of security as <b>Windows Hello</b> (which uses a dedicated infrared sensor
            and dedicated hardware).
          </p>
          <p style={{ margin: 0, fontSize: 13.5, lineHeight: 1.6 }}>
            A <b>photo or video</b> of you presented to the camera may be able to unlock the device.
            Do not enable this on a machine storing sensitive data.
          </p>
          <label style={{ display: 'flex', gap: 10, alignItems: 'center', fontSize: 13.5 }}>
            <ToggleSwitch
              checked={accepted}
              onChange={setAccepted}
              ariaLabel="I understand this is experimental"
            />
            <span>
              I understand this is <b>experimental</b>, that it is{' '}
              <b>not as secure as Windows Hello</b>, and that I use it at <b>my own risk</b> (“o uso
              é por conta e risco do usuário”).
            </span>
          </label>
          <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
            <button className="btn btn-secondary" onClick={onClose}>
              Cancel
            </button>
            <button
              className="btn btn-primary"
              disabled={!accepted}
              onClick={() => setStep('models')}
            >
              I understand — continue
            </button>
          </div>
        </div>
      )}

      {/* 2. Models download */}
      {step === 'models' && (
        <div style={{ display: 'grid', gap: 12 }}>
          <p style={{ margin: 0, fontSize: 13.5, lineHeight: 1.6 }}>
            Step 1/2 — Download the AI models (~250 MB, InsightFace <code>buffalo_l</code>). This
            happens once; they are stored locally.
          </p>
          {progress !== null && (
            <div
              style={{
                height: 10,
                borderRadius: 5,
                background: 'var(--bg-soft, #eee)',
                overflow: 'hidden',
              }}
            >
              <div
                style={{
                  height: '100%',
                  width: `${progress}%`,
                  background: '#4caf50',
                  transition: 'width .3s ease',
                }}
              />
            </div>
          )}
          <p style={{ margin: 0, fontSize: 12, color: 'var(--text-muted)' }}>
            {progress !== null ? `${progress}%` : 'Ready to download'}
          </p>
          {err && <p style={{ margin: 0, color: '#f44336', fontSize: 13 }}>{err}</p>}
          <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
            <button
              className="btn btn-secondary"
              disabled={busy}
              onClick={() => setStep('experimental')}
            >
              Back
            </button>
            <button
              className="btn btn-primary"
              disabled={busy}
              onClick={() => void startDownload()}
            >
              {progress !== null && progress < 100
                ? `⬇️ Downloading ${progress}%…`
                : '⬇️ Download models'}
            </button>
          </div>
        </div>
      )}

      {/* 3. Service install */}
      {step === 'service' && (
        <div style={{ display: 'grid', gap: 12 }}>
          <p style={{ margin: 0, fontSize: 13.5, lineHeight: 1.6 }}>
            Step 2/2 — Install &amp; start the MiControlFace auth service (runs as LocalSystem). You
            may see a UAC prompt.
          </p>
          {installMsg && <p style={{ margin: 0, color: '#4caf50', fontSize: 13 }}>{installMsg}</p>}
          {err && <p style={{ margin: 0, color: '#f44336', fontSize: 13 }}>{err}</p>}
          <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
            <button className="btn btn-secondary" disabled={busy} onClick={onClose}>
              Cancel
            </button>
            <button className="btn btn-primary" disabled={busy} onClick={() => void installSvc()}>
              {busy ? '⏳ Installing…' : '🛠️ Install auth service'}
            </button>
          </div>
        </div>
      )}

      {/* 4. Done */}
      {step === 'done' && (
        <div style={{ display: 'grid', gap: 12 }}>
          <p style={{ margin: 0, fontSize: 13.5, lineHeight: 1.6 }}>
            ✅ <b>Setup complete.</b> Models are ready and the auth service is running.
          </p>
          <p style={{ margin: 0, fontSize: 13.5, lineHeight: 1.6 }}>
            Next, enroll your face so unlock has a reference to match against:
          </p>
          <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
            <button className="btn btn-secondary" onClick={onClose}>
              Later
            </button>
            <button className="btn btn-primary" onClick={onDone}>
              📷 Enroll my face now
            </button>
          </div>
        </div>
      )}
    </InfoModal>
  );
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
      setMsg({ type: 'err', text: 'Enter the Windows sign-in password.' });
      return;
    }
    setBusy(true);
    setMsg(null);
    try {
      await invoke('face_set_password', { user: name, password });
      setPwConfigured(true);
      setPassword('');
      setMsg({ type: 'ok', text: `Password stored for "${name}" (LSA Secret).` });
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
      setMsg({ type: 'err', text: 'Pick a Windows account first.' });
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
        text: `Enrolled "${r.name}" (${r.frames} frames, label "${r.label}").`,
      });
      onEnrolled();
    } catch (e) {
      setMsg({ type: 'err', text: getFriendlyErr(e) });
    } finally {
      setBusy(false);
    }
  };

  const stepTitle: Record<EnrollStep, string> = {
    user: '1 · Windows account',
    hello: '2 · Verify with PIN / fingerprint',
    password: pwConfigured ? '3 · Unlock credential saved ✓' : '3 · Unlock credential (optional)',
    camera: '4 · Camera capture',
  };

  return (
    <InfoModal
      open
      onClose={() => {
        stopPreview();
        onClose();
      }}
      title={`Enroll face — ${stepTitle[step]}`}
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
                {done ? '✓ ' : `${i + 1}. `}
                {s === 'password' ? 'credential' : s === 'camera' ? 'capture' : s}
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
                Which <b>Windows account</b> should face unlock work for? The wizard only needs your
                sign-in permission.
              </p>
              <label style={{ display: 'grid', gap: 5 }}>
                <span style={{ fontSize: 12, color: 'var(--text-muted)', fontWeight: 600 }}>
                  Account
                </span>
                <select
                  className="text-input"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  disabled={userList.length === 0 || busy}
                >
                  {userList.length === 0 && (
                    <option value="">
                      {userListErr ? '— could not load users —' : 'Loading users…'}
                    </option>
                  )}
                  {userList.map((u) => (
                    <option key={u.name} value={u.name}>
                      {u.name}
                    </option>
                  ))}
                </select>
                {userListErr && (
                  <span style={{ fontSize: 11.5, color: 'var(--warning)' }}>⚠️ {userListErr}</span>
                )}
              </label>
              <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap' }}>
                <label style={{ display: 'grid', gap: 5, flex: 1, minWidth: 140 }}>
                  <span style={{ fontSize: 12, color: 'var(--text-muted)', fontWeight: 600 }}>
                    Label
                  </span>
                  <input
                    className="text-input"
                    placeholder="e.g. front, glasses"
                    value={label}
                    onChange={(e) => setLabel(e.target.value)}
                  />
                </label>
                <label style={{ display: 'grid', gap: 5, width: 110 }}>
                  <span style={{ fontSize: 12, color: 'var(--text-muted)', fontWeight: 600 }}>
                    Photos
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
                Next, Windows will ask you to prove it&apos;s really you with the{' '}
                <b>PIN or fingerprint</b> you already configured. No password is typed here.
              </p>
            </div>
            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
              <button className="btn btn-secondary" disabled={busy} onClick={onClose}>
                Cancel
              </button>
              <button
                className="btn btn-primary"
                disabled={busy || !name.trim()}
                onClick={() => setStep('hello')}
              >
                Continue →
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
                This is your <b>everyday sign-in</b> — the very first thing we need is proof that
                it&apos;s really you. A Windows dialog will appear. Use the{' '}
                <b>same PIN or fingerprint</b> you use to unlock this PC.
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
                ← Back
              </button>
              <button className="btn btn-primary" disabled={busy} onClick={() => void doHello()}>
                {busy ? '⏳ Waiting…' : '🔑 Verify with PIN / fingerprint'}
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
                    The unlock credential for <b>{name}</b> is <b>already stored</b> — nothing else
                    to do here. You&apos;ll keep signing in with your PIN as usual.
                  </p>
                  <p
                    style={{ margin: 0, fontSize: 12, color: 'var(--text-muted)', lineHeight: 1.5 }}
                  >
                    Want to change it? Go back to the previous step, clear the saved credential, and
                    re-enter the password when you next enroll.
                  </p>
                </>
              ) : (
                <>
                  <p style={{ margin: 0, fontSize: 13.5, lineHeight: 1.6 }}>
                    <b>Optional.</b> To let the camera unlock the lock screen, Windows needs the{' '}
                    <b>account credential itself</b>&nbsp;— the PIN is protected by Windows and it
                    will not hand it over to third-party software. We store that credential{' '}
                    <b>sealed in the Windows LSA</b>, read it only at the lock screen, and it never
                    leaves this PC.
                  </p>
                  <input
                    className="text-input"
                    type="password"
                    placeholder={`Windows account password for ${name} (used once, stored sealed)`}
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
                    You will <b>keep signing in with your PIN</b>. The camera just needs this once
                    to unlock the screen — you can skip and add it later.
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
                ← Back
              </button>
              {!pwConfigured && (
                <button
                  className="btn btn-ghost"
                  disabled={busy}
                  onClick={() => void savePassword(true)}
                >
                  Skip — PIN only
                </button>
              )}
              <button
                className="btn btn-primary"
                disabled={busy || (!pwConfigured && !password)}
                onClick={() => void savePassword(pwConfigured === true)}
              >
                {busy
                  ? '⏳ Storing…'
                  : pwConfigured
                    ? 'Continue →'
                    : '🔐 Save credential & continue'}
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
                  alt="camera preview"
                  style={{ width: '100%', height: '100%', objectFit: 'contain' }}
                />
              ) : (
                <span style={{ color: 'var(--text-dim)', fontSize: 13 }}>
                  {camErr ? `Camera error: ${camErr}` : 'Starting camera…'}
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
                Live preview — look directly at the camera
              </div>
            </div>
            {camErr && (
              <p style={{ margin: 0, color: 'var(--warning)', fontSize: 12.5 }}>
                ⚠️ {camErr} — check that the camera is not in use by another app, then close and
                reopen this dialog.
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
                Captures <b>{frames} frame(s)</b> for <b>{name}</b> and stores a{' '}
                <b>reference feature vector</b> — no photos are saved, only math.
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
                ← Back
              </button>
              <button className="btn btn-primary" disabled={busy} onClick={() => void doCapture()}>
                {busy ? '⏳ Capturing…' : `📷 Capture ${frames} frame${frames !== 1 ? 's' : ''}`}
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
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ type: 'ok' | 'err'; text: string } | null>(null);
  const [diagnostics, setDiagnostics] = useState<Record<string, unknown> | null>(null);
  const [diagnosticsError, setDiagnosticsError] = useState<string | null>(null);
  const [wizardOpen, setWizardOpen] = useState(false);
  const [enrollOpen, setEnrollOpen] = useState(false);

  const load = useCallback(async () => {
    try {
      const st = await invoke<FaceStatus>('face_status');
      setStatus(st);
      const s = await invoke<FaceSettings>('face_get_settings');
      setSettings({ ...DEFAULT_SETTINGS, ...s });
    } catch (e) {
      setMsg({ type: 'err', text: `load: ${String(e)}` });
    } finally {
      setLoading(false);
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
    return () => {
      void un.then((f) => f());
    };
  }, [load, loadModels]);

  const show = (type: 'ok' | 'err', text: string) => {
    setMsg({ type, text });
    setTimeout(() => setMsg(null), 6000);
  };

  const installService = async () => {
    setBusy(true);
    try {
      const r = await invoke<{ ok: boolean; stdout: string; stderr: string }>(
        'face_service_install',
      );
      show(
        r.ok ? 'ok' : 'err',
        r.ok ? 'Service installed & started.' : `install failed: ${r.stderr}`,
      );
      await load();
    } catch (e) {
      show('err', `install error: ${getFriendlyErr(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const saveSettings = async () => {
    setBusy(true);
    try {
      await invoke('face_set_settings', { settings });
      show('ok', 'Settings saved.');
    } catch (e) {
      show('err', `settings error: ${String(e)}`);
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
    if (
      !window.confirm(
        'Remove all downloaded/installed Face Unlock models?\n\n' +
          'This frees disk space (~250 MB) and disables face enrollment/unlock ' +
          'until the models are downloaded again.',
      )
    ) {
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
          ? `Removed ${r.removed.length} file(s).${r.warnings.length ? ` Warnings: ${r.warnings.join('; ')}` : ''}`
          : 'No model files found — nothing to remove.',
      );
      await loadModels();
    } catch (e) {
      show('err', `remove error: ${getFriendlyErr(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const rescanModels = async () => {
    setBusy(true);
    try {
      const m = await loadModels();
      const present = !!m && (m.installed || m.staged);
      show(
        'ok',
        present
          ? 'Models present — download button stays disabled.'
          : 'No models found — download is enabled again.',
      );
    } catch (e) {
      show('err', `rescan error: ${getFriendlyErr(e)}`);
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

  const toggleMaster = async (enabled: boolean) => {
    setBusy(true);
    try {
      await invoke('face_set_settings', {
        settings: { ...settings, face_unlock_enabled: enabled },
      });
      setSettings((s) => ({ ...s, face_unlock_enabled: enabled }));
      show('ok', enabled ? 'Face Unlock enabled.' : 'Face Unlock disabled (tile hidden).');
    } catch (e) {
      show('err', `toggle error: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="page">
      <PageHeader
        title="Face Unlock"
        subtitle="Windows Hello-style face sign-in using the built-in RGB webcam"
      />

      {/* ⚠️ Security notice */}
      <div className="card" style={{ borderLeft: '4px solid #e6a23c' }}>
        <div className="card-title">⚠️ Security notice</div>
        <p className="page-subtitle" style={{ margin: 0 }}>
          Face unlock here uses a <b>single RGB camera</b> — it is far less secure than the infrared
          Windows Hello sensor. A high-quality photo or video may bypass it. Do not enable this on a
          machine storing sensitive data. A <b>restore point</b> is recommended before enabling the
          lock-screen provider.
        </p>
      </div>

      {/* Status */}
      <div className="card">
        <div className="card-title">Status</div>
        {loading ? (
          <p className="page-subtitle">Loading…</p>
        ) : (
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fit,minmax(180px,1fr))',
              gap: 12,
            }}
          >
            <div className="stat">
              <span className="stat-value">{status?.service_running ? '✅' : '❌'}</span>
              <span className="stat-label">Auth service</span>
            </div>
            <div className="stat">
              <span className="stat-value">{status?.pipe_available ? '✅' : '❌'}</span>
              <span className="stat-label">Service pipe</span>
            </div>
            <div className="stat">
              <span className="stat-value">{status?.camera_available ? '✅' : '❌'}</span>
              <span className="stat-label">Webcam</span>
            </div>
            <div className="stat">
              <span className="stat-value">{status?.enrolled_profiles ?? 0}</span>
              <span className="stat-label">Enrolled profiles</span>
            </div>
          </div>
        )}
        <div style={{ marginTop: 12 }}>
          <button
            className="btn btn-primary"
            onClick={() => setWizardOpen(true)}
            disabled={busy || modelsInstalled}
            title={
              modelsInstalled
                ? 'Module already installed — use "Remove all modules" to re-download it.'
                : undefined
            }
            style={{ marginRight: 8, opacity: modelsInstalled ? 0.55 : 1 }}
          >
            ⬇️ Baixar e instalar módulo
          </button>
          {modelsInstalled && (
            <span style={{ fontSize: 11.5, color: 'var(--success)', marginRight: 8 }}>
              ✅ Module installed
            </span>
          )}
          <button
            className="btn btn-secondary"
            onClick={() => void rescanModels()}
            disabled={busy}
            title="Re-scan for downloaded/installed modules"
            style={{ marginRight: 8 }}
          >
            🔍 Re-scan modules
          </button>
          <button
            className="btn btn-secondary"
            onClick={() => void removeAllModels()}
            disabled={busy}
            title="Delete downloaded models (frees ~250 MB)"
            style={{ marginRight: 8 }}
          >
            🗑️ Remove all modules
          </button>
          <button
            className="btn btn-secondary"
            onClick={() => void installService()}
            disabled={busy}
          >
            ⚙️ Install / start auth service
          </button>
          <button
            className="btn btn-secondary"
            style={{ marginLeft: 8 }}
            onClick={() => void runDiagnostics()}
            disabled={busy}
          >
            🔍 Diagnostics
          </button>
          <button
            className="btn btn-secondary"
            style={{ marginLeft: 8 }}
            onClick={() => setEnrollOpen(true)}
            disabled={busy || (modelsKnown && !modelsInstalled)}
            title={
              modelsKnown && !modelsInstalled
                ? 'Download the module first — no recognition models are present.'
                : undefined
            }
          >
            📷 Camera preview / enroll
          </button>
        </div>
      </div>

      {diagnostics && (
        <div className="card">
          <div className="card-title">🔍 Diagnostics</div>
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
                      {isBool ? (okState ? '✅ OK' : '❌ Failed') : String(value)}
                    </div>
                  </div>
                );
              })}
          </div>
          <div style={{ marginTop: 10, fontSize: 11, color: 'var(--text-muted)', lineHeight: 1.6 }}>
            <div>
              Models dir: <code>{String(diagnostics.models_dir ?? '')}</code>
            </div>
            <div>
              Data dir: <code>{String(diagnostics.data_dir ?? '')}</code>
            </div>
          </div>
        </div>
      )}
      {diagnosticsError && (
        <div className="card" style={{ borderLeft: '4px solid var(--error)' }}>
          <div className="card-title">Diagnostics error</div>
          <p style={{ margin: 0, fontSize: 13, color: 'var(--error)' }}>{diagnosticsError}</p>
        </div>
      )}

      {/* Master toggle */}
      <div className="card">
        <div
          className="card-title"
          style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}
        >
          <span>Face Unlock enabled</span>
          <ToggleSwitch
            checked={settings.face_unlock_enabled}
            onChange={(v) => void toggleMaster(v)}
            disabled={busy}
            ariaLabel="Face Unlock enabled"
          />
        </div>
        <p className="page-subtitle" style={{ margin: 0 }}>
          When disabled, the Face Unlock tile is hidden from the lock screen but your enrolled
          faces, settings and stored password are kept.
        </p>
      </div>

      {/* Enrollment — single flow */}
      <div className="card">
        <div className="card-title">Enroll a face</div>
        <p className="page-subtitle">
          One guided flow: pick your <b>Windows account</b> → confirm with <b>Windows Hello</b> (PIN
          / fingerprint / face) → store the sign-in password once → capture your face. The camera
          stores a feature vector — no photos.
        </p>
        <button
          className="btn btn-primary"
          onClick={() => setEnrollOpen(true)}
          disabled={busy || (modelsKnown && !modelsInstalled)}
          title={
            modelsKnown && !modelsInstalled
              ? 'Download the module first — no recognition models are present.'
              : undefined
          }
        >
          📷 Enroll my face
        </button>
        <p style={{ fontSize: 12, color: 'var(--text-muted)', marginTop: 8 }}>
          Tip: enroll 2+ templates (different angles/lighting) to improve unlock reliability.
        </p>
      </div>

      {/* Settings */}
      <div className="card">
        <div className="card-title">Settings</div>
        <div style={{ display: 'grid', gap: 10, maxWidth: 480 }}>
          <label style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <span>Match threshold (similarity)</span>
            <input
              className="text-input"
              type="number"
              min={0}
              max={1}
              step={0.05}
              value={settings.match_threshold}
              onChange={(e) =>
                setSettings({ ...settings, match_threshold: Number(e.target.value) })
              }
              style={{ width: 80 }}
            />
          </label>
          <label style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <span>Active liveness (blink/turn)</span>
            <ToggleSwitch
              checked={settings.liveness_enabled}
              onChange={(v) => setSettings({ ...settings, liveness_enabled: v })}
            />
          </label>
          <label style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <span>Passive anti-spoof (photo/video)</span>
            <ToggleSwitch
              checked={settings.antispoof_enabled}
              onChange={(v) => setSettings({ ...settings, antispoof_enabled: v })}
            />
          </label>
          <label style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <span>Failures before lockout</span>
            <input
              className="text-input"
              type="number"
              min={1}
              max={20}
              value={settings.lockout_max_fails}
              onChange={(e) =>
                setSettings({ ...settings, lockout_max_fails: Number(e.target.value) })
              }
              style={{ width: 80 }}
            />
          </label>
          <label style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <span>Show tile at sign-in</span>
            <ToggleSwitch
              checked={settings.face_unlock_logon_enabled}
              onChange={(v) => setSettings({ ...settings, face_unlock_logon_enabled: v })}
            />
          </label>
          <label style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <span>Show tile at workstation unlock (Win+L)</span>
            <ToggleSwitch
              checked={settings.face_unlock_workstation_enabled}
              onChange={(v) => setSettings({ ...settings, face_unlock_workstation_enabled: v })}
            />
          </label>
          <label style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <span>Match margin (anti-misrouting)</span>
            <input
              className="text-input"
              type="number"
              min={0}
              max={1}
              step={0.01}
              value={settings.match_margin}
              onChange={(e) => setSettings({ ...settings, match_margin: Number(e.target.value) })}
              style={{ width: 80 }}
            />
          </label>
          <label style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <span>Anti-spoof threshold</span>
            <input
              className="text-input"
              type="number"
              min={0}
              max={1}
              step={0.05}
              value={settings.antispoof_threshold}
              onChange={(e) =>
                setSettings({ ...settings, antispoof_threshold: Number(e.target.value) })
              }
              style={{ width: 80 }}
            />
          </label>
          <label style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <span>Reject if 2+ faces in frame</span>
            <ToggleSwitch
              checked={settings.multi_face_protection_enabled}
              onChange={(v) => setSettings({ ...settings, multi_face_protection_enabled: v })}
            />
          </label>
          <label style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <span>Re-enrollment reminder (days)</span>
            <input
              className="text-input"
              type="number"
              min={0}
              max={3650}
              value={settings.renew_days}
              onChange={(e) => setSettings({ ...settings, renew_days: Number(e.target.value) })}
              style={{ width: 80 }}
            />
          </label>
        </div>
        <div style={{ marginTop: 12 }}>
          <button className="btn btn-primary" onClick={() => void saveSettings()} disabled={busy}>
            💾 Save settings
          </button>
        </div>
      </div>

      {/* Sign-in password (LSA) — managed inside the enroll wizard */}
      <div className="card">
        <div className="card-title">Sign-in password (LSA)</div>
        <p className="page-subtitle">
          The password is stored in an LSA Secret and read only by the Credential Provider at the
          lock screen — it never crosses the network or the service pipe. Use your Windows account
          password (not a PIN). Set or update it inside the <b>enroll wizard</b> below.
        </p>
        <button
          className="btn btn-secondary"
          onClick={() => setEnrollOpen(true)}
          disabled={busy || (modelsKnown && !modelsInstalled)}
        >
          🔐 Set / update password
        </button>
      </div>

      {/* Templates */}
      <div className="card">
        <div className="card-title">Enrolled faces</div>
        <p className="page-subtitle">
          Face enrollment requires the camera + recognition pipeline (built with the{' '}
          <code>face</code> feature). Templates store feature vectors only — no photos — encrypted
          with DPAPI machine scope.
        </p>
        <TemplateList />
      </div>

      {msg && (
        <div
          style={{
            marginTop: 12,
            padding: 10,
            borderRadius: 8,
            background: msg.type === 'ok' ? 'rgba(76,175,80,.15)' : 'rgba(244,67,54,.15)',
            color: msg.type === 'ok' ? '#4caf50' : '#f44336',
          }}
        >
          {msg.text}
        </div>
      )}

      {/* Setup wizard + enroll wizard modals */}
      {wizardOpen && (
        <SetupWizard
          onClose={() => setWizardOpen(false)}
          onDone={() => {
            setWizardOpen(false);
            setEnrollOpen(true);
          }}
        />
      )}
      {enrollOpen && (
        <EnrollWizard
          onClose={() => setEnrollOpen(false)}
          onEnrolled={() => {
            setEnrollOpen(false);
            void load();
          }}
        />
      )}
    </div>
  );
}

function TemplateList() {
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
        No faces enrolled yet.
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
            padding: 8,
            borderBottom: '1px solid var(--border, #eee)',
          }}
        >
          <div>
            <b>{p.name}</b>{' '}
            <span style={{ color: 'var(--text-dim)' }}>
              ({p.templates} template{p.templates !== 1 ? 's' : ''})
            </span>
            {p.labels.length > 0 && (
              <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>{p.labels.join(', ')}</div>
            )}
          </div>
          <button
            className="btn btn-secondary btn-sm"
            onClick={() => void deleteAll(p.name)}
            disabled={busy}
            style={{ fontSize: 12, padding: '4px 10px' }}
          >
            🗑 Delete
          </button>
        </li>
      ))}
    </ul>
  );
}
