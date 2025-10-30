'use client';
import clsx from 'clsx';
import {
  ProviderMarketplaceEvaluationSummary,
  ProviderMarketplaceSubmission,
} from '../../lib/marketplace';

// key: marketplace-console -> submission-card
interface MarketplaceSubmissionCardProps {
  submission: ProviderMarketplaceSubmission;
  evaluations: ProviderMarketplaceEvaluationSummary[];
}

function formatDate(value: string | null | undefined): string {
  if (!value) {
    return '—';
  }
  try {
    return new Date(value).toLocaleString();
  } catch {
    return value;
  }
}

function summarizeNotes(notes: string[]): string {
  if (notes.length === 0) {
    return 'No notes recorded';
  }
  return notes.join(', ');
}

export default function MarketplaceSubmissionCard({
  submission,
  evaluations,
}: MarketplaceSubmissionCardProps) {
  const postureBadge = submission.posture_vetoed ? 'bg-red-100 text-red-700' : 'bg-emerald-100 text-emerald-700';
  const latestEvaluation = [...evaluations].sort(
    (a, b) => new Date(b.evaluation.started_at).getTime() - new Date(a.evaluation.started_at).getTime(),
  )[0];

  return (
    <article className="border border-slate-200 rounded-lg bg-white shadow-sm p-4 space-y-4">
      <header className="flex flex-col md:flex-row md:items-center md:justify-between gap-3">
        <div className="space-y-1">
          <h2 className="text-lg font-semibold text-slate-800">{submission.manifest_uri}</h2>
          <p className="text-sm text-slate-600">
            Tier <span className="font-medium">{submission.tier}</span> · Status{' '}
            <span className="font-medium text-slate-700">{submission.status}</span>
          </p>
          {submission.artifact_digest && (
            <p className="text-xs font-mono text-slate-500 break-all">{submission.artifact_digest}</p>
          )}
        </div>
        <div className="flex flex-col items-start md:items-end gap-2">
          <span className={clsx('px-2 py-1 text-xs font-semibold rounded-full uppercase tracking-wide', postureBadge)}>
            {submission.posture_vetoed ? 'Posture vetoed' : 'Posture healthy'}
          </span>
          <div className="text-xs text-slate-500 space-y-1 text-right">
            <p>Submitted {formatDate(submission.created_at)}</p>
            <p>Updated {formatDate(submission.updated_at)}</p>
          </div>
        </div>
      </header>

      {submission.posture_notes.length > 0 && (
        <section className="border border-slate-100 rounded p-3 bg-slate-50 space-y-1">
          <h3 className="text-sm font-semibold text-slate-700">Posture notes</h3>
          <p className="text-xs text-slate-600">{summarizeNotes(submission.posture_notes)}</p>
        </section>
      )}

      {submission.release_notes && (
        <section className="border border-slate-100 rounded p-3 bg-slate-50 space-y-1">
          <h3 className="text-sm font-semibold text-slate-700">Release notes</h3>
          <p className="text-sm text-slate-600 whitespace-pre-line">{submission.release_notes}</p>
        </section>
      )}

      <section className="space-y-3">
        <header className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-slate-700">Evaluation timeline</h3>
          {latestEvaluation && (
            <span className="text-xs text-slate-500">
              Latest {formatDate(latestEvaluation.evaluation.updated_at)}
            </span>
          )}
        </header>
        {evaluations.length === 0 ? (
          <p className="text-sm text-slate-500">No evaluation runs captured yet.</p>
        ) : (
          <ol className="space-y-3">
            {evaluations
              .slice()
              .sort((a, b) => new Date(b.evaluation.started_at).getTime() - new Date(a.evaluation.started_at).getTime())
              .map(({ evaluation, promotions }) => {
                const promotionCount = promotions.length;
                return (
                  <li key={evaluation.id} className="border border-slate-100 rounded-md p-3 bg-slate-50 space-y-2">
                    <div className="flex flex-col md:flex-row md:items-center md:justify-between gap-2">
                      <div>
                        <p className="text-sm font-semibold text-slate-700">
                          {evaluation.evaluation_type}{' '}
                          <span className="text-xs font-normal text-slate-500">({evaluation.status})</span>
                        </p>
                        <p className="text-xs text-slate-500">
                          Started {formatDate(evaluation.started_at)} · Completed {formatDate(evaluation.completed_at ?? null)}
                        </p>
                      </div>
                      <span className="text-xs text-slate-500 uppercase">
                        {promotionCount === 0 ? 'No promotions opened' : `${promotionCount} promotion${promotionCount > 1 ? 's' : ''}`}
                      </span>
                    </div>
                    {evaluation.posture_notes.length > 0 && (
                      <p className="text-xs text-slate-600">{summarizeNotes(evaluation.posture_notes)}</p>
                    )}
                    {promotions.length > 0 && (
                      <div className="border border-amber-200 bg-amber-50 rounded p-2 space-y-2">
                        <h4 className="text-xs font-semibold text-amber-800 uppercase">Promotion gates</h4>
                        <ul className="space-y-2">
                          {promotions
                            .slice()
                            .sort((a, b) => new Date(b.opened_at).getTime() - new Date(a.opened_at).getTime())
                            .map((promotion) => (
                              <li
                                key={promotion.id}
                                className="bg-white border border-amber-200 rounded p-2 text-xs text-slate-700 space-y-1"
                              >
                                <div className="flex items-center justify-between">
                                  <span className="font-semibold">{promotion.gate}</span>
                                  <span className="uppercase tracking-wide text-[11px] text-amber-700">{promotion.status}</span>
                                </div>
                                <p className="text-slate-500">
                                  Opened {formatDate(promotion.opened_at)} · Closed {formatDate(promotion.closed_at ?? null)}
                                </p>
                                {promotion.notes.length > 0 && (
                                  <ul className="list-disc list-inside text-slate-600">
                                    {promotion.notes.map((note) => (
                                      <li key={note}>{note}</li>
                                    ))}
                                  </ul>
                                )}
                              </li>
                            ))}
                        </ul>
                      </div>
                    )}
                  </li>
                );
              })}
          </ol>
        )}
      </section>

      {submission.metadata && Object.keys(submission.metadata).length > 0 && (
        <details className="border border-slate-100 rounded p-3 bg-slate-50">
          <summary className="cursor-pointer text-sm font-semibold text-slate-700">Metadata</summary>
          <pre className="mt-2 text-xs text-slate-600 whitespace-pre-wrap break-words">
            {JSON.stringify(submission.metadata, null, 2)}
          </pre>
        </details>
      )}
    </article>
  );
}
