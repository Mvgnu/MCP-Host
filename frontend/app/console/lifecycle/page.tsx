'use client';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import Alert from '../../../components/Alert';
import Spinner from '../../../components/Spinner';
import {
  LifecycleConsoleEventEnvelope,
  LifecycleConsolePage,
  LifecycleRunDelta,
  LifecycleWorkspaceSnapshot,
} from '../../../lib/lifecycle-console';
import {
  LifecycleFilterBar,
  LifecycleRunDrilldownModal,
  LifecycleTimeline,
  LifecycleVerdictCard,
} from '../../../components/console';
import type { LifecycleFilters } from '../../../components/console/LifecycleFilterBar';

// key: lifecycle-console-ui -> page-shell
const PAGE_LIMIT = 10;
const RUN_LIMIT = 5;
const POLL_FALLBACK_MS = 15000;
const RECONNECT_DELAY_MS = 8000;
const DEFAULT_FILTERS: LifecycleFilters = {
  workspaceSearch: '',
  promotionLane: '',
  severity: '',
};
const STORAGE_STATE_KEY = 'lifecycle-console.state.v2';
const STORAGE_FILTER_KEY = 'lifecycle-console.filters.v2';
const STORAGE_CURSOR_KEY = 'lifecycle-console.cursor.v2';

type WorkspaceMap = Record<number, LifecycleWorkspaceSnapshot>;
type RunDeltaMap = Record<number, LifecycleRunDelta>;

interface PersistedState {
  workspaces: WorkspaceMap;
  order: number[];
  runDeltas: RunDeltaMap;
  lastCursor: number | null;
  lastEmittedAt: string | null;
}

interface SelectedRunRef {
  workspaceId: number;
  runId: number;
}

