// key: billing-lib -> rest-contracts

export interface BillingPlan {
  id: string;
  code: string;
  name: string;
  description?: string | null;
  billing_period: string;
  currency: string;
  amount_cents: number;
  active: boolean;
  created_at: string;
  updated_at: string;
}

export interface PlanEntitlement {
  id: string;
  plan_id: string;
  entitlement_key: string;
  limit_quantity?: number | null;
  reset_interval: string;
  metadata: Record<string, unknown>;
}

export interface OrganizationSubscription {
  id: string;
  organization_id: number;
  plan_id: string;
  status: string;
  trial_ends_at?: string | null;
  current_period_start: string;
  current_period_end?: string | null;
  canceled_at?: string | null;
  created_at: string;
  updated_at: string;
}

export interface BillingPlanCatalogEntry {
  plan: BillingPlan;
  entitlements: PlanEntitlement[];
}

export interface SubscriptionEnvelope {
  subscription: OrganizationSubscription;
  plan: BillingPlan;
}

export interface BillingQuotaOutcome {
  allowed: boolean;
  entitlement_key: string;
  limit_quantity?: number | null;
  used_quantity: number;
  remaining_quantity?: number | null;
  notes: string[];
}

export interface QuotaCheckResponse {
  outcome: BillingQuotaOutcome;
  recorded: boolean;
}

async function handleResponse<T>(response: Response): Promise<T> {
  if (!response.ok) {
    const detail = await response.text();
    throw new Error(detail || response.statusText);
  }
  return (await response.json()) as T;
}

export async function fetchPlanCatalog(): Promise<BillingPlanCatalogEntry[]> {
  const response = await fetch('/api/billing/catalog', {
    credentials: 'include',
  });
  return handleResponse(response);
}

export async function fetchPlans(): Promise<BillingPlan[]> {
  const response = await fetch('/api/billing/plans', {
    credentials: 'include',
  });
  return handleResponse(response);
}

export async function fetchSubscription(
  organizationId: number,
): Promise<SubscriptionEnvelope | null> {
  const response = await fetch(
    `/api/billing/organizations/${organizationId}/subscription`,
    {
      credentials: 'include',
    },
  );
  return handleResponse(response);
}

interface UpsertSubscriptionPayload {
  plan_id: string;
  status?: string;
  trial_ends_at?: string;
}

export async function upsertSubscription(
  organizationId: number,
  payload: UpsertSubscriptionPayload,
): Promise<SubscriptionEnvelope> {
  const response = await fetch(
    `/api/billing/organizations/${organizationId}/subscription`,
    {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    },
  );
  return handleResponse(response);
}

export async function checkQuota(
  organizationId: number,
  entitlementKey: string,
  requestedQuantity = 0,
): Promise<QuotaCheckResponse> {
  const response = await fetch(
    `/api/billing/organizations/${organizationId}/quotas/check`,
    {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        entitlement_key: entitlementKey,
        requested_quantity: requestedQuantity,
        record_usage: false,
      }),
    },
  );
  return handleResponse(response);
}
