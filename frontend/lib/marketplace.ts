// key: marketplace-lib -> provider-console

export interface ProviderMarketplaceSubmission {
  id: string;
  provider_id: string;
  submitted_by?: number | null;
  tier: string;
  manifest_uri: string;
  artifact_digest?: string | null;
  release_notes?: string | null;
  posture_state: Record<string, unknown>;
  posture_vetoed: boolean;
  posture_notes: string[];
  status: string;
  metadata: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

export interface ProviderMarketplaceEvaluation {
  id: string;
  submission_id: string;
  evaluation_type: string;
  status: string;
  started_at: string;
  completed_at?: string | null;
  evaluator_ref?: string | null;
  result: Record<string, unknown>;
  posture_state: Record<string, unknown>;
  posture_vetoed: boolean;
  posture_notes: string[];
  created_at: string;
  updated_at: string;
}

export interface ProviderMarketplacePromotion {
  id: string;
  evaluation_id: string;
  gate: string;
  status: string;
  opened_at: string;
  closed_at?: string | null;
  notes: string[];
  created_at: string;
  updated_at: string;
}

export interface ProviderMarketplaceEvaluationSummary {
  evaluation: ProviderMarketplaceEvaluation;
  promotions: ProviderMarketplacePromotion[];
}

export interface ProviderMarketplaceSubmissionSummary {
  submission: ProviderMarketplaceSubmission;
  evaluations: ProviderMarketplaceEvaluationSummary[];
}

export type ProviderMarketplaceEventType =
  | 'submission_created'
  | 'evaluation_started'
  | 'evaluation_transitioned'
  | 'promotion_created'
  | 'promotion_transitioned';

export interface ProviderMarketplaceStreamEvent {
  id: string;
  provider_id: string;
  submission_id?: string | null;
  evaluation_id?: string | null;
  promotion_id?: string | null;
  event_type: ProviderMarketplaceEventType;
  actor_ref?: string | null;
  payload: Record<string, unknown>;
  occurred_at: string;
}

interface SubmissionPayload {
  tier: string;
  manifest_uri: string;
  artifact_digest?: string;
  release_notes?: string;
  metadata?: Record<string, unknown>;
}

async function handleResponse<T>(response: Response): Promise<T> {
  if (!response.ok) {
    const detail = await response.text();
    throw new Error(detail || response.statusText);
  }
  return (await response.json()) as T;
}

export async function fetchProviderSubmissions(
  providerId: string,
): Promise<ProviderMarketplaceSubmissionSummary[]> {
  const response = await fetch(
    `/api/marketplace/providers/${providerId}/submissions`,
    {
      credentials: 'include',
    },
  );
  return handleResponse(response);
}

export async function createProviderSubmission(
  providerId: string,
  payload: SubmissionPayload,
): Promise<ProviderMarketplaceSubmission> {
  const response = await fetch(
    `/api/marketplace/providers/${providerId}/submissions`,
    {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    },
  );
  return handleResponse(response);
}

export function openMarketplaceEventStream(
  providerId: string,
  onEvent: (event: ProviderMarketplaceStreamEvent) => void,
): () => void {
  let closed = false;
  let eventSource: EventSource | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

  const connect = () => {
    if (closed) {
      return;
    }
    if (eventSource) {
      eventSource.close();
    }

    eventSource = new EventSource(
      `/api/marketplace/providers/${providerId}/events/stream`,
      { withCredentials: true },
    );

    eventSource.onmessage = (message) => {
      try {
        const parsed = JSON.parse(message.data) as ProviderMarketplaceStreamEvent;
        onEvent(parsed);
      } catch (error) {
        console.warn('failed to parse marketplace stream event', error);
      }
    };

    eventSource.onerror = () => {
      if (eventSource) {
        eventSource.close();
      }
      if (closed) {
        return;
      }
      reconnectTimer = setTimeout(connect, 8000);
    };
  };

  connect();

  return () => {
    closed = true;
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
    }
    if (eventSource) {
      eventSource.close();
    }
  };
}