export default function LifecycleConsolePage() {
  const [workspaces, setWorkspaces] = useState<WorkspaceMap>({});
  const [order, setOrder] = useState<number[]>([]);
  const [runDeltas, setRunDeltas] = useState<RunDeltaMap>({});
  const [filters, setFilters] = useState<LifecycleFilters>(DEFAULT_FILTERS);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [lastEmittedAt, setLastEmittedAt] = useState<string | null>(null);
  const [selectedRun, setSelectedRun] = useState<SelectedRunRef | null>(null);
  const [isOffline, setIsOffline] = useState<boolean>(typeof window !== 'undefined' ? !navigator.onLine : false);
  const sourceRef = useRef<EventSource | null>(null);
  const pollTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastCursorRef = useRef<number | null>(null);
  const bootstrapRef = useRef<boolean>(false);

  const persistCursor = useCallback((cursor: number | null) => {
    lastCursorRef.current = cursor;
    if (typeof window === 'undefined') return;
    if (cursor === null || cursor === undefined) {
      window.localStorage.removeItem(STORAGE_CURSOR_KEY);
      return;
    }
    window.localStorage.setItem(STORAGE_CURSOR_KEY, String(cursor));
  }, []);

  const persistFilters = useCallback((next: LifecycleFilters) => {
    if (typeof window === 'undefined') return;
    window.localStorage.setItem(STORAGE_FILTER_KEY, JSON.stringify(next));
  }, []);

  const persistState = useCallback(
    (state: PersistedState) => {
      if (typeof window === 'undefined') return;
      window.localStorage.setItem(STORAGE_STATE_KEY, JSON.stringify(state));
    },
    [],
  );

  const applyDelta = useCallback((delta: LifecycleConsoleEventEnvelope['delta']) => {
    if (!delta) return;
    setRunDeltas((current) => {
      const next: RunDeltaMap = { ...current };
      delta.workspaces.forEach((workspaceDelta) => {
        workspaceDelta.run_deltas.forEach((runDelta) => {
          next[runDelta.run_id] = runDelta;
        });
        workspaceDelta.removed_run_ids.forEach((runId) => {
          delete next[runId];
        });
      });
      return next;
    });
    setWorkspaces((current) => {
      const next: WorkspaceMap = { ...current };
      delta.workspaces.forEach((workspaceDelta) => {
        const existing = next[workspaceDelta.workspace_id];
        if (!existing) return;
        if (workspaceDelta.removed_run_ids.length > 0) {
          const remaining = existing.recent_runs.filter(
            (run) => !workspaceDelta.removed_run_ids.includes(run.run.id),
          );
          next[workspaceDelta.workspace_id] = {
            ...existing,
            recent_runs: remaining,
          };
        }
      });
      return next;
    });
  }, []);

  const applyPage = useCallback(
    (page: LifecycleConsolePage) => {
      if (page.workspaces.length === 0) {
        if (typeof page.next_cursor === 'number') {
          persistCursor(page.next_cursor);
        }
        return;
      }
      setWorkspaces((current) => {
        const next: WorkspaceMap = { ...current };
        page.workspaces.forEach((snapshot) => {
          next[snapshot.workspace.id] = snapshot;
        });
        return next;
      });
      setOrder((current) => {
        const seen = new Set(current);
        const merged = [...current];
        page.workspaces.forEach((snapshot) => {
          if (!seen.has(snapshot.workspace.id)) {
            merged.push(snapshot.workspace.id);
            seen.add(snapshot.workspace.id);
          }
        });
        merged.sort((a, b) => b - a);
        return merged;
      });
      const lastWorkspace = page.workspaces[page.workspaces.length - 1];
      persistCursor(lastWorkspace.workspace.id);
    },
    [persistCursor],
  );

  const stopPolling = useCallback(() => {
    if (pollTimerRef.current) {
      clearInterval(pollTimerRef.current);
      pollTimerRef.current = null;
    }
  }, []);

  const clearReconnectTimer = useCallback(() => {
    if (reconnectTimerRef.current) {
      clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    }
  }, []);

  const appendFilters = useCallback(
    (params: URLSearchParams) => {
      if (filters.workspaceSearch) {
        params.set('workspace_search', filters.workspaceSearch);
      }
      if (filters.promotionLane) {
        params.set('promotion_lane', filters.promotionLane);
      }
      if (filters.severity) {
        params.set('severity', filters.severity);
      }
    },
    [filters],
  );

  const fetchPage = useCallback(
    async (cursor?: number) => {
      try {
        const params = new URLSearchParams();
        params.set('limit', String(PAGE_LIMIT));
        params.set('run_limit', String(RUN_LIMIT));
        if (cursor) {
          params.set('cursor', String(cursor));
        }
        appendFilters(params);
        const response = await fetch(`/api/console/lifecycle?${params.toString()}`, {
          credentials: 'include',
        });
        if (!response.ok) {
          throw new Error(`Failed to fetch lifecycle page (${response.status})`);
        }
        const page = (await response.json()) as LifecycleConsolePage;
        applyPage(page);
        setError(null);
        if (page.workspaces.length === 0 && typeof page.next_cursor === 'number') {
          persistCursor(page.next_cursor);
        }
        setLastEmittedAt(new Date().toISOString());
      } catch (err) {
        console.error(err);
        setError(err instanceof Error ? err.message : 'Failed to load lifecycle console');
      } finally {
        setLoading(false);
      }
    },
    [appendFilters, applyPage, persistCursor],
  );

  const connectStream = useCallback(
    (cursor?: number) => {
      clearReconnectTimer();
      if (isOffline) {
        return;
      }
      if (sourceRef.current) {
        sourceRef.current.close();
        sourceRef.current = null;
      }
      const params = new URLSearchParams();
      params.set('limit', String(PAGE_LIMIT));
      params.set('run_limit', String(RUN_LIMIT));
      if (cursor) {
        params.set('cursor', String(cursor));
      }
      appendFilters(params);
      const es = new EventSource(`/api/console/lifecycle/stream?${params.toString()}`);
      es.addEventListener('lifecycle-snapshot', (event) => {
        try {
          const envelope = JSON.parse((event as MessageEvent).data) as LifecycleConsoleEventEnvelope;
          applyDelta(envelope.delta ?? null);
          if (envelope.page) {
            applyPage(envelope.page);
            stopPolling();
            setError(null);
          }
          if (typeof envelope.cursor === 'number') {
            persistCursor(envelope.cursor);
          }
          setLastEmittedAt(envelope.emitted_at);
        } catch (err) {
          console.error('Failed to parse lifecycle snapshot', err);
        }
      });
      es.addEventListener('lifecycle-heartbeat', (event) => {
        try {
          const envelope = JSON.parse((event as MessageEvent).data) as LifecycleConsoleEventEnvelope;
          setLastEmittedAt(envelope.emitted_at);
        } catch (err) {
          console.error('Failed to parse lifecycle heartbeat', err);
        }
      });
      es.addEventListener('lifecycle-error', (event) => {
        try {
          const envelope = JSON.parse((event as MessageEvent).data) as LifecycleConsoleEventEnvelope;
          setError(envelope.error ?? 'Lifecycle stream error');
        } catch (err) {
          setError('Lifecycle stream error');
        }
      });
      es.onerror = () => {
        setError('Lifecycle stream disconnected. Attempting to reconnect...');
        es.close();
        sourceRef.current = null;
        if (!pollTimerRef.current) {
          pollTimerRef.current = setInterval(async () => {
            await fetchPage(lastCursorRef.current ?? undefined);
          }, POLL_FALLBACK_MS);
        }
        if (!reconnectTimerRef.current) {
          reconnectTimerRef.current = setTimeout(() => {
            reconnectTimerRef.current = null;
            connectStream(lastCursorRef.current ?? undefined);
          }, RECONNECT_DELAY_MS);
        }
      };
      sourceRef.current = es;
    },
    [appendFilters, applyDelta, applyPage, clearReconnectTimer, fetchPage, isOffline, persistCursor, stopPolling],
  );

  useEffect(() => {
    if (typeof window === 'undefined' || bootstrapRef.current) {
      return;
    }
    try {
      const storedFilters = window.localStorage.getItem(STORAGE_FILTER_KEY);
      if (storedFilters) {
        const parsed = JSON.parse(storedFilters) as Partial<LifecycleFilters>;
        setFilters((current) => ({ ...current, ...parsed }));
      }
      const storedState = window.localStorage.getItem(STORAGE_STATE_KEY);
      if (storedState) {
        const parsed = JSON.parse(storedState) as PersistedState;
        if (parsed.workspaces) {
          setWorkspaces(parsed.workspaces);
        }
        if (parsed.order) {
          setOrder(parsed.order);
        }
        if (parsed.runDeltas) {
          setRunDeltas(parsed.runDeltas);
        }
        if (typeof parsed.lastCursor === 'number') {
          persistCursor(parsed.lastCursor);
        }
        if (parsed.lastEmittedAt) {
          setLastEmittedAt(parsed.lastEmittedAt);
        }
        if (parsed.workspaces && Object.keys(parsed.workspaces).length > 0) {
          setLoading(false);
        }
      }
      const storedCursor = window.localStorage.getItem(STORAGE_CURSOR_KEY);
      if (storedCursor) {
        const parsedCursor = Number(storedCursor);
        if (!Number.isNaN(parsedCursor)) {
          persistCursor(parsedCursor);
        }
      }
    } catch (err) {
      console.warn('Failed to hydrate lifecycle console cache', err);
    } finally {
      bootstrapRef.current = true;
    }
  }, [persistCursor]);

  useEffect(() => {
    const handleOnline = () => {
      setIsOffline(false);
      setError(null);
      if (sourceRef.current === null && bootstrapRef.current) {
        connectStream(lastCursorRef.current ?? undefined);
      }
    };
    const handleOffline = () => {
      setIsOffline(true);
      setError('Offline mode: displaying cached lifecycle data');
      if (sourceRef.current) {
        sourceRef.current.close();
        sourceRef.current = null;
      }
      stopPolling();
      clearReconnectTimer();
    };
    window.addEventListener('online', handleOnline);
    window.addEventListener('offline', handleOffline);
    return () => {
      window.removeEventListener('online', handleOnline);
      window.removeEventListener('offline', handleOffline);
    };
  }, [clearReconnectTimer, connectStream, stopPolling]);

  useEffect(() => {
    if (!bootstrapRef.current) {
      return;
    }
    stopPolling();
    clearReconnectTimer();
    if (sourceRef.current) {
      sourceRef.current.close();
      sourceRef.current = null;
    }
    setWorkspaces({});
    setOrder([]);
    setRunDeltas({});
    setLoading(true);
    persistCursor(null);
    fetchPage().then(() => {
      if (!isOffline) {
        connectStream(lastCursorRef.current ?? undefined);
      }
    });
  }, [clearReconnectTimer, connectStream, fetchPage, isOffline, persistCursor, stopPolling, filters]);

  useEffect(() => {
    persistFilters(filters);
  }, [filters, persistFilters]);

  useEffect(() => {
    persistState({
      workspaces,
      order,
      runDeltas,
      lastCursor: lastCursorRef.current,
      lastEmittedAt,
    });
  }, [workspaces, order, runDeltas, lastEmittedAt, persistState]);

  useEffect(() => {
    return () => {
      if (sourceRef.current) {
        sourceRef.current.close();
      }
      stopPolling();
      clearReconnectTimer();
    };
  }, [clearReconnectTimer, stopPolling]);

  const orderedWorkspaces = useMemo(() => {
    return order
      .map((id) => workspaces[id])
      .filter(Boolean)
      .sort((a, b) => {
        const left = new Date(a!.workspace.updated_at).getTime();
        const right = new Date(b!.workspace.updated_at).getTime();
        return right - left;
      });
  }, [order, workspaces]);

  const selectedWorkspace = selectedRun ? workspaces[selectedRun.workspaceId] : undefined;
  const selectedRunSnapshot = selectedWorkspace?.recent_runs.find((run) => run.run.id === selectedRun?.runId);
  const selectedRunDelta = selectedRunSnapshot ? runDeltas[selectedRunSnapshot.run.id] : undefined;

  return (
    <div className="space-y-6">
      <header className="space-y-1">
        <h1 className="text-2xl font-semibold">Lifecycle Console</h1>
        <p className="text-sm text-slate-600">
          Unified remediation, trust, intelligence, and marketplace telemetry with live streaming updates.
        </p>
        {lastEmittedAt && (
          <p className="text-xs text-slate-500">Last update: {new Date(lastEmittedAt).toLocaleString()}</p>
        )}
        {isOffline && <p className="text-xs text-amber-600">Offline mode enabled – serving cached lifecycle state.</p>}
      </header>
      <LifecycleFilterBar
        filters={filters}
        onChange={(next) => setFilters(next)}
        onReset={() => setFilters(DEFAULT_FILTERS)}
      />
      {error && <Alert message={error} />}
      {loading && <Spinner />}
      <section className="space-y-4">
        {orderedWorkspaces.map((snapshot) => (
          <article key={snapshot.workspace.id} className="space-y-3 border border-slate-200 rounded-lg p-4 bg-white shadow-sm">
            <div className="flex items-center justify-between flex-wrap gap-2">
              <div>
                <h2 className="text-lg font-semibold">{snapshot.workspace.display_name}</h2>
                <p className="text-sm text-slate-600">State: {snapshot.workspace.lifecycle_state}</p>
              </div>
              <LifecycleVerdictCard revision={snapshot.active_revision} />
            </div>
            <LifecycleTimeline
              runs={snapshot.recent_runs}
              onRunSelect={(run) => setSelectedRun({ workspaceId: snapshot.workspace.id, runId: run.run.id })}
              runDeltas={runDeltas}
            />
          </article>
        ))}
        {!loading && orderedWorkspaces.length === 0 && (
          <p className="text-sm text-slate-500">No lifecycle workspaces found for this organization.</p>
        )}
      </section>
      {selectedWorkspace && selectedRunSnapshot && (
        <LifecycleRunDrilldownModal
          workspace={selectedWorkspace}
          run={selectedRunSnapshot}
          delta={selectedRunDelta}
          onClose={() => setSelectedRun(null)}
        />
      )}
    </div>
  );
}
