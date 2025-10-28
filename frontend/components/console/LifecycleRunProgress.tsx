'use client';
import { useMemo } from 'react';
import { LifecycleRunSnapshot } from '../../lib/lifecycle-console';

// key: lifecycle-console-ui -> run-progress
interface Props {
  run: LifecycleRunSnapshot;
  onSelect?: (run: LifecycleRunSnapshot) => void;
}

function formatDuration(startedAt: string, completedAt?: string | null) {
  const started = new Date(startedAt).getTime();
  const finished = completedAt ? new Date(completedAt).getTime() : Date.now();
  if (Number.isNaN(started) || Number.isNaN(finished)) return 'unknown duration';
  const delta = Math.max(0, finished - started);
  const minutes = Math.floor(delta / 60000);
  const seconds = Math.floor((delta % 60000) / 1000);
  if (minutes === 0) return `${seconds}s`;
  return `${minutes}m ${seconds}s`;
}

export default function LifecycleRunProgress({ run, onSelect }: Props) {
  const statusBadge = useMemo(() => {
    const status = run.run.status;
    const base = 'px-2 py-1 rounded text-xs font-semibold';
    switch (status) {
      case 'succeeded':
        return `${base} bg-emerald-100 text-emerald-700`;
      case 'failed':
        return `${base} bg-rose-100 text-rose-700`;
      case 'cancelled':
        return `${base} bg-amber-100 text-amber-700`;
      default:
        return `${base} bg-sky-100 text-sky-700`;
    }
  }, [run.run.status]);

  return (
    <div className="border border-slate-200 rounded p-3 bg-white shadow-sm">
      <div className="flex items-center justify-between gap-2">
        <div>
          <p className="text-sm font-semibold">Run #{run.run.id}</p>
          <p className="text-xs text-slate-500">
            Started {new Date(run.run.started_at).toLocaleString()} · {formatDuration(run.run.started_at, run.run.completed_at)}
          </p>
        </div>
        <span className={statusBadge}>{run.run.status}</span>
        {onSelect && (
          <button
            type="button"
            className="px-2 py-1 text-xs font-medium rounded border border-slate-200 text-slate-600 hover:bg-slate-100 focus:outline-none focus:ring-2 focus:ring-sky-500"
            onClick={() => onSelect(run)}
          >
            View details
          </button>
        )}
      </div>
      {run.run.failure_reason && (
        <p className="mt-2 text-xs text-rose-600">
          Failure: {run.run.failure_reason}
        </p>
      )}
      {run.run.approval_required && (
        <p className="mt-2 text-xs text-amber-600">
          Awaiting approval – state: {run.run.approval_state ?? 'pending'}
        </p>
      )}
    </div>
  );
}
