'use client';
import LifecycleRunProgress from './LifecycleRunProgress';
import LifecycleTrustOverlay from './LifecycleTrustOverlay';
import { LifecycleRunSnapshot } from '../../lib/lifecycle-console';

// key: lifecycle-console-ui -> timeline
interface Props {
  runs: LifecycleRunSnapshot[];
}

export default function LifecycleTimeline({ runs }: Props) {
  if (runs.length === 0) {
    return <p className="text-sm text-slate-500">No remediation runs recorded yet.</p>;
  }
  return (
    <div className="space-y-4">
      {runs.map((run) => (
        <div key={run.run.id} className="space-y-2">
          <LifecycleRunProgress run={run} />
          <LifecycleTrustOverlay run={run} />
        </div>
      ))}
    </div>
  );
}
