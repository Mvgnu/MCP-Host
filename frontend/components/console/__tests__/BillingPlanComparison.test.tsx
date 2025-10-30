import { fireEvent, render, screen } from '@testing-library/react';
import BillingPlanComparison from '../BillingPlanComparison';
import type { BillingPlanCatalogEntry } from '../../../lib/billing';

describe('BillingPlanComparison', () => {
  function buildPlan(
    overrides: Partial<BillingPlanCatalogEntry> = {},
  ): BillingPlanCatalogEntry {
    const plan: BillingPlanCatalogEntry = {
      plan: {
        id: 'plan-basic',
        code: 'basic',
        name: 'Basic',
        description: 'Entry plan',
        billing_period: 'monthly',
        currency: 'usd',
        amount_cents: 9900,
        active: true,
        created_at: '2024-01-01T00:00:00.000Z',
        updated_at: '2024-01-01T00:00:00.000Z',
      },
      entitlements: [
        {
          id: 'ent-1',
          plan_id: 'plan-basic',
          entitlement_key: 'requests',
          limit_quantity: 1000,
          reset_interval: 'monthly',
          metadata: { label: 'API Requests' },
        },
      ],
    };
    return { ...plan, ...overrides };
  }

  it('renders a callout when no plans exist', () => {
    render(
      <BillingPlanComparison plans={[]} selectedPlanId={null} onSelect={() => {}} />,
    );

    expect(screen.getByText(/No billing plans are available/)).toBeInTheDocument();
  });

  it('highlights the selected plan and surfaces entitlement labels', () => {
    const plans = [
      buildPlan(),
      buildPlan({ plan: { ...buildPlan().plan, id: 'plan-pro', code: 'pro', name: 'Pro', amount_cents: 29900 } }),
    ];
    render(
      <BillingPlanComparison
        plans={plans}
        selectedPlanId="plan-pro"
        onSelect={() => {}}
      />,
    );

    const selectedBadge = screen.getByText('Selected');
    expect(selectedBadge).toBeInTheDocument();
    const entitlementLabels = screen.getAllByText('API Requests');
    expect(entitlementLabels.length).toBeGreaterThan(0);
    const selectedCard = screen.getByTestId('billing-plan-pro');
    expect(selectedCard.firstElementChild).toHaveClass('border-blue-500');
  });

  it('invokes onSelect when a plan is clicked', () => {
    const handleSelect = jest.fn();
    const plans = [buildPlan()];
    render(
      <BillingPlanComparison
        plans={plans}
        selectedPlanId={null}
        onSelect={handleSelect}
      />,
    );

    fireEvent.click(screen.getByTestId('billing-plan-basic'));
    expect(handleSelect).toHaveBeenCalledWith('plan-basic');
  });
});
