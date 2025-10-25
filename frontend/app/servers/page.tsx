'use client';
import { useEffect, useState } from 'react';
import Spinner from '../../components/Spinner';
import Alert from '../../components/Alert';
import MetricsChart from '../../components/MetricsChart';
import ServerCard from '../../components/ServerCard';
import { useServerStore, Server } from '../../lib/store';

interface LogData {
  id: number;
  entries: { id: number; collected_at: string; log_text: string }[];
}

interface MetricsData {
  id: number;
  events: { id: number; timestamp: string; event_type: string; details?: any }[];
}

export default function Servers() {
  const { servers, fetchServers, updateStatus, loading, error: storeError } = useServerStore();
  const [error, setError] = useState<string | null>(null);
  const [logs, setLogs] = useState<LogData | null>(null);
  const [source, setSource] = useState<EventSource | null>(null);
  const [metricsSource, setMetricsSource] = useState<EventSource | null>(null);
  const [actionId, setActionId] = useState<number | null>(null);
  const [metrics, setMetrics] = useState<MetricsData | null>(null);


  const closeLogs = () => {
    if (source) {
      source.close();
      setSource(null);
    }
    if (metricsSource) {
      metricsSource.close();
      setMetricsSource(null);
    }
    setLogs(null);
    setMetrics(null);
    setError(null);
  };

  useEffect(() => {
    fetchServers();
    const es = new EventSource('/api/servers/stream');
    es.onmessage = (e) => {
      const upd = JSON.parse(e.data) as { id: number; status: string };
      updateStatus(upd.id, upd.status);
    };
    return () => {
      es.close();
      if (source) source.close();
      if (metricsSource) metricsSource.close();
    };
  }, []);

  const start = async (id: number) => {
    setActionId(id);
    setError(null);
    const res = await fetch(`/api/servers/${id}/start`, { method: 'POST', credentials: 'include' });
    setActionId(null);
    if (!res.ok) {
      setError(await res.text());
    }
  };

  const stop = async (id: number) => {
    setActionId(id);
    setError(null);
    const res = await fetch(`/api/servers/${id}/stop`, { method: 'POST', credentials: 'include' });
    setActionId(null);
    if (!res.ok) {
      setError(await res.text());
    }
  };

  const del = async (id: number) => {
    setActionId(id);
    setError(null);
    const res = await fetch(`/api/servers/${id}`, { method: 'DELETE', credentials: 'include' });
    setActionId(null);
    if (!res.ok) {
      setError(await res.text());
    }
    fetchServers();
  };

  const redeploy = async (id: number) => {
    setActionId(id);
    setError(null);
    const res = await fetch(`/api/servers/${id}/redeploy`, { method: 'POST', credentials: 'include' });
    setActionId(null);
    if (!res.ok) {
      setError(await res.text());
    }
  };

  const viewMetrics = async (id: number) => {
    const res = await fetch(`/api/servers/${id}/metrics`, {
      credentials: 'include',
    });
    if (res.ok) {
      const events = await res.json();
      setMetrics({ id, events });
    }
    if (metricsSource) metricsSource.close();
    const es = new EventSource(`/api/servers/${id}/metrics/stream`);
    es.onmessage = (e) => {
      const event = JSON.parse(e.data);
      setMetrics(prev =>
        prev && prev.id === id
          ? { ...prev, events: [event, ...prev.events] }
          : prev
      );
    };
    setMetricsSource(es);
  };

  const viewLogs = async (id: number) => {
    const res = await fetch(`/api/servers/${id}/logs/history`, {
      credentials: 'include',
    });
    if (res.ok) {
      const entries = await res.json();
      setLogs({ id, entries });
    }
    if (source) source.close();
    const es = new EventSource(`/api/servers/${id}/logs/stream`);
    es.onmessage = (e) => {
      setLogs(prev =>
        prev && prev.id === id
          ? { ...prev, entries: [{ id: Date.now(), collected_at: new Date().toISOString(), log_text: e.data }, ...prev.entries] }
          : prev
      );
    };
    setSource(es);
  };

  return (
    <div className="p-4">
      <h1 className="text-2xl mb-4">Your Servers</h1>
      {(error || storeError) && <Alert message={(error || storeError) as string} />}
      {loading && <Spinner />}
      <ul className="grid gap-4 md:grid-cols-2">
        {servers.map((s) => (
          <ServerCard
            key={s.id}
            server={s}
            actionId={actionId}
            start={start}
            stop={stop}
            del={del}
            redeploy={redeploy}
            viewLogs={viewLogs}
            viewMetrics={viewMetrics}
            closeLogs={closeLogs}
            logs={logs?.id === s.id}
            metrics={metrics?.id === s.id}
          />
        ))}
      </ul>
      {logs && (
        <pre className="mt-2 whitespace-pre-wrap bg-black text-green-300 p-2 rounded overflow-auto max-h-60 text-sm">
          {logs.entries.map((e) => `${e.collected_at}: ${e.log_text}\n`).join('')}
        </pre>
      )}
      {metrics && (
        <div className="mt-2 bg-gray-900 p-2 rounded">
          <MetricsChart events={metrics.events.slice().reverse()} />
        </div>
      )}
    </div>
  );
}
