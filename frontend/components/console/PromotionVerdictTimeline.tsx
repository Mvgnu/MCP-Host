'use client';
import { LifecyclePromotionPosture, LifecyclePromotionPostureDelta } from '../../lib/lifecycle-console';
import clsx from 'clsx';

// key: lifecycle-console-ui -> promotion-verdict-timeline
interface Props {
  promotions: LifecyclePromotionPosture[];
  promotionDeltas?: Record<number, LifecyclePromotionPostureDelta>;
}

function formatDate(value: string) {
  try {
    return new Date(value).toLocaleString();
  } catch {
    return value;
  }
}

export default function PromotionVerdictTimeline({ promotions, promotionDeltas }: Props) {
  if (promotions.length === 0) {
    return null;
  }
  const sorted = [...promotions].sort((a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime());
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
    </section>
  );
}
