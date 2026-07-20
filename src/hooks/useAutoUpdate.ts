import { useState, useEffect, useCallback, useRef } from 'react';
import { check } from '@tauri-apps/plugin-updater';
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

      await update.downloadAndInstall((event) => {
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

      setState('ready');
      // Auto-relaunch after a short delay
      setTimeout(() => {
        void relaunch();
      }, 1500);
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
