'use client';
import { useEffect } from 'react';
import {
  LifecycleFieldChange,
  LifecycleRunDelta,
  LifecycleRunSnapshot,
  LifecycleWorkspaceSnapshot,
} from '../../lib/lifecycle-console';
import LifecycleTrustOverlay from './LifecycleTrustOverlay';

// key: lifecycle-console-ui -> drilldown-modal
interface Props {
  workspace: LifecycleWorkspaceSnapshot;
  run: LifecycleRunSnapshot;
  delta?: LifecycleRunDelta;
  onClose: () => void;
}

function ChangeList({ title, changes }: { title: string; changes: LifecycleFieldChange[] }) {
  if (changes.length === 0) {
    return null;
  }
  return (
    <section className="space-y-1">
      <h3 className="text-xs font-semibold uppercase tracking-wide text-slate-500">{title}</h3>
      <ul className="space-y-1 text-xs">
        {changes.map((change, index) => (
          <li key={`${change.field}-${index}`} className="flex items-start gap-1">
            <span className="font-medium text-slate-600">{change.field}</span>
            <span className="text-slate-500">{change.previous ?? '—'} → {change.current ?? '—'}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

export default function LifecycleRunDrilldownModal({ workspace, run, delta, onClose }: Props) {
  useEffect(() => {
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKey);
    return () => window.removeEventListener('keydown', handleKey);
  }, [onClose]);

  const timeline = [...workspace.recent_runs].sort((a, b) => {
    const left = new Date(a.run.started_at).getTime();
    const right = new Date(b.run.started_at).getTime();
    return right - left;
  });

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/60 p-4">
      <div className="w-full max-w-4xl rounded-lg bg-white shadow-xl overflow-hidden">
        <header className="flex items-center justify-between border-b border-slate-200 px-6 py-4">
          <div>
            <p className="text-xs uppercase tracking-wide text-slate-500">Remediation run</p>
            <h2 className="text-lg font-semibold text-slate-800">
              {workspace.workspace.display_name} · Run #{run.run.id}
            </h2>
            <p className="text-xs text-slate-500">Started {new Date(run.run.started_at).toLocaleString()}</p>
          </div>
          <button
            type="button"
            className="rounded px-3 py-1 text-sm font-medium text-slate-600 border border-slate-200 hover:bg-slate-100 focus:outline-none focus:ring-2 focus:ring-sky-500"
            onClick={onClose}
          >
            Close
          </button>
        </header>
        <div className="grid gap-6 p-6 lg:grid-cols-[2fr_1fr]">
          <div className="space-y-4">
            <section className="space-y-1">
              <h3 className="text-sm font-semibold text-slate-700">Run context</h3>
              <p className="text-xs text-slate-600">
                Status: <span className="font-medium text-slate-800">{run.run.status}</span>
              </p>
              {typeof run.duration_ms === 'number' && (
                <p className="text-xs text-slate-600">
                  Duration (ms): <span className="font-medium">{Math.max(0, Math.floor(run.duration_ms))}</span>
                </p>
              )}
              {typeof run.duration_seconds === 'number' && (
                <p className="text-xs text-slate-600">
                  Duration: <span className="font-medium">{formatDuration(run.duration_seconds)}</span>
                </p>
              )}
              {run.execution_window && (
                <p className="text-xs text-slate-600">
                  Execution window:{' '}
                  <span className="font-medium">
                    {new Date(run.execution_window.started_at).toLocaleString()} →{' '}
                    {run.execution_window.completed_at
                      ? new Date(run.execution_window.completed_at).toLocaleString()
                      : 'in-progress'}
                  </span>
                </p>
              )}
              {(typeof run.retry_attempt === 'number' || typeof run.retry_limit === 'number') && (
                <p className="text-xs text-slate-600">
                  Attempt:{' '}
                  <span className="font-medium">
                    {typeof run.retry_attempt === 'number' ? Math.floor(run.retry_attempt) : '–'}
                    {typeof run.retry_limit === 'number'
                      ? `/${Math.floor(run.retry_limit)}`
                      : ''}
                  </span>
                </p>
              )}
              {typeof run.retry_count === 'number' && (
                <p className="text-xs text-slate-600">
                  Total attempts recorded: <span className="font-medium">{Math.floor(run.retry_count)}</span>
                </p>
              )}
              {(run.retry_ledger ?? []).length > 0 && (
                <div className="space-y-1">
                  <p className="text-xs font-semibold text-slate-600">Retry ledger</p>
                  <ul className="space-y-1 text-[11px] text-slate-600">
                    {(run.retry_ledger ?? []).map((entry, index) => (
                      <li key={`${entry.attempt}-${index}`}>
                        <span className="font-medium">Attempt {Math.floor(entry.attempt)}</span>
                        {entry.status && <span className="ml-1">· {entry.status}</span>}
                        {entry.reason && <span className="ml-1 text-slate-500">({entry.reason})</span>}
                        {entry.observed_at && (
                          <span className="ml-1 text-slate-500">
                            @ {new Date(entry.observed_at).toLocaleString()}
                          </span>
                        )}
                      </li>
                    ))}
                  </ul>
                </div>
              )}
              {run.override_reason && (
                <p className="text-xs text-amber-600">Override reason: {run.override_reason}</p>
              )}
              {run.manual_override && (
                <p className="text-xs text-amber-600">
                  Manual override actor:{' '}
                  <span className="font-medium">
                    {run.manual_override.actor_email ??
                      (typeof run.manual_override.actor_id === 'number'
                        ? `user#${run.manual_override.actor_id}`
                        : 'unknown')}
                  </span>
                </p>
              )}
              {run.promotion_verdict && (
                <p className="text-xs text-emerald-600">
                  Promotion verdict #{run.promotion_verdict.verdict_id}:{' '}
                  {run.promotion_verdict.allowed === false
                    ? 'blocked'
                    : run.promotion_verdict.allowed === true
                    ? 'allowed'
                    : 'pending'}
                  {run.promotion_verdict.stage && ` · stage:${run.promotion_verdict.stage}`}
                  {run.promotion_verdict.track_name && ` · track:${run.promotion_verdict.track_name}`}
                  {run.promotion_verdict.track_tier && ` · tier:${run.promotion_verdict.track_tier}`}
                </p>
              )}
              {run.run.failure_reason && <p className="text-xs text-rose-600">Failure: {run.run.failure_reason}</p>}
              {run.run.last_error && <p className="text-xs text-rose-500">Last error: {run.run.last_error}</p>}
              {run.run.approval_required && (
                <p className="text-xs text-amber-600">
                  Awaiting approval – state: {run.run.approval_state ?? 'pending'}
                </p>
              )}
              {(run.artifacts ?? []).length > 0 && (
                <div className="space-y-1">
                  <p className="text-xs font-semibold text-slate-600">Artifacts</p>
                  <ul className="space-y-1 text-[11px] text-slate-600">
                    {(run.artifacts ?? []).map((artifact) => (
                      <li key={`${artifact.manifest_digest}-${artifact.manifest_tag ?? ''}`}>
                        <span className="font-medium">{artifact.manifest_digest}</span>
                        <span className="ml-1 text-slate-500">
                          {renderArtifactDetails(artifact)}
                        </span>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
              {(run.artifact_fingerprints ?? []).length > 0 && (
                <div className="space-y-1">
                  <p className="text-xs font-semibold text-slate-600">Artifact fingerprints</p>
                  <ul className="space-y-1 text-[11px] text-slate-600">
                    {(run.artifact_fingerprints ?? []).map((fingerprint) => (
                      <li key={`${fingerprint.manifest_digest}-${fingerprint.fingerprint}`}>
                        {fingerprint.manifest_digest} → {fingerprint.fingerprint}
                      </li>
                    ))}
                  </ul>
                </div>
              )}
              {run.run.metadata && (
                <pre className="mt-2 max-h-48 overflow-auto rounded bg-slate-900/5 p-2 text-[10px] text-slate-700">
                  {JSON.stringify(run.run.metadata, null, 2)}
                </pre>
              )}
            </section>
            <LifecycleTrustOverlay run={run} delta={delta} />
            {delta && (
              <div className="grid gap-4 md:grid-cols-3">
                <ChangeList title="Trust changes" changes={delta.trust_changes} />
                <ChangeList title="Intelligence changes" changes={delta.intelligence_changes} />
                <ChangeList title="Marketplace changes" changes={delta.marketplace_changes} />
                <ChangeList title="Analytics changes" changes={delta.analytics_changes} />
                <ChangeList title="Artifact changes" changes={delta.artifact_changes} />
              </div>
            )}
          </div>
          <aside className="space-y-3">
            <h3 className="text-sm font-semibold text-slate-700">Workspace timeline</h3>
            <ol className="space-y-2 text-xs">
              {timeline.map((snapshot) => {
                const isActive = snapshot.run.id === run.run.id;
                return (
                  <li
                    key={snapshot.run.id}
                    className={`rounded border p-2 ${
                      isActive ? 'border-sky-400 bg-sky-50 text-sky-800' : 'border-slate-200 bg-slate-50 text-slate-600'
                    }`}
                  >
                    <p className="font-medium">
                      Run #{snapshot.run.id} · {snapshot.run.status}
                    </p>
                    <p>{new Date(snapshot.run.started_at).toLocaleString()}</p>
                  </li>
                );
              })}
            </ol>
          </aside>
        </div>
      </div>
    </div>
  );
}

function formatDuration(seconds: number) {
  if (!Number.isFinite(seconds) || seconds < 0) {
    return 'unknown';
  }
  if (seconds < 60) {
    return `${Math.floor(seconds)}s`;
  }
  const minutes = Math.floor(seconds / 60);
  const remSeconds = Math.floor(seconds % 60);
  if (minutes < 60) {
    return remSeconds === 0 ? `${minutes}m` : `${minutes}m ${remSeconds}s`;
  }
  const hours = Math.floor(minutes / 60);
  const remMinutes = minutes % 60;
  return remMinutes === 0 ? `${hours}h` : `${hours}h ${remMinutes}m`;
}

function renderArtifactDetails(
  artifact: NonNullable<LifecycleRunSnapshot['artifacts']>[number],
) {
  const parts: string[] = [];
  if (artifact.lane) parts.push(`lane=${artifact.lane}`);
  if (artifact.stage) parts.push(`stage=${artifact.stage}`);
  if (artifact.track_name) parts.push(`track=${artifact.track_name}`);
  if (artifact.track_tier) parts.push(`tier=${artifact.track_tier}`);
  if (artifact.manifest_tag) parts.push(`tag=${artifact.manifest_tag}`);
  if (artifact.registry_image) parts.push(`image=${artifact.registry_image}`);
  if (artifact.build_status) parts.push(`build=${artifact.build_status}`);
  if (typeof artifact.duration_seconds === 'number') {
    const formatted = formatDuration(artifact.duration_seconds);
    parts.push(`duration=${formatted}`);
  }
  if (artifact.completed_at) parts.push(`completed=${artifact.completed_at}`);
  return parts.length === 0 ? 'no artifact metadata' : parts.join(', ');
}
