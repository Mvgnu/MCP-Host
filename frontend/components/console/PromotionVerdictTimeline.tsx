'use client';
import {
  LifecyclePromotionPosture,
  LifecyclePromotionPostureDelta,
  LifecyclePromotionRunDelta,
  RemediationRun,
} from '../../lib/lifecycle-console';
import clsx from 'clsx';

// key: lifecycle-console-ui -> promotion-verdict-timeline
interface Props {
  promotions: LifecyclePromotionPosture[];
  promotionRuns: RemediationRun[];
  promotionDeltas?: Record<number, LifecyclePromotionPostureDelta>;
  promotionRunDeltas?: Record<number, LifecyclePromotionRunDelta>;
}

function formatDate(value: string) {
  try {
    return new Date(value).toLocaleString();
  } catch {
    return value;
  }
}

export default function PromotionVerdictTimeline({
  promotions,
  promotionRuns,
  promotionDeltas,
  promotionRunDeltas,
}: Props) {
  if (promotions.length === 0) {
    return null;
  }
  const sorted = [...promotions].sort((a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime());
  const automationRuns = [...promotionRuns].sort(
    (a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
  );
  return (
    <section className="border border-amber-200 bg-amber-50 text-amber-900 rounded p-3 space-y-3">
      <header className="flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold">Promotion posture</h3>
        <span className="text-xs text-amber-700">Latest {formatDate(sorted[0].updated_at)}</span>
      </header>
      <ol className="space-y-2">
        {sorted.map((promotion) => {
          const delta = promotionDeltas?.[promotion.promotion_id];
          const blocked = !promotion.allowed;
          const vetoSummary = promotion.veto_reasons.length > 0 ? promotion.veto_reasons.join(', ') : 'No veto reasons recorded';
          return (
            <li key={promotion.promotion_id} className="bg-white rounded border border-amber-200 p-2 space-y-2">
              <div className="flex items-start justify-between gap-3">
                <div>
                  <p className="text-sm font-semibold">
                    {promotion.track_name} · {promotion.stage}{' '}
                    <span className="text-xs font-normal text-slate-500">({promotion.status})</span>
                  </p>
                  <p className="text-xs text-slate-500">Track tier: {promotion.track_tier}</p>
                </div>
                <span
                  className={clsx('text-xs font-semibold uppercase', blocked ? 'text-red-600' : 'text-emerald-600')}
                >
                  {blocked ? 'Blocked' : 'Allowed'}
                </span>
              </div>
              {delta && (
                <p className="text-xs text-amber-600">Updated {formatDate(delta.updated_at)}</p>
              )}
              <div className="text-xs space-y-1">
                <p className="font-semibold text-slate-700">Veto reasons</p>
                <p className="text-slate-600">{vetoSummary}</p>
                {promotion.remediation_hooks.length > 0 && (
                  <div>
                    <p className="font-semibold text-slate-700">Remediation hooks</p>
                    <ul className="list-disc list-inside text-slate-600">
                      {promotion.remediation_hooks.map((hook) => (
                        <li key={hook}>{hook}</li>
                      ))}
                    </ul>
                  </div>
                )}
                {promotion.notes.length > 0 && (
                  <div>
                    <p className="font-semibold text-slate-700">Notes</p>
                    <ul className="list-disc list-inside text-slate-600">
                      {promotion.notes.map((note) => (
                        <li key={note}>{note}</li>
                      ))}
                    </ul>
                  </div>
                )}
                {promotion.signals && (
                  <details className="border border-slate-200 rounded p-2 bg-slate-50">
                    <summary className="cursor-pointer text-slate-700">Signal metadata</summary>
                    <pre className="mt-1 whitespace-pre-wrap break-words text-[11px] text-slate-600">
                      {JSON.stringify(promotion.signals, null, 2)}
                    </pre>
                  </details>
                )}
              </div>
            </li>
          );
        })}
      </ol>
      {automationRuns.length > 0 && (
        <div className="space-y-2 border border-amber-200 rounded bg-white p-3">
          <header className="flex items-center justify-between gap-2">
            <h4 className="text-sm font-semibold text-slate-700">Promotion automation</h4>
            <span className="text-xs text-amber-700">
              Latest {formatDate(automationRuns[0].updated_at)}
            </span>
          </header>
          <div className="overflow-x-auto">
            <table className="min-w-full text-xs text-left">
              <thead className="text-slate-600 uppercase">
                <tr>
                  <th className="px-2 py-1">Run ID</th>
                  <th className="px-2 py-1">Status</th>
                  <th className="px-2 py-1">Attempt</th>
                  <th className="px-2 py-1">Lane</th>
                  <th className="px-2 py-1">Stage</th>
                  <th className="px-2 py-1">Updated</th>
                  <th className="px-2 py-1">Changes</th>
                </tr>
              </thead>
              <tbody>
                {automationRuns.map((run) => {
                  const gateContext = extractGateContext(run.promotion_gate_context ?? {});
                  const attempt = extractAttempt(run.automation_payload ?? {});
                  const delta = promotionRunDeltas?.[run.id];
                  const changeSummary = summarizePromotionRunChanges(delta);
                  return (
                    <tr key={run.id} className="border-t border-amber-100">
                      <td className="px-2 py-1 font-mono text-[11px] text-slate-700">{run.id}</td>
                      <td className="px-2 py-1 text-slate-700">{run.status}</td>
                      <td className="px-2 py-1 text-slate-600">{attempt ?? '–'}</td>
                      <td className="px-2 py-1 text-slate-600">{gateContext.lane ?? '–'}</td>
                      <td className="px-2 py-1 text-slate-600">{gateContext.stage ?? '–'}</td>
                      <td className="px-2 py-1 text-slate-600">{formatDate(run.updated_at)}</td>
                      <td className="px-2 py-1 text-slate-600">
                        {changeSummary.length > 0 ? (
                          <ul className="list-disc list-inside space-y-0.5">
                            {changeSummary.map((entry) => (
                              <li key={entry}>{entry}</li>
                            ))}
                          </ul>
                        ) : (
                          <span className="text-slate-400">No recent changes</span>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </section>
  );
}

function extractAttempt(payload: Record<string, unknown> | null | undefined): string | null {
  if (!payload || typeof payload !== 'object') {
    return null;
  }
  const attempt = (payload as Record<string, unknown>).attempt;
  if (typeof attempt === 'number') {
    return String(attempt);
  }
  if (typeof attempt === 'string') {
    return attempt;
  }
  return null;
}

function extractGateContext(value: unknown): { lane?: string; stage?: string } {
  if (!value || typeof value !== 'object') {
    return {};
  }
  const lane = typeof (value as Record<string, unknown>).lane === 'string'
    ? ((value as Record<string, unknown>).lane as string)
    : undefined;
  const stage = typeof (value as Record<string, unknown>).stage === 'string'
    ? ((value as Record<string, unknown>).stage as string)
    : undefined;
  return { lane, stage };
}

function summarizePromotionRunChanges(delta?: LifecyclePromotionRunDelta): string[] {
  if (!delta) return [];
  const summaries: string[] = [];
  if (delta.automation_payload_changes.length > 0) {
    summaries.push('automation payload updated');
  }
  if (delta.gate_context_changes.length > 0) {
    summaries.push('gate context updated');
  }
  if (delta.metadata_changes.length > 0) {
    summaries.push('metadata refreshed');
  }
  return summaries;
}
