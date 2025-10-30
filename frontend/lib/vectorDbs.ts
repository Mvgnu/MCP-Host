// key: vector-dbs-lib -> console-governance

export interface VectorDbRecord {
  id: number;
  name: string;
  db_type: string;
  url?: string | null;
  created_at: string;
}

export interface VectorDbResidencyPolicy {
  id: number;
  vector_db_id: number;
  region: string;
  data_classification: string;
  enforcement_mode: string;
  active: boolean;
  created_at: string;
  updated_at: string;
}

export interface UpsertVectorDbResidencyPolicyPayload {
  region: string;
  data_classification?: string;
  enforcement_mode?: string;
  active?: boolean;
}

export interface VectorDbAttachmentRecord {
  id: string;
  vector_db_id: number;
  attachment_type: string;
  attachment_ref: string;
  residency_policy_id: number;
  provider_key_binding_id: string;
  provider_key_id: string;
  provider_key_rotation_due_at?: string | null;
  attached_at: string;
  detached_at?: string | null;
  detached_reason?: string | null;
  metadata: Record<string, unknown>;
}

export interface CreateVectorDbAttachmentPayload {
  attachment_type: string;
  attachment_ref: string;
  residency_policy_id: number;
  provider_key_binding_id: string;
  metadata?: Record<string, unknown>;
}

export interface DetachVectorDbAttachmentPayload {
  reason?: string;
}

export interface VectorDbIncidentRecord {
  id: string;
  vector_db_id: number;
  attachment_id?: string | null;
  incident_type: string;
  severity: string;
  occurred_at: string;
  resolved_at?: string | null;
  summary?: string | null;
  notes: Record<string, unknown>;
}

export interface CreateVectorDbIncidentPayload {
  incident_type: string;
  severity?: string;
  attachment_id?: string;
  summary?: string;
  notes?: Record<string, unknown>;
}

export interface ResolveVectorDbIncidentPayload {
  resolution_summary?: string;
  resolution_notes?: Record<string, unknown>;
}

async function handleResponse<T>(response: Response): Promise<T> {
  if (!response.ok) {
    const detail = await response.text();
    throw new Error(detail || response.statusText);
  }
  return (await response.json()) as T;
}

async function handleEmptyResponse(response: Response): Promise<void> {
  if (!response.ok) {
    const detail = await response.text();
    throw new Error(detail || response.statusText);
  }
}

export async function fetchVectorDbs(): Promise<VectorDbRecord[]> {
  const response = await fetch('/api/vector-dbs', {
    credentials: 'include',
  });
  return handleResponse(response);
}

export async function upsertResidencyPolicy(
  vectorDbId: number,
  payload: UpsertVectorDbResidencyPolicyPayload,
): Promise<VectorDbResidencyPolicy> {
  const response = await fetch(`/api/vector-dbs/${vectorDbId}/residency-policies`, {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  });
  return handleResponse(response);
}

export async function listResidencyPolicies(
  vectorDbId: number,
): Promise<VectorDbResidencyPolicy[]> {
  const response = await fetch(`/api/vector-dbs/${vectorDbId}/residency-policies`, {
    credentials: 'include',
  });
  return handleResponse(response);
}

export async function listAttachments(
  vectorDbId: number,
): Promise<VectorDbAttachmentRecord[]> {
  const response = await fetch(`/api/vector-dbs/${vectorDbId}/attachments`, {
    credentials: 'include',
  });
  return handleResponse(response);
}

export async function createAttachment(
  vectorDbId: number,
  payload: CreateVectorDbAttachmentPayload,
): Promise<VectorDbAttachmentRecord> {
  const response = await fetch(`/api/vector-dbs/${vectorDbId}/attachments`, {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  });
  return handleResponse(response);
}

export async function detachAttachment(
  vectorDbId: number,
  attachmentId: string,
  payload: DetachVectorDbAttachmentPayload,
): Promise<VectorDbAttachmentRecord> {
  const response = await fetch(
    `/api/vector-dbs/${vectorDbId}/attachments/${attachmentId}`,
    {
      method: 'PATCH',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    },
  );
  return handleResponse(response);
}

export async function listIncidents(
  vectorDbId: number,
): Promise<VectorDbIncidentRecord[]> {
  const response = await fetch(`/api/vector-dbs/${vectorDbId}/incidents`, {
    credentials: 'include',
  });
  return handleResponse(response);
}

export async function logIncident(
  vectorDbId: number,
  payload: CreateVectorDbIncidentPayload,
): Promise<VectorDbIncidentRecord> {
  const response = await fetch(`/api/vector-dbs/${vectorDbId}/incidents`, {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  });
  return handleResponse(response);
}

export async function resolveIncident(
  vectorDbId: number,
  incidentId: string,
  payload: ResolveVectorDbIncidentPayload,
): Promise<VectorDbIncidentRecord> {
  const response = await fetch(`/api/vector-dbs/${vectorDbId}/incidents/${incidentId}`, {
    method: 'PATCH',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  });
  return handleResponse(response);
}

export async function deleteVectorDb(vectorDbId: number): Promise<void> {
  const response = await fetch(`/api/vector-dbs/${vectorDbId}`, {
    method: 'DELETE',
    credentials: 'include',
  });
  await handleEmptyResponse(response);
}
