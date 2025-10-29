'use client';
import {
  LifecycleFieldChange,
  LifecycleRunDelta,
  LifecycleRunSnapshot,
} from '../../lib/lifecycle-console';

// key: lifecycle-console-ui -> trust-overlay
interface Props {
  run: LifecycleRunSnapshot;
  delta?: LifecycleRunDelta;
}

function ChangeGroup({ title, changes }: { title: string; changes: LifecycleFieldChange[] }) {
  if (changes.length === 0) {
    return null;
  }
  return (
    <div>
      <p className="font-semibold text-slate-600">{title}</p>
      <ul className="mt-1 space-y-1">
        {changes.map((change, index) => (
          <li key={`${change.field}-${index}`} className="text-slate-600">
            <span className="font-medium text-slate-700">{change.field}</span>: {change.previous ?? '—'} →{' '}
            {change.current ?? '—'}
          </li>
        ))}
      </ul>
    </div>
  );
}

export default function LifecycleTrustOverlay({ run, delta }: Props) {
  const trust = run.trust;
  const marketplace = run.marketplace;
  const changeGroups = delta
    ? (
        [
          { title: 'Trust changes', entries: delta.trust_changes },
          { title: 'Intelligence changes', entries: delta.intelligence_changes },
          { title: 'Marketplace changes', entries: delta.marketplace_changes },
          { title: 'Analytics changes', entries: delta.analytics_changes },
          { title: 'Artifact changes', entries: delta.artifact_changes },
        ] as { title: string; entries: LifecycleFieldChange[] }[]
      ).filter((group) => group.entries.length > 0)
    : [];
  return (
    <div className="grid gap-2 md:grid-cols-4 text-xs mt-2">
      <div className="bg-slate-50 border border-slate-200 rounded p-2">
        <p className="font-semibold text-slate-700">Trust</p>
        {trust ? (
          <ul className="mt-1 space-y-1">
            <li>Attestation: {trust.attestation_status}</li>
            <li>Lifecycle: {trust.lifecycle_state}</li>
            {trust.remediation_state && <li>Remediation: {trust.remediation_state}</li>}
            <li>Attempts: {trust.remediation_attempts}</li>
          </ul>
        ) : (
          <p className="text-slate-500">No trust record captured.</p>
        )}
      </div>
      <div className="bg-slate-50 border border-slate-200 rounded p-2">
        <p className="font-semibold text-slate-700">Intelligence</p>
        {run.intelligence.length > 0 ? (
          <ul className="mt-1 space-y-1">
            {run.intelligence.slice(0, 3).map((score) => (
              <li key={`${score.capability}-${score.backend ?? 'core'}`}>
                {score.capability}: {score.status} ({Math.round(score.score)} · {(score.confidence * 100).toFixed(0)}% confidence)
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-slate-500">No intelligence scores recorded.</p>
        )}
      </div>
      <div className="bg-slate-50 border border-slate-200 rounded p-2">
        <p className="font-semibold text-slate-700">Marketplace</p>
        {marketplace ? (
          <ul className="mt-1 space-y-1">
            <li>Status: {marketplace.status}</li>
            {marketplace.last_completed_at && <li>Completed: {new Date(marketplace.last_completed_at).toLocaleString()}</li>}
          </ul>
        ) : (
          <p className="text-slate-500">No marketplace readiness captured.</p>
        )}
      </div>
      <div className="bg-slate-50 border border-slate-200 rounded p-2">
        <p className="font-semibold text-slate-700">Recent changes</p>
        {delta ? (
          changeGroups.length > 0 ? (
            <div className="mt-1 space-y-2">
              {changeGroups.map((group) => (
                <ChangeGroup key={group.title} title={group.title} changes={group.entries} />
              ))}
            </div>
          ) : (
            <p className="text-slate-500">No material changes detected.</p>
          )
        ) : (
          <p className="text-slate-500">Awaiting delta telemetry.</p>
        )}
      </div>
    </div>
  );
}
