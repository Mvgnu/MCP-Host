'use client';
import { useMemo } from 'react';
import { LifecycleRunArtifact, LifecycleRunSnapshot } from '../../lib/lifecycle-console';

// key: lifecycle-console-ui -> run-progress
interface Props {
  run: LifecycleRunSnapshot;
  onSelect?: (run: LifecycleRunSnapshot) => void;
}

function formatDurationSeconds(durationSeconds?: number | null) {
  if (typeof durationSeconds !== 'number' || Number.isNaN(durationSeconds) || durationSeconds < 0) {
    return undefined;
  }
  if (durationSeconds < 60) {
    return `${Math.floor(durationSeconds)}s`;
  }
  const minutes = Math.floor(durationSeconds / 60);
  const seconds = Math.floor(durationSeconds % 60);
  if (minutes < 60) {
    return seconds === 0 ? `${minutes}m` : `${minutes}m ${seconds}s`;
  }
  const hours = Math.floor(minutes / 60);
  const remMinutes = minutes % 60;
  return remMinutes === 0 ? `${hours}h` : `${hours}h ${remMinutes}m`;
}

function fallbackDuration(startedAt: string, completedAt?: string | null) {
  const started = new Date(startedAt).getTime();
  const finished = completedAt ? new Date(completedAt).getTime() : Date.now();
  if (Number.isNaN(started) || Number.isNaN(finished)) return undefined;
  const seconds = Math.max(0, Math.floor((finished - started) / 1000));
  return formatDurationSeconds(seconds);
}

function summarizeAttempt(attempt?: number | null, retryLimit?: number | null) {
  if (typeof attempt !== 'number' && typeof retryLimit !== 'number') {
    return undefined;
  }
  const attemptPart = typeof attempt === 'number' ? String(Math.floor(attempt)) : '–';
  if (typeof retryLimit !== 'number') {
    return attemptPart;
  }
  return `${attemptPart}/${Math.floor(retryLimit)}`;
}

function summarizeArtifact(artifact: LifecycleRunArtifact) {
  const digest = artifact.manifest_digest;
  const shortDigest = digest.length > 12 ? `${digest.slice(0, 12)}…` : digest;
  const parts: string[] = [];
  if (artifact.lane) parts.push(`lane:${artifact.lane}`);
  if (artifact.stage) parts.push(`stage:${artifact.stage}`);
  if (artifact.manifest_tag) parts.push(`tag:${artifact.manifest_tag}`);
  if (artifact.registry_image) parts.push(`image:${artifact.registry_image}`);
  if (artifact.build_status) parts.push(`build:${artifact.build_status}`);
  if (typeof artifact.duration_seconds === 'number') {
    const formatted = formatDurationSeconds(artifact.duration_seconds);
    if (formatted) parts.push(`duration:${formatted}`);
  }
  return parts.length > 0 ? `${shortDigest} (${parts.join(', ')})` : shortDigest;
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
            Started {new Date(run.run.started_at).toLocaleString()} ·
            {(() => {
              const formatted =
                formatDurationSeconds(run.duration_seconds ?? null) ??
                fallbackDuration(run.run.started_at, run.run.completed_at);
              return formatted ? ` ${formatted}` : ' unknown duration';
            })()}
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
      <div className="mt-2 flex flex-wrap gap-2 text-[11px] text-slate-600">
        {summarizeAttempt(run.retry_attempt ?? null, run.retry_limit ?? null) && (
          <span className="rounded border border-slate-200 bg-slate-50 px-2 py-0.5">
            Attempt {summarizeAttempt(run.retry_attempt ?? null, run.retry_limit ?? null)}
          </span>
        )}
        {run.override_reason && (
          <span className="rounded border border-amber-200 bg-amber-50 px-2 py-0.5 text-amber-700">
            Override: {run.override_reason}
          </span>
        )}
        {(run.artifacts ?? []).map((artifact) => (
          <span
            key={`${run.run.id}-${artifact.manifest_digest}-${artifact.manifest_tag ?? ''}`}
            className="rounded border border-sky-200 bg-sky-50 px-2 py-0.5 text-sky-700"
          >
            {summarizeArtifact(artifact)}
          </span>
        ))}
      </div>
    </div>
  );
}
