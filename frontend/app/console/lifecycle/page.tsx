'use client';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import Alert from '../../../components/Alert';
import Spinner from '../../../components/Spinner';
import {
  LifecycleConsoleEventEnvelope,
  LifecycleConsolePage,
  LifecycleWorkspaceSnapshot,
} from '../../../lib/lifecycle-console';
import { LifecycleTimeline, LifecycleVerdictCard } from '../../../components/console';

// key: lifecycle-console-ui -> page-shell
const PAGE_LIMIT = 10;
const RUN_LIMIT = 5;
const POLL_FALLBACK_MS = 15000;
const RECONNECT_DELAY_MS = 8000;

type WorkspaceMap = Record<number, LifecycleWorkspaceSnapshot>;

export default function LifecycleConsolePage() {
  const [workspaces, setWorkspaces] = useState<WorkspaceMap>({});
  const [order, setOrder] = useState<number[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [lastEmittedAt, setLastEmittedAt] = useState<string | null>(null);
  const sourceRef = useRef<EventSource | null>(null);
  const pollTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastCursorRef = useRef<number | null>(null);

  const applyPage = useCallback((page: LifecycleConsolePage) => {
    if (page.workspaces.length === 0) {
      return;
    }
    setWorkspaces((current) => {
      const next = { ...current };
      page.workspaces.forEach((snapshot) => {
        next[snapshot.workspace.id] = snapshot;
      });
      return next;
    });
    setOrder((current) => {
      const seen = new Set(current);
      const combined = [...current];
      page.workspaces.forEach((snapshot) => {
        if (!seen.has(snapshot.workspace.id)) {
          combined.push(snapshot.workspace.id);
          seen.add(snapshot.workspace.id);
        }
      });
      combined.sort((a, b) => b - a);
      return combined;
    });
    const lastWorkspace = page.workspaces[page.workspaces.length - 1];
    lastCursorRef.current = lastWorkspace.workspace.id;
  }, []);

  const fetchPage = useCallback(
    async (cursor?: number) => {
      try {
        const params = new URLSearchParams();
        params.set('limit', String(PAGE_LIMIT));
        params.set('run_limit', String(RUN_LIMIT));
        if (cursor) {
          params.set('cursor', String(cursor));
        }
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
          lastCursorRef.current = page.next_cursor;
        }
        setLastEmittedAt(new Date().toISOString());
      } catch (err) {
        console.error(err);
        setError(err instanceof Error ? err.message : 'Failed to load lifecycle console');
      } finally {
        setLoading(false);
      }
    },
    [applyPage],
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

  const connectStream = useCallback((cursor?: number) => {
    clearReconnectTimer();
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
    const es = new EventSource(`/api/console/lifecycle/stream?${params.toString()}`);
    es.addEventListener('lifecycle-snapshot', (event) => {
      try {
        const envelope = JSON.parse((event as MessageEvent).data) as LifecycleConsoleEventEnvelope;
        if (envelope.page) {
          applyPage(envelope.page);
          stopPolling();
          setError(null);
        }
        if (typeof envelope.cursor === 'number') {
          lastCursorRef.current = envelope.cursor;
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
  }, [applyPage, clearReconnectTimer, fetchPage, stopPolling]);

  useEffect(() => {
    fetchPage().then(() => {
      connectStream(lastCursorRef.current ?? undefined);
    });
    return () => {
      if (sourceRef.current) {
        sourceRef.current.close();
      }
      stopPolling();
      clearReconnectTimer();
    };
  }, [clearReconnectTimer, connectStream, fetchPage, stopPolling]);

  const orderedWorkspaces = useMemo(() => {
    return order
      .map((id) => workspaces[id])
      .filter(Boolean)
      .sort((a, b) => {
        const left = new Date(a.workspace.updated_at).getTime();
        const right = new Date(b.workspace.updated_at).getTime();
        return right - left;
      });
  }, [order, workspaces]);

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
      </header>
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
            <LifecycleTimeline runs={snapshot.recent_runs} />
          </article>
        ))}
        {!loading && orderedWorkspaces.length === 0 && (
          <p className="text-sm text-slate-500">No lifecycle workspaces found for this organization.</p>
        )}
      </section>
    </div>
  );
}
