import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { t } from '../hooks/useI18n';
import { Badge, Button, Card, Icon, InfoHint, Segmented, Switch } from './ui';

// ── Backend contract (hw/startup_manager.rs) ────────────────────────────────

type StartupSource =
  | 'hkcu_run'
  | 'hklm_run'
  | 'hklm_run32'
  | 'user_startup_folder'
  | 'common_startup_folder'
  | 'user_logon_task'
  | 'common_logon_task';

interface StartupItem {
  id: string;
  name: string;
  command: string;
  executable: string | null;
  publisher: string | null;
  version: string | null;
  file_exists: boolean | null;
  enabled: boolean;
  source: StartupSource;
  location: string;
  requires_admin: boolean;
  link_key: string | null;
  linked_count: number;
  toggleable: boolean;
  toggle_blocked: 'folder_entry_not_shortcut' | null;
  managed_in_settings: boolean;
  principal: string | null;
}

interface StartupSnapshot {
  items: StartupItem[];
  tasks_available: boolean;
  task_scan_error: string | null;
  tasks_skipped: number;
  cached: boolean;
}

const SOURCE_ORDER: StartupSource[] = [
  'hkcu_run',
  'hklm_run',
  'hklm_run32',
  'user_startup_folder',
  'common_startup_folder',
  'user_logon_task',
  'common_logon_task',
];

function sourceLabel(source: StartupSource): string {
  switch (source) {
    case 'hkcu_run':
      return t('startup.source.hkcuRun');
    case 'hklm_run':
      return t('startup.source.hklmRun');
    case 'hklm_run32':
      return t('startup.source.hklmRun32');
    case 'user_startup_folder':
      return t('startup.source.userStartupFolder');
    case 'common_startup_folder':
      return t('startup.source.commonStartupFolder');
    case 'user_logon_task':
      return t('startup.source.userLogonTask');
    case 'common_logon_task':
      return t('startup.source.commonLogonTask');
  }
}

function errorMessage(error: unknown): string {
  if (typeof error === 'object' && error !== null && 'message' in error) {
    return String((error as { message: unknown }).message);
  }
  return String(error);
}

function baseName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

// ── Row ──────────────────────────────────────────────────────────────────────

interface RowProps {
  item: StartupItem;
  pending: boolean;
  error?: string;
  linkedOthers: StartupItem[];
  showSource: boolean;
  onToggle: (item: StartupItem, next: boolean) => void;
}

function StartupRow({ item, pending, error, linkedOthers, showSource, onToggle }: RowProps) {
  const truncated = item.command.length > 96 ? `${item.command.slice(0, 96)}…` : item.command;
  const commandHint = t('startup.commandHint', {
    command: item.command,
    location: item.location,
  });
  const linkedHint =
    linkedOthers.length > 0
      ? t('startup.linkedHint', {
          sources: linkedOthers
            .map((other) => `${other.name} (${sourceLabel(other.source)})`)
            .join(', '),
        })
      : null;

  return (
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: '1fr auto',
        gap: 16,
        alignItems: 'center',
        padding: '12px 0',
        borderTop: '1px solid var(--hairline, var(--border))',
      }}
    >
      <div style={{ minWidth: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
          <span style={{ fontSize: 13, fontWeight: 600 }}>{item.name}</span>
          {showSource && <Badge tone="neutral">{sourceLabel(item.source)}</Badge>}
          {item.requires_admin && (
            <>
              <Badge tone="info" icon="shield">
                {t('startup.adminBadge')}
              </Badge>
              <InfoHint label={t('startup.adminHint')} />
            </>
          )}
          {item.file_exists === false && (
            <>
              <Badge tone="warning" icon="alert-triangle">
                {t('startup.missingBadge')}
              </Badge>
              <InfoHint
                label={t('startup.missingHint', {
                  path: item.executable ?? item.command,
                })}
              />
            </>
          )}
          {item.linked_count > 1 && (
            <>
              <Badge tone="accent">{t('startup.linkedBadge', { count: item.linked_count })}</Badge>
              {linkedHint && <InfoHint label={linkedHint} />}
            </>
          )}
          {item.managed_in_settings && (
            <>
              <Badge tone="neutral" icon="gear">
                {t('startup.managedBadge')}
              </Badge>
              <InfoHint label={t('startup.managedHint')} />
            </>
          )}
        </div>

        <div
          style={{
            marginTop: 4,
            fontSize: 11,
            color: 'var(--text-muted)',
            display: 'flex',
            gap: 6,
            flexWrap: 'wrap',
          }}
        >
          <span>{item.publisher ?? t('startup.publisherUnknown')}</span>
          {item.version && <span>· {item.version}</span>}
          {item.principal && <span>· {t('startup.principal', { principal: item.principal })}</span>}
        </div>

        <div
          style={{
            marginTop: 4,
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            minWidth: 0,
          }}
        >
          <code
            style={{
              fontFamily: 'var(--font-mono)',
              fontSize: 11,
              color: 'var(--text-dim)',
              whiteSpace: 'nowrap',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              minWidth: 0,
            }}
          >
            {truncated}
          </code>
          <InfoHint label={commandHint} buttonLabel={t('startup.commandHintLabel')} />
        </div>

        {!item.toggleable && item.toggle_blocked === 'folder_entry_not_shortcut' && (
          <div style={{ marginTop: 4, fontSize: 11, color: 'var(--warning)' }}>
            {t('startup.toggleBlockedFolder')}
          </div>
        )}

        {error && (
          <div style={{ marginTop: 6, fontSize: 12, color: 'var(--color-error, #e53935)' }}>
            {t('startup.toggleError')} {error}
          </div>
        )}
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: 4 }}>
        <Switch
          bare
          ariaLabel={t('startup.toggleAria', { name: item.name })}
          checked={item.enabled}
          pending={pending}
          disabled={!item.toggleable || item.managed_in_settings}
          onChange={(next) => onToggle(item, next)}
        />
        <span
          aria-live="polite"
          style={{
            fontSize: 10,
            color: pending ? 'var(--accent, var(--text-muted))' : 'var(--text-dim)',
            minHeight: 12,
            whiteSpace: 'nowrap',
          }}
        >
          {pending
            ? t('startup.applying')
            : item.enabled
              ? t('startup.enabled')
              : t('startup.disabled')}
        </span>
      </div>
    </div>
  );
}

