'use client';
import { LifecycleRunSnapshot } from '../../lib/lifecycle-console';

// key: lifecycle-console-ui -> trust-overlay
interface Props {
  run: LifecycleRunSnapshot;
}

export default function LifecycleTrustOverlay({ run }: Props) {
  const trust = run.trust;
  const marketplace = run.marketplace;
  return (
    <div className="grid gap-2 md:grid-cols-3 text-xs mt-2">
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
    </div>
  );
}
