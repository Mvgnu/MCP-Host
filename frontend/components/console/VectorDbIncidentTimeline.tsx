'use client';
import { useCallback, useMemo, useState } from 'react';
import Button from '../Button';
import Textarea from '../Textarea';
import Spinner from '../Spinner';
import type { ResolveVectorDbIncidentPayload, VectorDbIncidentRecord } from '../../lib/vectorDbs';

// key: vector-dbs-console -> incident-timeline
interface VectorDbIncidentTimelineProps {
  incidents: VectorDbIncidentRecord[];
  loading?: boolean;
  onResolve: (
    incidentId: string,
    payload: ResolveVectorDbIncidentPayload,
  ) => Promise<void> | void;
}

interface ResolutionState {
  summary: string;
  notes: string;
}

function formatDate(value: string | null | undefined): string {
  if (!value) {
    return '—';
  }
  try {
    return new Date(value).toLocaleString();
  } catch {
    return value;
  }
}

export default function VectorDbIncidentTimeline({
  incidents,
  loading = false,
  onResolve,
}: VectorDbIncidentTimelineProps) {
  const sortedIncidents = useMemo(
    () =>
      incidents
        .slice()
        .sort((a, b) => new Date(b.occurred_at).getTime() - new Date(a.occurred_at).getTime()),
    [incidents],
  );
  const [resolutionState, setResolutionState] = useState<Record<string, ResolutionState>>({});
  const [pending, setPending] = useState<Record<string, boolean>>({});
  const [errorById, setErrorById] = useState<Record<string, string>>({});

  const handleChange = useCallback((incidentId: string, field: keyof ResolutionState, value: string) => {
    setResolutionState((current) => ({
      ...current,
      [incidentId]: {
        summary: field === 'summary' ? value : current[incidentId]?.summary ?? '',
        notes: field === 'notes' ? value : current[incidentId]?.notes ?? '',
      },
    }));
  }, []);

  const handleResolve = useCallback(
    async (incidentId: string) => {
      setPending((current) => ({ ...current, [incidentId]: true }));
      setErrorById((current) => ({ ...current, [incidentId]: '' }));
      try {
        const state = resolutionState[incidentId] ?? { summary: '', notes: '' };
        const payload: ResolveVectorDbIncidentPayload = {
          resolution_summary: state.summary.trim() || undefined,
          resolution_notes: state.notes.trim() ? { note: state.notes.trim() } : undefined,
        };
        await onResolve(incidentId, payload);
        setResolutionState((current) => ({ ...current, [incidentId]: { summary: '', notes: '' } }));
      } catch (cause) {
        console.error('failed to resolve vector db incident', cause);
        setErrorById((current) => ({
          ...current,
          [incidentId]: cause instanceof Error ? cause.message : 'Failed to resolve incident',
        }));
      } finally {
        setPending((current) => ({ ...current, [incidentId]: false }));
      }
    },
    [onResolve, resolutionState],
  );

  return (
    <section className="border border-slate-200 rounded-lg bg-white shadow-sm p-4 space-y-4">
      <header className="space-y-1">
        <h2 className="text-lg font-semibold text-slate-800">Incident timeline</h2>
        <p className="text-sm text-slate-600">
          Track residency enforcement breaches and close incidents once remediation is verified.
        </p>
      </header>

      {loading ? (
        <div className="flex items-center gap-2 text-sm text-slate-600">
          <Spinner size="sm" /> Loading incidents…
        </div>
      ) : sortedIncidents.length === 0 ? (
        <p className="text-sm text-slate-500">No incidents logged yet.</p>
      ) : (
        <ol className="space-y-4">
          {sortedIncidents.map((incident) => {
            const pendingResolve = pending[incident.id] ?? false;
            const error = errorById[incident.id] ?? '';
            const state = resolutionState[incident.id] ?? { summary: '', notes: '' };
            const statusBadge = incident.resolved_at
              ? 'bg-emerald-100 text-emerald-700'
              : 'bg-amber-100 text-amber-700';
            const statusLabel = incident.resolved_at ? 'Resolved' : 'Open';

            return (
              <li key={incident.id} className="border border-slate-100 rounded-lg p-4 bg-slate-50 space-y-3">
                <div className="flex flex-col md:flex-row md:items-center md:justify-between gap-3">
                  <div className="space-y-1">
                    <h3 className="text-sm font-semibold text-slate-700">
                      {incident.incident_type} · {incident.severity}
                    </h3>
                    <p className="text-xs text-slate-500">
                      Occurred {formatDate(incident.occurred_at)}
                      {incident.attachment_id && ` · Attachment ${incident.attachment_id}`}
                    </p>
                    {incident.summary && <p className="text-xs text-slate-500">{incident.summary}</p>}
                  </div>
                  <span className={`inline-flex items-center px-2 py-1 text-xs font-semibold rounded-full ${statusBadge}`}>
                    {statusLabel}
                  </span>
                </div>
                <details className="border border-slate-200 bg-white rounded p-2 text-xs text-slate-600">
                  <summary className="cursor-pointer font-semibold text-slate-700">Notes</summary>
                  <pre className="mt-2 whitespace-pre-wrap break-words">
                    {JSON.stringify(incident.notes, null, 2)}
                  </pre>
                </details>
                <p className="text-xs text-slate-500">
                  Resolved {formatDate(incident.resolved_at ?? null)}
                </p>
                {incident.resolved_at ? null : (
                  <div className="space-y-2">
                    <Textarea
                      label="Resolution summary"
                      value={state.summary}
                      onChange={(event) => handleChange(incident.id, 'summary', event.target.value)}
                      placeholder="Remediation outcome"
                      rows={2}
                    />
                    <Textarea
                      label="Resolution notes"
                      value={state.notes}
                      onChange={(event) => handleChange(incident.id, 'notes', event.target.value)}
                      placeholder="Additional metadata captured as JSON note"
                      rows={2}
                    />
                    {error && <p className="text-xs text-red-600">{error}</p>}
                    <Button onClick={() => handleResolve(incident.id)} disabled={pendingResolve}>
                      {pendingResolve ? 'Resolving…' : 'Resolve incident'}
                    </Button>
                  </div>
                )}
              </li>
            );
          })}
        </ol>
      )}
    </section>
  );
}
