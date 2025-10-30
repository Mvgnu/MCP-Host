'use client';

// key: billing-console-ui -> plan-comparison

import clsx from 'clsx';
import type { BillingPlanCatalogEntry } from '../../lib/billing';

interface BillingPlanComparisonProps {
  plans: BillingPlanCatalogEntry[];
  selectedPlanId?: string | null;
  onSelect: (planId: string) => void;
}

function formatAmount(plan: BillingPlanCatalogEntry['plan']) {
  const dollars = (plan.amount_cents / 100).toFixed(2);
  return `${plan.currency.toUpperCase()} $${dollars}/${plan.billing_period}`;
}

export default function BillingPlanComparison({
  plans,
  selectedPlanId,
  onSelect,
}: BillingPlanComparisonProps) {
  if (!plans.length) {
    return (
      <div className="rounded border border-dashed border-gray-600 p-6 text-sm text-gray-400">
        No billing plans are available. Configure plans in the billing admin to continue.
      </div>
    );
  }

  return (
    <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
      {plans.map(({ plan, entitlements }) => {
        const selected = plan.id === selectedPlanId;
        return (
          <button
            key={plan.id}
            type="button"
            onClick={() => onSelect(plan.id)}
            className={clsx(
              'text-left transition focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400',
            )}
            data-testid={`billing-plan-${plan.code}`}
          >
            <div
              className={clsx(
                'h-full rounded border p-4 shadow-sm',
                selected
                  ? 'border-blue-500 bg-blue-950/40'
                  : 'border-gray-700 bg-gray-800 hover:border-blue-400',
              )}
            >
              <div className="flex items-start justify-between gap-2">
                <div>
                  <h3 className="text-lg font-semibold text-white">{plan.name}</h3>
                  <p className="text-sm text-gray-400">{formatAmount(plan)}</p>
                </div>
                {selected && (
                  <span className="rounded-full bg-blue-500 px-3 py-1 text-xs font-semibold text-white">
                    Selected
                  </span>
                )}
              </div>
              {plan.description && (
                <p className="mt-3 text-sm text-gray-300">{plan.description}</p>
              )}
              <div className="mt-4 space-y-2 text-sm text-gray-200">
                <p className="font-medium uppercase tracking-wide text-gray-400">Entitlements</p>
                <ul className="space-y-1">
                  {entitlements.map((entitlement) => {
                    const limit =
                      entitlement.limit_quantity != null
                        ? `Limit: ${entitlement.limit_quantity}/${entitlement.reset_interval}`
                        : 'Unlimited';
                    const label =
                      typeof entitlement.metadata.label === 'string'
                        ? entitlement.metadata.label
                        : entitlement.entitlement_key;
                    return (
                      <li
                        key={`${plan.id}-${entitlement.entitlement_key}`}
                        className="rounded border border-gray-700 bg-gray-900/40 px-3 py-2"
                      >
                        <span className="block text-sm font-semibold text-white">{label}</span>
                        <span className="block text-xs text-gray-400">{limit}</span>
                      </li>
                    );
                  })}
                </ul>
              </div>
            </div>
          </button>
        );
      })}
    </div>
  );
}
