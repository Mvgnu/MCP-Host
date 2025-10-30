'use client';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import Alert from '../../../../components/Alert';
import Button from '../../../../components/Button';
import Input from '../../../../components/Input';
import Spinner from '../../../../components/Spinner';
import Textarea from '../../../../components/Textarea';
import {
  VectorDbAttachmentList,
  VectorDbIncidentTimeline,
  VectorDbResidencyCard,
} from '../../../../components/console';
import type {
  CreateVectorDbAttachmentPayload,
  CreateVectorDbIncidentPayload,
  DetachVectorDbAttachmentPayload,
  ResolveVectorDbIncidentPayload,
  UpsertVectorDbResidencyPolicyPayload,
  VectorDbAttachmentRecord,
  VectorDbIncidentRecord,
  VectorDbRecord,
  VectorDbResidencyPolicy,
} from '../../../../lib/vectorDbs';
import {
  createAttachment,
  detachAttachment,
  fetchVectorDbs,
  listAttachments,
  listIncidents,
  listResidencyPolicies,
  logIncident,
  resolveIncident,
  upsertResidencyPolicy,
} from '../../../../lib/vectorDbs';

// key: vector-dbs-console-page -> governance
interface AttachmentFormState {
  attachmentType: string;
  attachmentRef: string;
  residencyPolicyId: string;
  bindingId: string;
  metadata: string;
}

interface IncidentFormState {
  incidentType: string;
  severity: string;
  attachmentId: string;
  summary: string;
  notes: string;
}

