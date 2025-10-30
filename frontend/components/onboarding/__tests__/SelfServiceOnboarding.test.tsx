import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import SelfServiceOnboarding from '../SelfServiceOnboarding';
import { BillingPlanCatalogEntry } from '../../../lib/billing';

// key: onboarding-tests -> self-service-flow

describe('SelfServiceOnboarding', () => {
  const catalog: BillingPlanCatalogEntry[] = [
    {
      plan: {
        id: 'plan-basic',
        code: 'basic',
        name: 'Basic',
        description: 'Best for pilots',
        billing_period: 'monthly',
        currency: 'USD',
        amount_cents: 5000,
        active: true,
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
      },
      entitlements: [],
    },
  ];

  it('guides the user through account, organization, and plan assignment', async () => {
    const registerUser = jest.fn().mockResolvedValue(undefined);
    const loginUser = jest.fn().mockResolvedValue(undefined);
    const createOrganization = jest
      .fn()
      .mockResolvedValue({ id: 42, name: 'Test Org' });
    const fetchPlanCatalog = jest.fn().mockResolvedValue(catalog);
    const upsertSubscription = jest.fn().mockResolvedValue(undefined);
    const listInvitations = jest.fn().mockResolvedValue([]);
    const createInvitation = jest.fn();

    render(
      <SelfServiceOnboarding
        services={{
          registerUser,
          loginUser,
          createOrganization,
          fetchPlanCatalog,
          upsertSubscription,
          listInvitations,
          createInvitation,
        }}
      />,
    );

    fireEvent.change(screen.getByLabelText(/Work Email/i), {
      target: { value: 'owner@example.com' },
    });
    fireEvent.change(screen.getByLabelText(/^Password$/i), {
      target: { value: 'supersecret' },
    });
    fireEvent.change(screen.getByLabelText(/Confirm Password/i), {
      target: { value: 'supersecret' },
    });
    fireEvent.click(screen.getByRole('button', { name: /Create account/i }));

    await waitFor(() => expect(registerUser).toHaveBeenCalled());
    await waitFor(() => expect(loginUser).toHaveBeenCalled());
    expect(screen.getByLabelText(/Organization name/i)).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText(/Organization name/i), {
      target: { value: 'Test Org' },
    });
    fireEvent.click(screen.getByRole('button', { name: /Create organization/i }));

    await waitFor(() => expect(createOrganization).toHaveBeenCalled());
    await waitFor(() => expect(fetchPlanCatalog).toHaveBeenCalled());
    expect(await screen.findByText(/Basic —/i)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /Assign plan/i }));
    await waitFor(() => expect(upsertSubscription).toHaveBeenCalled());

    expect(
      await screen.findByRole('heading', { name: /Pending invitations/i }),
    ).toBeInTheDocument();
  });
});
