'use client';
import { useCallback, useMemo, useState } from 'react';
import Button from '../Button';
import Spinner from '../Spinner';
import type { DetachVectorDbAttachmentPayload, VectorDbAttachmentRecord } from '../../lib/vectorDbs';

// key: vector-dbs-console -> attachment-list
interface VectorDbAttachmentListProps {
  attachments: VectorDbAttachmentRecord[];
  loading?: boolean;
  onDetach: (
    attachmentId: string,
    payload: DetachVectorDbAttachmentPayload,
  ) => Promise<void> | void;
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

function rotationBadge(rotationDueAt?: string | null): string {
  if (!rotationDueAt) {
    return 'bg-slate-200 text-slate-600';
  }
  const dueDate = new Date(rotationDueAt).getTime();
  const now = Date.now();
  const diffDays = (dueDate - now) / (1000 * 60 * 60 * 24);
  if (diffDays <= 0) {
    return 'bg-red-100 text-red-700';
  }
  if (diffDays <= 14) {
    return 'bg-amber-100 text-amber-700';
  }
  return 'bg-emerald-100 text-emerald-700';
}

export default function VectorDbAttachmentList({
  attachments,
  loading = false,
  onDetach,
}: VectorDbAttachmentListProps) {
  const [reasonById, setReasonById] = useState<Record<string, string>>({});
  const [pending, setPending] = useState<Record<string, boolean>>({});
  const [errorById, setErrorById] = useState<Record<string, string>>({});

  const sortedAttachments = useMemo(
    () =>
      attachments
        .slice()
        .sort((a, b) => new Date(b.attached_at).getTime() - new Date(a.attached_at).getTime()),
    [attachments],
  );

  const handleReasonChange = useCallback((attachmentId: string, value: string) => {
    setReasonById((current) => ({ ...current, [attachmentId]: value }));
  }, []);

  const handleDetach = useCallback(
    async (attachmentId: string) => {
      setPending((current) => ({ ...current, [attachmentId]: true }));
      setErrorById((current) => ({ ...current, [attachmentId]: '' }));
      try {
        const reason = reasonById[attachmentId]?.trim();
        await onDetach(attachmentId, { reason: reason || undefined });
        setReasonById((current) => ({ ...current, [attachmentId]: '' }));
      } catch (cause) {
        console.error('failed to detach vector db attachment', cause);
        setErrorById((current) => ({
          ...current,
          [attachmentId]: cause instanceof Error ? cause.message : 'Failed to detach attachment',
        }));
      } finally {
        setPending((current) => ({ ...current, [attachmentId]: false }));
      }
    },
    [onDetach, reasonById],
  );

  return (
    <section className="border border-slate-200 rounded-lg bg-white shadow-sm p-4 space-y-4">
      <header className="space-y-1">
        <h2 className="text-lg font-semibold text-slate-800">Attachments</h2>
        <p className="text-sm text-slate-600">
          Validate BYOK bindings and residency policies for services using this vector database.
        </p>
      </header>

      {loading ? (
        <div className="flex items-center gap-2 text-sm text-slate-600">
          <Spinner size="sm" /> Loading attachments…
        </div>
      ) : sortedAttachments.length === 0 ? (
        <p className="text-sm text-slate-500">No attachments recorded yet.</p>
      ) : (
        <ul className="space-y-4">
          {sortedAttachments.map((attachment) => {
            const reason = reasonById[attachment.id] ?? '';
            const pendingDetach = pending[attachment.id] ?? false;
            const error = errorById[attachment.id] ?? '';
            const rotationClass = rotationBadge(attachment.provider_key_rotation_due_at);
            const rotationLabel = attachment.provider_key_rotation_due_at
              ? `Rotate by ${formatDate(attachment.provider_key_rotation_due_at)}`
              : 'No rotation deadline recorded';

            return (
              <li key={attachment.id} className="border border-slate-100 rounded-lg p-4 bg-slate-50 space-y-3">
                <div className="flex flex-col md:flex-row md:items-center md:justify-between gap-3">
                  <div className="space-y-1">
                    <h3 className="text-sm font-semibold text-slate-700">{attachment.attachment_type}</h3>
                    <p className="text-xs font-mono text-slate-500 break-all">{attachment.attachment_ref}</p>
                    <p className="text-xs text-slate-500">Policy #{attachment.residency_policy_id}</p>
                  </div>
                  <div className="flex flex-col items-start md:items-end gap-2 text-xs text-slate-500">
                    <p>Attached {formatDate(attachment.attached_at)}</p>
                    <p>Detached {formatDate(attachment.detached_at ?? null)}</p>
                    <span className={`inline-flex items-center px-2 py-1 font-semibold rounded-full ${rotationClass}`}>
                      {rotationLabel}
                    </span>
                  </div>
                </div>

                <p className="text-xs text-slate-500">
                  Provider key binding {attachment.provider_key_binding_id} · Key {attachment.provider_key_id}
                </p>

                {attachment.metadata && Object.keys(attachment.metadata).length > 0 && (
                  <details className="border border-slate-200 bg-white rounded p-2 text-xs text-slate-600">
                    <summary className="cursor-pointer font-semibold text-slate-700">Metadata</summary>
                    <pre className="mt-2 whitespace-pre-wrap break-words">
                      {JSON.stringify(attachment.metadata, null, 2)}
                    </pre>
                  </details>
                )}

                {attachment.detached_at ? (
                  <p className="text-xs text-slate-500">
                    Detached reason: {attachment.detached_reason || 'Not provided'}
                  </p>
                ) : (
                  <div className="space-y-2">
                    <label className="flex flex-col text-sm text-slate-700">
                      Detach reason
                      <input
                        type="text"
                        value={reason}
                        onChange={(event) => handleReasonChange(attachment.id, event.target.value)}
                        className="mt-1 border border-slate-300 rounded px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                        placeholder="Rotation or remediation note"
                      />
                    </label>
                    {error && <p className="text-xs text-red-600">{error}</p>}
                    <Button onClick={() => handleDetach(attachment.id)} disabled={pendingDetach}>
                      {pendingDetach ? 'Detaching…' : 'Detach attachment'}
                    </Button>
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