export default function VectorDbGovernancePage() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const initialId = searchParams.get('vectorDbId');
  const initialVectorDbId = initialId ? Number.parseInt(initialId, 10) : null;
  const [vectorDbs, setVectorDbs] = useState<VectorDbRecord[]>([]);
  const [vectorDbId, setVectorDbId] = useState<number | null>(initialVectorDbId);
  const [vectorDbInput, setVectorDbInput] = useState(initialId ?? '');
  const [loadingVectorDbs, setLoadingVectorDbs] = useState(true);
  const [loadingResidency, setLoadingResidency] = useState(false);
  const [loadingAttachments, setLoadingAttachments] = useState(false);
  const [loadingIncidents, setLoadingIncidents] = useState(false);
  const [residencyPolicies, setResidencyPolicies] = useState<VectorDbResidencyPolicy[]>([]);
  const [attachments, setAttachments] = useState<VectorDbAttachmentRecord[]>([]);
  const [incidents, setIncidents] = useState<VectorDbIncidentRecord[]>([]);
  const [globalError, setGlobalError] = useState<string | null>(null);
  const [selectionError, setSelectionError] = useState<string | null>(null);
  const [attachmentError, setAttachmentError] = useState<string | null>(null);
  const [incidentError, setIncidentError] = useState<string | null>(null);

  const [attachmentForm, setAttachmentForm] = useState<AttachmentFormState>({
    attachmentType: '',
    attachmentRef: '',
    residencyPolicyId: '',
    bindingId: '',
    metadata: '',
  });

  const [incidentForm, setIncidentForm] = useState<IncidentFormState>({
    incidentType: '',
    severity: 'medium',
    attachmentId: '',
    summary: '',
    notes: '',
  });

  const loadVectorDbs = useCallback(async () => {
    setLoadingVectorDbs(true);
    try {
      setGlobalError(null);
      const records = await fetchVectorDbs();
      setVectorDbs(records);
    } catch (cause) {
      console.error('failed to load vector dbs', cause);
      setGlobalError(cause instanceof Error ? cause.message : 'Failed to load vector databases');
    } finally {
      setLoadingVectorDbs(false);
    }
  }, []);

  const loadResidency = useCallback(
    async (id: number) => {
      setLoadingResidency(true);
      try {
        const policies = await listResidencyPolicies(id);
        setResidencyPolicies(policies);
      } catch (cause) {
        console.error('failed to load residency policies', cause);
        setGlobalError(cause instanceof Error ? cause.message : 'Failed to load residency policies');
      } finally {
        setLoadingResidency(false);
      }
    },
    [],
  );

  const loadAttachments = useCallback(
    async (id: number) => {
      setLoadingAttachments(true);
      try {
        const records = await listAttachments(id);
        setAttachments(records);
      } catch (cause) {
        console.error('failed to load attachments', cause);
        setGlobalError(cause instanceof Error ? cause.message : 'Failed to load attachments');
      } finally {
        setLoadingAttachments(false);
      }
    },
    [],
  );

  const loadIncidents = useCallback(
    async (id: number) => {
      setLoadingIncidents(true);
      try {
        const records = await listIncidents(id);
        setIncidents(records);
      } catch (cause) {
        console.error('failed to load incidents', cause);
        setGlobalError(cause instanceof Error ? cause.message : 'Failed to load incidents');
      } finally {
        setLoadingIncidents(false);
      }
    },
    [],
  );

  useEffect(() => {
    loadVectorDbs();
  }, [loadVectorDbs]);

  useEffect(() => {
    if (!vectorDbId) {
      return;
    }
    loadResidency(vectorDbId);
    loadAttachments(vectorDbId);
    loadIncidents(vectorDbId);
  }, [vectorDbId, loadResidency, loadAttachments, loadIncidents]);

  const selectedVectorDb = useMemo(
    () => vectorDbs.find((item) => item.id === vectorDbId) ?? null,
    [vectorDbs, vectorDbId],
  );

  const handleSelectVectorDb = useCallback(
    (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const trimmed = vectorDbInput.trim();
      if (!trimmed) {
        setVectorDbId(null);
        setResidencyPolicies([]);
        setAttachments([]);
        setIncidents([]);
        router.replace('?');
        return;
      }
      const parsed = Number.parseInt(trimmed, 10);
      if (Number.isNaN(parsed)) {
        setSelectionError('Vector DB ID must be a number.');
        return;
      }
      setSelectionError(null);
      setVectorDbId(parsed);
      const params = new URLSearchParams(window.location.search);
      params.set('vectorDbId', parsed.toString());
      router.replace(`?${params.toString()}`);
    },
    [router, vectorDbInput],
  );

  const handleAttachmentFormChange = useCallback(
    (event: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      const { name, value } = event.target;
      setAttachmentForm((current) => ({ ...current, [name]: value }));
    },
    [],
  );

  const handleIncidentFormChange = useCallback(
    (event: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      const { name, value } = event.target;
      setIncidentForm((current) => ({ ...current, [name]: value }));
    },
    [],
  );

  const handleCreateAttachment = useCallback(
    async (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      if (!vectorDbId) {
        setAttachmentError('Select a vector DB before attaching workloads.');
        return;
      }
      const residencyPolicyId = Number.parseInt(attachmentForm.residencyPolicyId, 10);
      if (Number.isNaN(residencyPolicyId)) {
        setAttachmentError('Residency policy ID must be numeric.');
        return;
      }
      setAttachmentError(null);
      let metadata: Record<string, unknown> | undefined;
      if (attachmentForm.metadata.trim()) {
        try {
          metadata = JSON.parse(attachmentForm.metadata);
        } catch (error) {
          setAttachmentError('Attachment metadata must be valid JSON.');
          return;
        }
      }
      const payload: CreateVectorDbAttachmentPayload = {
        attachment_type: attachmentForm.attachmentType.trim(),
        attachment_ref: attachmentForm.attachmentRef.trim(),
        residency_policy_id: residencyPolicyId,
        provider_key_binding_id: attachmentForm.bindingId.trim(),
        metadata,
      };
      try {
        await createAttachment(vectorDbId, payload);
        setAttachmentForm({ attachmentType: '', attachmentRef: '', residencyPolicyId: '', bindingId: '', metadata: '' });
        await loadAttachments(vectorDbId);
      } catch (cause) {
        console.error('failed to create attachment', cause);
        setAttachmentError(cause instanceof Error ? cause.message : 'Failed to create attachment');
      }
    },
    [attachmentForm, loadAttachments, vectorDbId],
  );

  const handleLogIncident = useCallback(
    async (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      if (!vectorDbId) {
        setIncidentError('Select a vector DB before logging incidents.');
        return;
      }
      setIncidentError(null);
      let notes: Record<string, unknown> | undefined;
      if (incidentForm.notes.trim()) {
        try {
          notes = JSON.parse(incidentForm.notes);
        } catch (error) {
          setIncidentError('Incident notes must be valid JSON.');
          return;
        }
      }
      const payload: CreateVectorDbIncidentPayload = {
        incident_type: incidentForm.incidentType.trim(),
        severity: incidentForm.severity.trim() || undefined,
        attachment_id: incidentForm.attachmentId.trim() || undefined,
        summary: incidentForm.summary.trim() || undefined,
        notes,
      };
      try {
        await logIncident(vectorDbId, payload);
        setIncidentForm({ incidentType: '', severity: 'medium', attachmentId: '', summary: '', notes: '' });
        await loadIncidents(vectorDbId);
      } catch (cause) {
        console.error('failed to log incident', cause);
        setIncidentError(cause instanceof Error ? cause.message : 'Failed to log incident');
      }
    },
    [incidentForm, loadIncidents, vectorDbId],
  );

  const handleUpsertResidency = useCallback(
    async (payload: UpsertVectorDbResidencyPolicyPayload) => {
      if (!vectorDbId) {
        throw new Error('Select a vector DB before updating residency.');
      }
      await upsertResidencyPolicy(vectorDbId, payload);
      await loadResidency(vectorDbId);
    },
    [loadResidency, vectorDbId],
  );

  const handleDetachAttachment = useCallback(
    async (attachmentId: string, payload: DetachVectorDbAttachmentPayload) => {
      if (!vectorDbId) {
        throw new Error('Vector DB not selected');
      }
      await detachAttachment(vectorDbId, attachmentId, payload);
      await loadAttachments(vectorDbId);
    },
    [loadAttachments, vectorDbId],
  );

  const handleResolveIncident = useCallback(
    async (incidentId: string, payload: ResolveVectorDbIncidentPayload) => {
      if (!vectorDbId) {
        throw new Error('Vector DB not selected');
      }
      await resolveIncident(vectorDbId, incidentId, payload);
      await loadIncidents(vectorDbId);
    },
    [loadIncidents, vectorDbId],
  );

  const vectorDbOptions = useMemo(
    () =>
      vectorDbs.map((record) => (
        <option key={record.id} value={record.id}>
          {record.id} · {record.name}
        </option>
      )),
    [vectorDbs],
  );

  return (
    <div className="space-y-6">
      <header className="space-y-2">
        <h1 className="text-2xl font-semibold text-slate-900">Vector DB governance</h1>
        <p className="text-sm text-slate-600">
          Manage residency policies, BYOK bindings, and incident triage for federated vector databases.
        </p>
      </header>

      {globalError && <Alert message={globalError} type="error" />}

      <section className="border border-slate-200 rounded-lg bg-white shadow-sm p-4 space-y-3">
        <h2 className="text-lg font-semibold text-slate-800">Select vector DB</h2>
        <form className="space-y-3" onSubmit={handleSelectVectorDb}>
          <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
            <label className="flex flex-col gap-2 text-sm text-slate-700">
              Select from managed vector DBs
              <select
                value={vectorDbInput}
                onChange={(event) => setVectorDbInput(event.target.value)}
                className="border border-slate-300 rounded px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
              >
                <option value="">Choose vector DB…</option>
                {vectorDbOptions}
              </select>
            </label>
            <Input
              label="Vector DB ID"
              name="vectorDbId"
              value={vectorDbInput}
              onChange={(event) => setVectorDbInput(event.target.value)}
              placeholder="44"
            />
          </div>
          {selectionError && <p className="text-sm text-red-600">{selectionError}</p>}
          <Button disabled={loadingVectorDbs}>{loadingVectorDbs ? 'Loading…' : 'Load vector DB'}</Button>
        </form>
      </section>

      {selectedVectorDb ? (
        <section className="space-y-6">
          <div className="border border-slate-200 rounded-lg bg-white shadow-sm p-4 space-y-1">
            <h2 className="text-lg font-semibold text-slate-800">{selectedVectorDb.name}</h2>
            <p className="text-sm text-slate-600">
              Created {new Date(selectedVectorDb.created_at).toLocaleString()} · Type {selectedVectorDb.db_type}
            </p>
          </div>

          <VectorDbResidencyCard
            policies={residencyPolicies}
            onUpsert={handleUpsertResidency}
            loading={loadingResidency}
          />

          <section className="border border-slate-200 rounded-lg bg-white shadow-sm p-4 space-y-3">
            <h2 className="text-lg font-semibold text-slate-800">Attach workload</h2>
            <form className="space-y-3" onSubmit={handleCreateAttachment}>
              <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                <Input
                  label="Attachment type"
                  name="attachmentType"
                  value={attachmentForm.attachmentType}
                  onChange={handleAttachmentFormChange}
                  placeholder="assistant"
                  required
                />
                <Input
                  label="Attachment reference"
                  name="attachmentRef"
                  value={attachmentForm.attachmentRef}
                  onChange={handleAttachmentFormChange}
                  placeholder="service-uuid"
                  required
                />
                <Input
                  label="Residency policy ID"
                  name="residencyPolicyId"
                  value={attachmentForm.residencyPolicyId}
                  onChange={handleAttachmentFormChange}
                  placeholder="10"
                  required
                />
                <Input
                  label="Provider key binding ID"
                  name="bindingId"
                  value={attachmentForm.bindingId}
                  onChange={handleAttachmentFormChange}
                  placeholder="binding-uuid"
                  required
                />
              </div>
              <Textarea
                label="Attachment metadata (JSON)"
                name="metadata"
                value={attachmentForm.metadata}
                onChange={handleAttachmentFormChange}
                placeholder='{"environment":"prod"}'
                rows={2}
              />
              {attachmentError && <p className="text-sm text-red-600">{attachmentError}</p>}
              <Button>Attach workload</Button>
            </form>
          </section>

          <VectorDbAttachmentList
            attachments={attachments}
            loading={loadingAttachments}
            onDetach={handleDetachAttachment}
          />

          <section className="border border-slate-200 rounded-lg bg-white shadow-sm p-4 space-y-3">
            <h2 className="text-lg font-semibold text-slate-800">Log incident</h2>
            <form className="space-y-3" onSubmit={handleLogIncident}>
              <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                <Input
                  label="Incident type"
                  name="incidentType"
                  value={incidentForm.incidentType}
                  onChange={handleIncidentFormChange}
                  placeholder="residency_breach"
                  required
                />
                <Input
                  label="Severity"
                  name="severity"
                  value={incidentForm.severity}
                  onChange={handleIncidentFormChange}
                  placeholder="high"
                />
                <Input
                  label="Attachment ID"
                  name="attachmentId"
                  value={incidentForm.attachmentId}
                  onChange={handleIncidentFormChange}
                  placeholder="attach-uuid"
                />
                <Input
                  label="Incident summary"
                  name="summary"
                  value={incidentForm.summary}
                  onChange={handleIncidentFormChange}
                  placeholder="Policy violation context"
                />
              </div>
              <Textarea
                label="Incident notes (JSON)"
                name="notes"
                value={incidentForm.notes}
                onChange={handleIncidentFormChange}
                placeholder='{"impact":"regional"}'
                rows={2}
              />
              {incidentError && <p className="text-sm text-red-600">{incidentError}</p>}
              <Button>Log incident</Button>
            </form>
          </section>

          <VectorDbIncidentTimeline
            incidents={incidents}
            loading={loadingIncidents}
            onResolve={handleResolveIncident}
          />
        </section>
      ) : loadingVectorDbs ? (
        <div className="flex items-center gap-2 text-sm text-slate-600">
          <Spinner size="sm" /> Loading managed vector databases…
        </div>
      ) : (
        <p className="text-sm text-slate-500">
          Select a vector DB to manage residency posture, attachments, and incidents.
        </p>
      )}
    </div>
  );
}
