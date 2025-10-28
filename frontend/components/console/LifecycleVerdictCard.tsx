'use client';
import { LifecycleWorkspaceRevision } from '../../lib/lifecycle-console';

// key: lifecycle-console-ui -> verdict-card
interface Props {
  revision?: LifecycleWorkspaceRevision;
}

function summarizeRevision(revision?: LifecycleWorkspaceRevision) {
  if (!revision) {
    return 'No active revision';
  }
  const gateSummary = revision.gate_snapshots.slice(0, 2).map((snapshot) => {
    return `${snapshot.snapshot_type}: ${snapshot.status}`;
  });
  if (gateSummary.length === 0) {
    return 'No gate snapshots recorded';
  }
  return gateSummary.join(' · ');
}

export default function LifecycleVerdictCard({ revision }: Props) {
  return (
    <div className="border border-indigo-200 bg-indigo-50 text-indigo-900 rounded p-3">
      <p className="text-sm font-semibold">Promotion verdict</p>
      <p className="text-xs mt-1">{summarizeRevision(revision)}</p>
      {revision && (
        <p className="text-xs mt-1 text-indigo-700">
          Revision #{revision.revision.revision_number} updated{' '}
          {new Date(revision.revision.updated_at).toLocaleString()}
        </p>
      )}
    </div>
  );
}
