'use client';
import clsx from 'clsx';

/* musikconnect:
   purpose: Render a textual timeline of registry metrics with friendly labels
   inputs: MetricEvent list ordered newest-first
   outputs: Accessible list of formatted metric entries for dashboards
   status: experimental
   depends_on: clsx
   related_docs: ../../progress.md
*/

interface MetricEvent {
  id: number;
  timestamp: string;
  event_type: string;
  details?: Record<string, unknown> | null;
}

const FRIENDLY_LABELS: Record<string, string> = {
  tag_started: 'Tagging started',
  tag_succeeded: 'Tagging completed',
  push_started: 'Push started',
  push_retry: 'Push retry scheduled',
  push_succeeded: 'Push completed',
  push_failed: 'Push failed',
};

const BADGE_VARIANTS: Record<string, string> = {
  tag_started: 'bg-sky-600/30 text-sky-200 border-sky-500/40',
  tag_succeeded: 'bg-emerald-600/30 text-emerald-200 border-emerald-500/40',
  push_started: 'bg-indigo-600/30 text-indigo-200 border-indigo-500/40',
  push_retry: 'bg-amber-600/30 text-amber-200 border-amber-500/40',
  push_succeeded: 'bg-emerald-600/30 text-emerald-200 border-emerald-500/40',
  push_failed: 'bg-rose-600/30 text-rose-200 border-rose-500/40',
};

function formatDetails(event: MetricEvent): string[] {
  const details = event.details ?? {};
  switch (event.event_type) {
    case 'tag_started':
      return [
        `Repository: ${details.registry_endpoint ?? 'unknown'}`,
        `Tag: ${details.tag ?? 'latest'}`,
      ];
    case 'tag_succeeded':
      return [
        `Repository: ${details.registry_endpoint ?? 'unknown'}`,
        `Tag: ${details.tag ?? 'latest'}`,
      ];
    case 'push_started':
      return [
        `Attempt ${details.attempt ?? '1'} of ${details.retry_limit ?? '1'}`,
        `Repository: ${details.registry_endpoint ?? 'unknown'}`,
      ];
    case 'push_retry':
      return [
        `Retry ${details.attempt ?? '?'} of ${details.retry_limit ?? '?'}`,
        `Repository: ${details.registry_endpoint ?? 'unknown'}`,
        details.error ? `Reason: ${details.error}` : 'Reason: unknown',
      ];
    case 'push_succeeded':
      return [
        `Completed on attempt ${details.attempt ?? '1'}`,
        `Repository: ${details.registry_endpoint ?? 'unknown'}`,
      ];
    case 'push_failed':
      return [
        `Repository: ${details.registry_endpoint ?? 'unknown'}`,
        `Failure kind: ${details.error_kind ?? 'unknown'}`,
        details.error ? `Error: ${details.error}` : 'Error: unknown',
        `Attempt ${details.attempt ?? '?'} of ${details.retry_limit ?? '?'}`,
        `Auth expired: ${details.auth_expired === true ? 'yes' : 'no'}`,
      ];
    default:
      return Object.entries(details).map(([key, value]) => `${key}: ${String(value)}`);
  }
}

export default function MetricsEventList({ events }: { events: MetricEvent[] }) {
  if (!events.length) {
    return (
      <p className="text-sm text-gray-300" data-testid="metrics-empty">
        No registry telemetry has been recorded yet.
      </p>
    );
  }

  return (
    <ul className="space-y-2" data-testid="metrics-event-list">
      {events.map((event) => {
        const label = FRIENDLY_LABELS[event.event_type] ?? event.event_type;
        const badgeClass = BADGE_VARIANTS[event.event_type] ?? 'bg-slate-600/30 text-slate-200 border-slate-500/40';
        const detailLines = formatDetails(event);
        const timestamp = new Date(event.timestamp).toLocaleString();

        return (
          <li
            key={event.id}
            className="rounded border border-slate-600/40 bg-slate-900/60 p-3 text-sm text-slate-100"
          >
            <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-2">
              <span className={clsx('inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium border', badgeClass)}>
                {label}
              </span>
              <time className="text-xs text-slate-400" dateTime={event.timestamp}>
                {timestamp}
              </time>
            </div>
            <ul className="mt-2 space-y-1 text-xs text-slate-300">
              {detailLines.map((line, idx) => (
                <li key={idx}>{line}</li>
              ))}
            </ul>
          </li>
        );
      })}
    </ul>
  );
}
