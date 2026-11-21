import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import ToggleSwitch from '../../components/ToggleSwitch';
import { PageHeader } from './PageHeader';
import { t } from '../../hooks/useI18n';
import { Icon, type IconName } from '../../components/ui/icons';
import { InfoHint } from '../../components/ui/info-hint';

// ── Types matching Rust structs ──────────────────────────────────────────────

interface CleanupItem {
  category: string;
  description: string;
  size_bytes: number;
  file_count: number;
}

interface CleanupResult {
  category: string;
  freed_bytes: number;
  files_removed: number;
  files_skipped: number;
  errors: string[];
}

// ── Helpers ──────────────────────────────────────────────────────────────────

function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`;
}

const CATEGORY_ICONS: Record<string, IconName> = {
  windows_temp: 'archive',
  windows_update_cache: 'package',
  browser_cache: 'globe',
  recycle_bin: 'trash',
  thumbnail_cache: 'image',
  windows_logs: 'clipboard',
};

/**
 * Só as categorias cujo custo não se lê no nome. "Cache do browser" diz tudo;
 * "Recycle Bin" não diz que é a única que não volta atrás, e "Windows Update
 * cache" não diz que o Windows a volta a descarregar.
 */
const CATEGORY_HINTS: Record<string, string> = {
  recycle_bin: 'hint.cleanupRecycleBin',
  windows_update_cache: 'hint.cleanupUpdateCache',
};

// ── Component ────────────────────────────────────────────────────────────────

export default function CleanupTab() {
  const [items, setItems] = useState<CleanupItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [scanning, setScanning] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [results, setResults] = useState<CleanupResult[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const fetchItems = useCallback(async () => {
    setScanning(true);
    setErrorMsg(null);
    try {
      const scanned = await invoke<CleanupItem[]>('scan_junk_files');
      setItems(scanned);
      // Select all by default
      setSelected(new Set(scanned.map((i) => i.category)));
    } catch (e) {
      setErrorMsg(String(e));
    }
    setScanning(false);
    setLoading(false);
  }, []);

  useEffect(() => {
    void fetchItems();
  }, [fetchItems]);

  const handleClean = async () => {
    if (selected.size === 0) return;
    setCleaning(true);
    setErrorMsg(null);
    try {
      const categories = Array.from(selected);
      const res = await invoke<CleanupResult[]>('clean_junk_files', { categories });
      setResults(res);
      // Re-scan after cleanup
      void fetchItems();
    } catch (e) {
      setErrorMsg(String(e));
    }
    setCleaning(false);
  };

  const toggleCategory = (category: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(category)) {
        next.delete(category);
      } else {
        next.add(category);
      }
      return next;
    });
  };

  const totalSize = items
    .filter((i) => selected.has(i.category))
    .reduce((sum, i) => sum + i.size_bytes, 0);
  const totalFiles = items
    .filter((i) => selected.has(i.category))
    .reduce((sum, i) => sum + i.file_count, 0);

  if (loading) {
    return (
      <>
        <PageHeader title={t('cleanup.title')} subtitle={t('cleanup.subtitle')} />
        <div className="loading-spinner">{t('common.loading')}</div>
      </>
    );
  }

  return (
    <>
      <PageHeader title={t('cleanup.title')} subtitle={t('cleanup.subtitle')} />

      {errorMsg && (
        <div className="alert alert-error" style={{ marginBottom: 16 }}>
          <Icon name="alert-triangle" size={16} /> {errorMsg}
        </div>
      )}

      {/* Cleanup Results */}
      {results && (
        <div
          className={`alert ${results.some((r) => r.errors.length > 0) ? 'alert-warn' : 'alert-success'}`}
          style={{ marginBottom: 16 }}
        >
          <Icon name="check-circle" size={16} /> {t('cleanup.complete')}
          {results.some((r) => r.errors.length > 0) && ` — ${t('cleanup.partial')}`}
          <ul style={{ marginTop: 8, fontSize: 12 }}>
            {results.map((r) => (
              <li key={r.category}>
                <Icon name={CATEGORY_ICONS[r.category] ?? 'folder'} size={16} /> {r.category}:{' '}
                {formatBytes(r.freed_bytes)} {t('cleanup.freed')}, {r.files_removed}{' '}
                {t('cleanup.filesRemoved')}
                {r.files_skipped > 0 && ` (${r.files_skipped} ${t('cleanup.skipped')})`}
                {r.errors.length > 0 && (
                  <div style={{ color: 'var(--error, #e5484d)', marginTop: 2 }}>
                    <Icon name="alert-triangle" size={16} /> {r.errors[0]}
                    {r.errors.length > 1 && ` (+${r.errors.length - 1})`}
                  </div>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}

      {/* Scan & Clean Actions */}
      <div className="card" style={{ marginBottom: 16 }}>
        <h3>{t('cleanup.actions')}</h3>
        <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', alignItems: 'center' }}>
          <button
            className="btn btn-secondary"
            onClick={fetchItems}
            disabled={scanning || cleaning}
          >
            {scanning ? (
              <>
                <Icon name="search" size={16} /> {t('cleanup.scanning')}
              </>
            ) : (
              <>
                <Icon name="search" size={16} /> {t('cleanup.rescan')}
              </>
            )}
          </button>
          <button
            className="btn btn-primary"
            onClick={handleClean}
            disabled={cleaning || selected.size === 0}
          >
            {cleaning ? (
              <>
                <Icon name="broom" size={16} /> {t('cleanup.cleaning')}
              </>
            ) : (
              <>
                <Icon name="broom" size={16} /> {t('cleanup.cleanNow')}
              </>
            )}
          </button>
          {selected.size > 0 && (
            <span className="text-muted" style={{ fontSize: 12 }}>
              {t('cleanup.selected')}: {formatBytes(totalSize)} · {totalFiles} {t('cleanup.files')}
            </span>
          )}
        </div>
      </div>

      {/* Category List */}
      {items.length === 0 ? (
        <div className="card">
          <p className="text-muted">
            <Icon name="check-circle" size={16} /> {t('cleanup.nothingFound')}
          </p>
        </div>
      ) : (
        items.map((item) => (
          <div className="card" key={item.category} style={{ marginBottom: 12 }}>
            <div className="toggle-row" style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
              <ToggleSwitch
                checked={selected.has(item.category)}
                onChange={() => toggleCategory(item.category)}
                disabled={cleaning}
                ariaLabel={item.description}
              />
              <span>
                <Icon name={CATEGORY_ICONS[item.category] ?? 'folder'} size={24} />
              </span>
              <div style={{ flex: 1 }}>
                <div className="mc-label-with-hint">
                  <span style={{ fontWeight: 600 }}>{item.description}</span>
                  {CATEGORY_HINTS[item.category] && (
                    <InfoHint
                      label={t(CATEGORY_HINTS[item.category] as Parameters<typeof t>[0])}
                      buttonLabel={t('hint.about', { name: item.description })}
                    />
                  )}
                </div>
                <span className="text-muted" style={{ fontSize: 12 }}>
                  {formatBytes(item.size_bytes)} · {item.file_count} {t('cleanup.files')}
                </span>
              </div>
            </div>
          </div>
        ))
      )}

      {/* Info Card */}
      <div className="card">
        <h3>
          <Icon name="info" size={16} /> {t('cleanup.about')}
        </h3>
        <p className="text-muted" style={{ fontSize: 12 }}>
          {t('cleanup.aboutDesc')}
        </p>
      </div>
    </>
  );
}
