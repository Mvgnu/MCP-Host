'use client';
import LifecycleRunProgress from './LifecycleRunProgress';
import LifecycleTrustOverlay from './LifecycleTrustOverlay';
import { LifecycleRunDelta, LifecycleRunSnapshot } from '../../lib/lifecycle-console';

// key: lifecycle-console-ui -> timeline
interface Props {
  runs: LifecycleRunSnapshot[];
  onRunSelect?: (run: LifecycleRunSnapshot) => void;
  runDeltas?: Record<number, LifecycleRunDelta>;
  onApproveRun?: (run: LifecycleRunSnapshot) => void;
  onRejectRun?: (run: LifecycleRunSnapshot) => void;
  pendingApprovals?: Record<number, boolean>;
}

export default function LifecycleTimeline({
  runs,
  onRunSelect,
  runDeltas,
  onApproveRun,
  onRejectRun,
  pendingApprovals,
}: Props) {
  if (runs.length === 0) {
    return <p className="text-sm text-slate-500">No remediation runs recorded yet.</p>;
  }
  return (
    <div className="space-y-4">
      {runs.map((run) => (
        <div key={run.run.id} className="space-y-2">
          <LifecycleRunProgress
            run={run}
            onSelect={onRunSelect}
            onApprove={onApproveRun}
            onReject={onRejectRun}
            pendingApproval={pendingApprovals?.[run.run.id] ?? false}
          />
          <LifecycleTrustOverlay run={run} delta={runDeltas?.[run.run.id]} />
        </div>
      ))}
    </div>
  );
}
