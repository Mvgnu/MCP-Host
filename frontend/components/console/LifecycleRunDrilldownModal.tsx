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
              {run.run.failure_reason && <p className="text-xs text-rose-600">Failure: {run.run.failure_reason}</p>}
              {run.run.last_error && <p className="text-xs text-rose-500">Last error: {run.run.last_error}</p>}
              {run.run.approval_required && (
                <p className="text-xs text-amber-600">
                  Awaiting approval – state: {run.run.approval_state ?? 'pending'}
                </p>
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
