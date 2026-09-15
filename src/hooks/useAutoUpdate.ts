import { useState, useEffect, useCallback, useRef } from 'react';
import { check } from '@tauri-apps/plugin-updater';
import { invoke } from '@tauri-apps/api/core';
import { relaunch } from '@tauri-apps/plugin-process';

export type AppUpdateState =
  'idle' | 'checking' | 'available' | 'downloading' | 'installing' | 'ready' | 'error';

export interface AppUpdateInfo {
  version: string;
  date: string;
  body: string;
}

export function useAutoUpdate() {
  const [state, setState] = useState<AppUpdateState>('idle');
  const [updateInfo, setUpdateInfo] = useState<AppUpdateInfo | null>(null);
  const [progress, setProgress] = useState(0);
  const [errorMsg, setErrorMsg] = useState('');
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const checkForUpdate = useCallback(async (silent = false) => {
    if (!silent) setState('checking');
    try {
      const update = await check();
      if (!update) {
        if (!silent && mountedRef.current) {
          setState('idle');
        }
        return null;
      }

      if (!mountedRef.current) return null;

      setUpdateInfo({
        version: update.version,
        date: update.date ?? '',
        body: update.body ?? '',
      });
      setState('available');
      return update;
    } catch (e) {
      if (!silent && mountedRef.current) {
        setErrorMsg(String(e));
        setState('error');
      }
      return null;
    }
  }, []);

  const downloadAndInstall = useCallback(async () => {
    try {
      setState('downloading');
      setProgress(0);

      const update = await check();
      if (!update) {
        setState('idle');
        return;
      }

      let total = 0;
      let downloaded = 0;

      // S57 FIX: download via the Tauri updater plugin (signature-verified),
      // then install via the MiControlBridge SYSTEM service (`install_update`)
      // instead of the plugin's own `install()`. The plugin launches the NSIS
      // installer with ShellExecuteW verb "open" — NO elevation — so on this
      // perMachine install (RequestExecutionLevel admin + service hooks) every
      // `sc` command failed with "OpenService FAILED 5: Access is denied", the
      // visible wizard was shown, and the update was never applied. The bridge
      // launches the same installer silently (/S /P /UPDATE /R) with SYSTEM
      // privileges, which is the only correct path for a perMachine install.
      await update.download((event) => {
        if (!mountedRef.current) return;
        switch (event.event) {
          case 'Started':
            total = event.data.contentLength ?? 0;
            break;
          case 'Progress':
            downloaded += event.data.chunkLength;
            if (total > 0) {
              setProgress(Math.round((downloaded / total) * 100));
            }
            break;
          case 'Finished':
            setProgress(100);
            break;
        }
      });

      if (!mountedRef.current) return;

      setState('installing');

      // Locate the installer the plugin just wrote to its temp directory and
      // hand it to the bridge. The temp dir is named
      // "<app>-<version>-updater-" + random, and the file is
      // "<app>-<version>-installer.exe".
      const installerPath = await invoke<string | null>('find_latest_downloaded_installer');
      if (!installerPath) {
        throw new Error(
          'Downloaded installer not found after update download — cannot install via bridge.',
        );
      }

      await invoke('install_update', { installerPath });

      if (!mountedRef.current) return;

      setState('ready');
      // The installer's /R flag relaunches the app for us; relaunch() here is
      // a safety net in case /R is skipped (e.g. installer launched late).
      setTimeout(() => {
        void relaunch();
      }, 4000);
    } catch (e) {
      if (!mountedRef.current) return;
      setErrorMsg(String(e));
      setState('error');
    }
  }, []);

  const dismiss = useCallback(() => {
    setState('idle');
    setUpdateInfo(null);
    setProgress(0);
    setErrorMsg('');
  }, []);

  // Check silently on startup (after 3s delay) and every 4 hours
  useEffect(() => {
    const initialTimer = setTimeout(() => {
      void checkForUpdate(true);
    }, 3000);
    const interval = setInterval(
      () => {
        void checkForUpdate(true);
      },
      4 * 60 * 60 * 1000,
    );
    return () => {
      clearTimeout(initialTimer);
      clearInterval(interval);
    };
  }, [checkForUpdate]);

  return {
    state,
    updateInfo,
    progress,
    errorMsg,
    checkForUpdate: () => checkForUpdate(false),
    downloadAndInstall,
    dismiss,
  };
}