// ── Manager ─────────────────────────────────────────────────────────────────

export default function StartupManager() {
  const [snapshot, setSnapshot] = useState<StartupSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [stateFilter, setStateFilter] = useState(0); // 0 all, 1 enabled, 2 disabled
  const [groupBy, setGroupBy] = useState(0); // 0 source, 1 program
  const [pendingIds, setPendingIds] = useState<Set<string>>(new Set());
  const [rowErrors, setRowErrors] = useState<Map<string, string>>(new Map());

  const load = useCallback(async (refresh: boolean) => {
    setLoading(true);
    setLoadError(null);
    try {
      const data = await invoke<StartupSnapshot>('get_startup_items', { refresh });
      setSnapshot(data);
    } catch (error) {
      setLoadError(errorMessage(error));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load(false);
  }, [load]);

  const items = useMemo(() => snapshot?.items ?? [], [snapshot]);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return items.filter((item) => {
      if (stateFilter === 1 && !item.enabled) return false;
      if (stateFilter === 2 && item.enabled) return false;
      if (!needle) return true;
      return (
        item.name.toLowerCase().includes(needle) ||
        item.command.toLowerCase().includes(needle) ||
        (item.publisher ?? '').toLowerCase().includes(needle)
      );
    });
  }, [items, query, stateFilter]);

  const groups = useMemo(() => {
    if (groupBy === 0) {
      return SOURCE_ORDER.map((source) => ({
        key: source,
        label: sourceLabel(source),
        items: filtered.filter((item) => item.source === source),
      })).filter((group) => group.items.length > 0);
    }

    const byProgram = new Map<string, StartupItem[]>();
    for (const item of filtered) {
      const key = item.link_key ?? `id:${item.id}`;
      const bucket = byProgram.get(key);
      if (bucket) bucket.push(item);
      else byProgram.set(key, [item]);
    }
    return [...byProgram.entries()]
      .map(([key, groupItems]) => ({
        key,
        label: baseName(groupItems[0].executable ?? groupItems[0].name),
        items: groupItems,
      }))
      .sort((a, b) => a.label.localeCompare(b.label, undefined, { sensitivity: 'base' }));
  }, [filtered, groupBy]);

  const activeCount = items.filter((item) => item.enabled).length;

  const linkedOthersFor = useCallback(
    (item: StartupItem) =>
      item.link_key
        ? items.filter((other) => other.id !== item.id && other.link_key === item.link_key)
        : [],
    [items],
  );

  const handleToggle = useCallback(async (item: StartupItem, next: boolean) => {
    setPendingIds((prev) => new Set(prev).add(item.id));
    setRowErrors((prev) => {
      const map = new Map(prev);
      map.delete(item.id);
      return map;
    });
    try {
      await invoke('set_startup_item', { id: item.id, enabled: next });
      setSnapshot((prev) =>
        prev
          ? {
              ...prev,
              items: prev.items.map((current) =>
                current.id === item.id ? { ...current, enabled: next } : current,
              ),
            }
          : prev,
      );
    } catch (error) {
      setRowErrors((prev) => new Map(prev).set(item.id, errorMessage(error)));
    } finally {
      setPendingIds((prev) => {
        const nextSet = new Set(prev);
        nextSet.delete(item.id);
        return nextSet;
      });
    }
  }, []);

  return (
    <Card
      title={t('startup.entriesTitle')}
      icon={<Icon name="rocket" size={16} />}
      meta={
        <span>
          {t('startup.activeCount', {
            active: activeCount,
            total: items.length,
          })}
        </span>
      }
      action={
        <Button
          variant="secondary"
          size="sm"
          icon="refresh"
          pending={loading}
          onClick={() => void load(true)}
        >
          {t('startup.refresh')}
        </Button>
      }
    >
      <div
        style={{
          display: 'flex',
          flexWrap: 'wrap',
          alignItems: 'end',
          gap: 12,
          marginBottom: 12,
        }}
      >
        <label style={{ flex: '1 1 220px', minWidth: 200 }}>
          <span
            style={{
              display: 'block',
              fontSize: 11,
              color: 'var(--text-muted)',
              marginBottom: 4,
            }}
          >
            {t('startup.searchLabel')}
          </span>
          <input
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t('startup.searchPlaceholder')}
            aria-label={t('startup.searchLabel')}
            style={{
              width: '100%',
              height: 32,
              padding: '0 10px',
              fontSize: 12,
              color: 'var(--text)',
              background: 'var(--surface-2)',
              border: '1px solid var(--hairline, var(--border))',
              borderRadius: 8,
            }}
          />
        </label>

        <Segmented
          label={t('startup.stateFilter')}
          value={stateFilter}
          onChange={setStateFilter}
          options={[
            { value: 0, label: t('startup.filterAll') },
            { value: 1, label: t('startup.filterEnabled') },
            { value: 2, label: t('startup.filterDisabled') },
          ]}
        />

        <Segmented
          label={t('startup.groupBy')}
          value={groupBy}
          onChange={setGroupBy}
          options={[
            { value: 0, label: t('startup.groupSource') },
            { value: 1, label: t('startup.groupProgram') },
          ]}
        />
      </div>

      {loadError && (
        <div className="alert alert-error" role="alert" style={{ marginBottom: 12 }}>
          <Icon name="alert-triangle" size={16} /> {t('startup.loadError')} {loadError}
          <Button
            variant="ghost"
            size="sm"
            icon="refresh"
            onClick={() => void load(true)}
            style={{ marginLeft: 8 }}
          >
            {t('startup.retry')}
          </Button>
        </div>
      )}

      {snapshot && !snapshot.tasks_available && (
        <div className="alert alert-warning" role="status" style={{ marginBottom: 12 }}>
          <Icon name="alert-triangle" size={16} />{' '}
          {t('startup.tasksUnavailable', {
            error: snapshot.task_scan_error ?? '—',
          })}
        </div>
      )}

      {snapshot && snapshot.tasks_skipped > 0 && (
        <p className="text-muted" style={{ fontSize: 11, margin: '0 0 8px' }}>
          {t('startup.tasksSkipped', { count: snapshot.tasks_skipped })}
        </p>
      )}

      {loading && !snapshot ? (
        <p className="text-muted">{t('startup.loading')}</p>
      ) : items.length === 0 ? (
        <div style={{ padding: '24px 0', textAlign: 'center' }}>
          <Icon name="rocket" size={24} />
          <h3 style={{ margin: '8px 0 4px', fontSize: 14 }}>{t('startup.emptyTitle')}</h3>
          <p className="text-muted" style={{ margin: 0, fontSize: 12 }}>
            {t('startup.emptyBody')}
          </p>
        </div>
      ) : filtered.length === 0 ? (
        <div style={{ padding: '24px 0', textAlign: 'center' }}>
          <p className="text-muted" style={{ margin: '0 0 8px', fontSize: 12 }}>
            {t('startup.searchEmpty', { query: query.trim() })}
          </p>
          <Button variant="ghost" size="sm" onClick={() => setQuery('')}>
            {t('startup.clearSearch')}
          </Button>
        </div>
      ) : (
        groups.map((group) => (
          <section key={group.key} style={{ marginTop: 8 }}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                padding: '8px 0 4px',
              }}
            >
              <h3 style={{ margin: 0, fontSize: 12, fontWeight: 600 }}>{group.label}</h3>
              <Badge tone="neutral">{group.items.length}</Badge>
            </div>
            {group.items.map((item) => (
              <StartupRow
                key={item.id}
                item={item}
                pending={pendingIds.has(item.id)}
                error={rowErrors.get(item.id)}
                linkedOthers={linkedOthersFor(item)}
                showSource={groupBy === 1}
                onToggle={(target, next) => void handleToggle(target, next)}
              />
            ))}
          </section>
        ))
      )}
    </Card>
  );
}
