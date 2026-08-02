import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
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
  lockout_max_fails: number;
  lockout_seconds: number;
  face_unlock_enabled: boolean;
  face_unlock_logon_enabled: boolean;
  face_unlock_workstation_enabled: boolean;
}

const DEFAULT_SETTINGS: FaceSettings = {
  match_threshold: 0.4,
  match_margin: 0.05,
  liveness_enabled: true,
  antispoof_enabled: true,
  antispoof_threshold: 0.55,
  lockout_max_fails: 5,
  lockout_seconds: 30,
  face_unlock_enabled: true,
  face_unlock_logon_enabled: true,
  face_unlock_workstation_enabled: true,
};

export default function FaceUnlockTab() {
  const [status, setStatus] = useState<FaceStatus | null>(null);
  const [settings, setSettings] = useState<FaceSettings>(DEFAULT_SETTINGS);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ type: 'ok' | 'err'; text: string } | null>(null);
  const [user, setUser] = useState('');
  const [password, setPassword] = useState('');
  const [diagnostics, setDiagnostics] = useState<string>('');

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

  useEffect(() => {
    void load();
  }, [load]);

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
      show('err', `install error: ${String(e)}`);
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

  const savePassword = async () => {
    if (!user || !password) {
      show('err', 'Enter Windows account name and password.');
      return;
    }
    setBusy(true);
    try {
      await invoke('face_set_password', { user, password });
      setPassword('');
      show('ok', `Password stored for "${user}".`);
    } catch (e) {
      show('err', `password error: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const runDiagnostics = async () => {
    setBusy(true);
    try {
      const d = await invoke<Record<string, unknown>>('face_diagnostics');
      setDiagnostics(JSON.stringify(d, null, 2));
    } catch (e) {
      setDiagnostics(`error: ${String(e)}`);
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
        </div>
      </div>

      {diagnostics && (
        <div className="card">
          <div className="card-title">Diagnostics</div>
          <pre
            style={{
              fontSize: 12,
              whiteSpace: 'pre-wrap',
              background: 'var(--bg-soft, #f5f5f5)',
              padding: 12,
              borderRadius: 8,
            }}
          >
            {diagnostics}
          </pre>
        </div>
      )}

      {/* Settings */}
      <div className="card">
        <div className="card-title">Settings</div>
        <div style={{ display: 'grid', gap: 10, maxWidth: 480 }}>
          <label style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <span>Match threshold (similarity)</span>
            <input
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
            <input
              type="checkbox"
              checked={settings.liveness_enabled}
              onChange={(e) => setSettings({ ...settings, liveness_enabled: e.target.checked })}
            />
          </label>
          <label style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <span>Passive anti-spoof (photo/video)</span>
            <input
              type="checkbox"
              checked={settings.antispoof_enabled}
              onChange={(e) => setSettings({ ...settings, antispoof_enabled: e.target.checked })}
            />
          </label>
          <label style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <span>Failures before lockout</span>
            <input
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
            <input
              type="checkbox"
              checked={settings.face_unlock_logon_enabled}
              onChange={(e) =>
                setSettings({ ...settings, face_unlock_logon_enabled: e.target.checked })
              }
            />
          </label>
          <label style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <span>Show tile at workstation unlock (Win+L)</span>
            <input
              type="checkbox"
              checked={settings.face_unlock_workstation_enabled}
              onChange={(e) =>
                setSettings({ ...settings, face_unlock_workstation_enabled: e.target.checked })
              }
            />
          </label>
        </div>
        <div style={{ marginTop: 12 }}>
          <button className="btn btn-primary" onClick={() => void saveSettings()} disabled={busy}>
            💾 Save settings
          </button>
        </div>
      </div>

      {/* Password */}
      <div className="card">
        <div className="card-title">Sign-in password (LSA)</div>
        <p className="page-subtitle">
          The password is stored in an LSA Secret and read only by the Credential Provider at the
          lock screen — it never crosses the network or the service pipe. Use your Windows account
          password (not a PIN).
        </p>
        <div style={{ display: 'flex', gap: 8, maxWidth: 560, flexWrap: 'wrap' }}>
          <input
            placeholder="Windows account name"
            value={user}
            onChange={(e) => setUser(e.target.value)}
            style={{ flex: 1, minWidth: 160 }}
          />
          <input
            type="password"
            placeholder="Password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            style={{ flex: 1, minWidth: 160 }}
          />
          <button className="btn btn-primary" onClick={() => void savePassword()} disabled={busy}>
            🔐 Store password
          </button>
        </div>
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
    </div>
  );
}

function TemplateList() {
  const [profiles, setProfiles] = useState<{ name: string; templates: number; labels: string[] }[]>(
    [],
  );

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
        </li>
      ))}
    </ul>
  );
}
