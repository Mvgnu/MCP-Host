'use client';

// key: billing-console-ui -> subscription-wizard

import { useEffect, useMemo, useState } from 'react';
import Alert from '../../../../components/Alert';
import Button from '../../../../components/Button';
import Card from '../../../../components/Card';
import Spinner from '../../../../components/Spinner';
import {
  BillingPlanCatalogEntry,
  BillingQuotaOutcome,
  SubscriptionEnvelope,
  checkQuota,
  fetchPlanCatalog,
  fetchSubscription,
  upsertSubscription,
} from '../../../../lib/billing';
import { BillingPlanComparison } from '../../../../components/console';

const DEFAULT_TRIAL_DAYS = 14;
const MS_PER_DAY = 86_400_000;

type BillingWizardParams = {
  params: { organizationId: string };
};

function computeTrialDate(days: number) {
  const now = new Date();
  const target = new Date(now.getTime() + Math.max(1, days) * MS_PER_DAY);
  return target;
}

function describeTrial(trialEndsAt: string | null | undefined) {
  if (!trialEndsAt) {
    return null;
  }
  const date = new Date(trialEndsAt);
  if (Number.isNaN(date.getTime())) {
    return null;
  }
  return date.toLocaleString();
}

export default function BillingWizardPage({ params }: BillingWizardParams) {
  const organizationId = Number.parseInt(params.organizationId, 10);
  const [catalog, setCatalog] = useState<BillingPlanCatalogEntry[]>([]);
  const [subscription, setSubscription] = useState<SubscriptionEnvelope | null>(null);
  const [selectedPlanId, setSelectedPlanId] = useState<string | null>(null);
  const [selectedEntitlement, setSelectedEntitlement] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionStatus, setActionStatus] = useState<'idle' | 'saving' | 'success' | 'error'>('idle');
  const [actionMessage, setActionMessage] = useState('');
  const [trialEnabled, setTrialEnabled] = useState(false);
  const [trialDays, setTrialDays] = useState<number>(DEFAULT_TRIAL_DAYS);
  const [quotaOutcome, setQuotaOutcome] = useState<BillingQuotaOutcome | null>(null);
  const [quotaLoading, setQuotaLoading] = useState(false);
  const [quotaError, setQuotaError] = useState<string | null>(null);

  const selectedPlan = useMemo(
    () => catalog.find((entry) => entry.plan.id === selectedPlanId) ?? null,
    [catalog, selectedPlanId],
  );

  const trialEndDate = trialEnabled ? computeTrialDate(trialDays) : null;

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    (async () => {
      try {
        const [planCatalog, activeSubscription] = await Promise.all([
          fetchPlanCatalog(),
          fetchSubscription(organizationId),
        ]);
        if (cancelled) {
          return;
        }
        setCatalog(planCatalog);
        setSubscription(activeSubscription);
        const initialPlanId =
          activeSubscription?.plan.id ?? planCatalog[0]?.plan.id ?? null;
        setSelectedPlanId(initialPlanId);
        if (initialPlanId) {
          const initialPlan = planCatalog.find((entry) => entry.plan.id === initialPlanId);
          const firstEntitlement = initialPlan?.entitlements[0]?.entitlement_key ?? null;
          setSelectedEntitlement(firstEntitlement);
        }
        if (
          activeSubscription?.subscription.status === 'trialing' &&
          activeSubscription.subscription.trial_ends_at
        ) {
          const remainingMs =
            new Date(activeSubscription.subscription.trial_ends_at).getTime() -
            Date.now();
          const remainingDays = Math.max(1, Math.ceil(remainingMs / MS_PER_DAY));
          setTrialEnabled(true);
          setTrialDays(remainingDays);
        } else {
          setTrialEnabled(false);
          setTrialDays(DEFAULT_TRIAL_DAYS);
        }
      } catch (err) {
        if (cancelled) {
          return;
        }
        setError(err instanceof Error ? err.message : 'Failed to load billing catalog');
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [organizationId]);

  useEffect(() => {
    if (!selectedPlanId) {
      setSelectedEntitlement(null);
      return;
    }
    const match = catalog.find((entry) => entry.plan.id === selectedPlanId);
    if (!match) {
      setSelectedEntitlement(null);
      return;
    }
    const hasExisting = match.entitlements.some(
      (entitlement) => entitlement.entitlement_key === selectedEntitlement,
    );
    if (!hasExisting) {
      setSelectedEntitlement(match.entitlements[0]?.entitlement_key ?? null);
    }
  }, [catalog, selectedPlanId, selectedEntitlement]);

  useEffect(() => {
    if (!selectedEntitlement) {
      setQuotaOutcome(null);
      setQuotaLoading(false);
      return;
    }
    let cancelled = false;
    setQuotaLoading(true);
    setQuotaError(null);
    checkQuota(organizationId, selectedEntitlement, 0)
      .then((response) => {
        if (!cancelled) {
          setQuotaOutcome(response.outcome);
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setQuotaOutcome(null);
          setQuotaError(err instanceof Error ? err.message : 'Failed to evaluate quota');
        }
      })
      .finally(() => {
        if (!cancelled) {
          setQuotaLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [organizationId, selectedEntitlement]);

  const assignPlan = async () => {
    if (!selectedPlan) {
      return;
    }
    setActionStatus('saving');
    setActionMessage('');
    try {
      const payload: { plan_id: string; status?: string; trial_ends_at?: string } = {
        plan_id: selectedPlan.plan.id,
      };
      if (trialEnabled) {
        const trialEnds = computeTrialDate(trialDays).toISOString();
        payload.status = 'trialing';
        payload.trial_ends_at = trialEnds;
      }
      const response = await upsertSubscription(organizationId, payload);
      setSubscription(response);
      setActionStatus('success');
      setActionMessage('Subscription updated successfully.');
    } catch (err) {
      setActionStatus('error');
      setActionMessage(
        err instanceof Error ? err.message : 'Failed to update subscription.',
      );
    }
  };

  const subscriptionStatus = subscription?.subscription.status ?? 'unassigned';
  const trialDescription = describeTrial(subscription?.subscription.trial_ends_at);

  return (
    <div className="space-y-6 py-6">
      <header className="space-y-2">
        <h1 className="text-3xl font-semibold text-white">Subscription Wizard</h1>
        <p className="text-sm text-gray-400">
          Compare plans, stage trials, and reconcile billing notes for organization #{' '}
          {organizationId}.
        </p>
      </header>

      {error && <Alert message={error} type="error" />}

      {loading ? (
        <div className="flex justify-center py-12">
          <Spinner />
        </div>
      ) : (
        <div className="space-y-8">
          <section className="space-y-4">
            <h2 className="text-xl font-semibold text-white">Choose a plan</h2>
            <BillingPlanComparison
              plans={catalog}
              selectedPlanId={selectedPlanId}
              onSelect={setSelectedPlanId}
            />
          </section>

          <section className="grid gap-4 lg:grid-cols-2">
            <Card>
              <div className="space-y-4">
                <div>
                  <h3 className="text-lg font-semibold text-white">Plan details</h3>
                  {selectedPlan ? (
                    <p className="text-sm text-gray-300">
                      {selectedPlan.plan.name} · {selectedPlan.plan.billing_period}{' '}
                      billing · {(selectedPlan.plan.amount_cents / 100).toLocaleString(undefined, { minimumFractionDigits: 2 })}{' '}
                      {selectedPlan.plan.currency.toUpperCase()}
                    </p>
                  ) : (
                    <p className="text-sm text-gray-400">Select a plan to continue.</p>
                  )}
                </div>

                <div className="space-y-2">
                  <label className="flex items-center gap-2 text-sm text-gray-200">
                    <input
                      type="checkbox"
                      checked={trialEnabled}
                      onChange={(event) => setTrialEnabled(event.target.checked)}
                    />
                    Start with a trial period
                  </label>
                  {trialEnabled && (
                    <div className="flex items-center gap-2 text-sm text-gray-200">
                      <label htmlFor="trial-days" className="text-gray-300">
                        Trial length (days)
                      </label>
                      <input
                        id="trial-days"
                        type="number"
                        min={1}
                        value={trialDays}
                        onChange={(event) =>
                          setTrialDays(Math.max(1, Number.parseInt(event.target.value, 10) || DEFAULT_TRIAL_DAYS))
                        }
                        className="w-20 rounded border border-gray-600 bg-gray-900 px-2 py-1"
                      />
                      <span className="text-xs text-gray-400">
                        Ends on {trialEndDate?.toLocaleString()}
                      </span>
                    </div>
                  )}
                </div>

                <div>
                  <Button onClick={assignPlan} disabled={actionStatus === 'saving' || !selectedPlan}>
                    {actionStatus === 'saving' ? 'Assigning…' : 'Assign plan'}
                  </Button>
                  {actionStatus === 'success' && (
                    <p className="mt-2 text-sm text-green-400">{actionMessage}</p>
                  )}
                  {actionStatus === 'error' && (
                    <p className="mt-2 text-sm text-red-400">{actionMessage}</p>
                  )}
                </div>
              </div>
            </Card>

            <Card>
              <div className="space-y-4">
                <div>
                  <h3 className="text-lg font-semibold text-white">Billing notes</h3>
                  <p className="text-sm text-gray-300">
                    Evaluate entitlement quotas to surface BillingQuotaOutcome notes without recording usage.
                  </p>
                </div>
                {selectedPlan && selectedPlan.entitlements.length > 0 ? (
                  <div className="space-y-3">
                    <label className="text-sm text-gray-300" htmlFor="entitlement-select">
                      Entitlement
                    </label>
                    <select
                      id="entitlement-select"
                      value={selectedEntitlement ?? ''}
                      onChange={(event) => setSelectedEntitlement(event.target.value || null)}
                      className="w-full rounded border border-gray-600 bg-gray-900 px-3 py-2 text-sm"
                    >
                      {selectedPlan.entitlements.map((entitlement) => (
                        <option key={entitlement.entitlement_key} value={entitlement.entitlement_key}>
                          {typeof entitlement.metadata.label === 'string'
                            ? entitlement.metadata.label
                            : entitlement.entitlement_key}
                        </option>
                      ))}
                    </select>
                    <div className="rounded border border-gray-700 bg-gray-900/60 p-3 text-sm text-gray-200">
                      {quotaLoading && <span>Evaluating quota…</span>}
                      {!quotaLoading && quotaOutcome && (
                        <div className="space-y-1">
                          <div className="font-medium text-white">
                            {quotaOutcome.allowed ? 'Allowed' : 'Blocked'}
                          </div>
                          <div>Used: {quotaOutcome.used_quantity}</div>
                          {quotaOutcome.limit_quantity != null && (
                            <div>Limit: {quotaOutcome.limit_quantity}</div>
                          )}
                          {quotaOutcome.remaining_quantity != null && (
                            <div>Remaining: {quotaOutcome.remaining_quantity}</div>
                          )}
                          {quotaOutcome.notes.length > 0 && (
                            <ul className="list-disc pl-5 text-xs text-gray-300">
                              {quotaOutcome.notes.map((note) => (
                                <li key={note}>{note}</li>
                              ))}
                            </ul>
                          )}
                        </div>
                      )}
                      {!quotaLoading && !quotaOutcome && !quotaError && (
                        <span>Select an entitlement to evaluate notes.</span>
                      )}
                      {quotaError && <span className="text-red-400">{quotaError}</span>}
                    </div>
                  </div>
                ) : (
                  <p className="text-sm text-gray-400">
                    The selected plan does not define entitlements yet. Configure entitlement limits to enable quota checks.
                  </p>
                )}
              </div>
            </Card>
          </section>

          <section>
            <Card>
              <div className="space-y-2">
                <h3 className="text-lg font-semibold text-white">Current subscription</h3>
                <p className="text-sm text-gray-300">Status: {subscriptionStatus}</p>
                {subscription?.plan && (
                  <p className="text-sm text-gray-300">
                    Active plan: {subscription.plan.name} ({subscription.plan.code})
                  </p>
                )}
                {trialDescription && (
                  <p className="text-sm text-gray-300">Trial ends: {trialDescription}</p>
                )}
                {!subscription && (
                  <p className="text-sm text-gray-400">
                    No subscription is currently assigned. Select a plan to activate billing for this organization.
                  </p>
                )}
              </div>
            </Card>
          </section>
        </div>
      )}
    </div>
  );
}
