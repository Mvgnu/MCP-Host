'use client';

// key: onboarding-ui -> self-service-funnel

import { FormEvent, useCallback, useMemo, useState } from 'react';
import Alert from '../Alert';
import Button from '../Button';
import Card from '../Card';
import Input from '../Input';
import Spinner from '../Spinner';
import {
  BillingPlanCatalogEntry,
  fetchPlanCatalog,
  upsertSubscription,
} from '../../lib/billing';
import { loginUser, registerUser } from '../../lib/auth';
import {
  Invitation,
  OrgInfo,
  createInvitation,
  createOrganization,
  listInvitations,
} from '../../lib/organizations';

const STEP_ORDER = ['account', 'organization', 'plan', 'invites', 'complete'] as const;
type Step = (typeof STEP_ORDER)[number];

type Credentials = { email: string; password: string; confirm: string };
type OrganizationForm = { name: string };

type PlanState = {
  catalog: BillingPlanCatalogEntry[];
  selectedPlanId: string | null;
  enableTrial: boolean;
  trialDays: number;
  saving: boolean;
  error: string | null;
};

type InvitationState = {
  entries: Invitation[];
  email: string;
  saving: boolean;
  error: string | null;
  successMessage: string | null;
};

type Services = {
  registerUser: typeof registerUser;
  loginUser: typeof loginUser;
  createOrganization: typeof createOrganization;
  fetchPlanCatalog: typeof fetchPlanCatalog;
  upsertSubscription: typeof upsertSubscription;
  listInvitations: typeof listInvitations;
  createInvitation: typeof createInvitation;
};

const DEFAULT_SERVICES: Services = {
  registerUser,
  loginUser,
  createOrganization,
  fetchPlanCatalog,
  upsertSubscription,
  listInvitations,
  createInvitation,
};

const DEFAULT_PLAN_DAYS = 14;

export interface SelfServiceOnboardingProps {
  services?: Partial<Services>;
}

export default function SelfServiceOnboarding({
  services: overrides,
}: SelfServiceOnboardingProps) {
  const services: Services = useMemo(
    () => ({ ...DEFAULT_SERVICES, ...overrides }),
    [overrides],
  );
  const [step, setStep] = useState<Step>('account');
  const [accountForm, setAccountForm] = useState<Credentials>({
    email: '',
    password: '',
    confirm: '',
  });
  const [accountError, setAccountError] = useState<string | null>(null);
  const [accountSaving, setAccountSaving] = useState(false);

  const [organizationForm, setOrganizationForm] = useState<OrganizationForm>({
    name: '',
  });
  const [organization, setOrganization] = useState<OrgInfo | null>(null);
  const [organizationError, setOrganizationError] = useState<string | null>(null);
  const [organizationSaving, setOrganizationSaving] = useState(false);

  const [planState, setPlanState] = useState<PlanState>({
    catalog: [],
    selectedPlanId: null,
    enableTrial: true,
    trialDays: DEFAULT_PLAN_DAYS,
    saving: false,
    error: null,
  });

  const [invitationState, setInvitationState] = useState<InvitationState>({
    entries: [],
    email: '',
    saving: false,
    error: null,
    successMessage: null,
  });

  const [completionMessage, setCompletionMessage] = useState<string | null>(null);

  const canAdvance = useMemo(() => {
    switch (step) {
      case 'account':
        return (
          accountForm.email.trim().length > 0 &&
          accountForm.password.trim().length >= 8 &&
          accountForm.password === accountForm.confirm
        );
      case 'organization':
        return organizationForm.name.trim().length > 0;
      case 'plan':
        return planState.selectedPlanId !== null;
      case 'invites':
        return true;
      case 'complete':
        return false;
      default:
        return false;
    }
  }, [step, accountForm, organizationForm, planState.selectedPlanId]);

  const updatePlanCatalog = useCallback(async (orgId: number) => {
    setPlanState((prev) => ({ ...prev, error: null, saving: false }));
    try {
      const catalog = await services.fetchPlanCatalog();
      setPlanState((prev) => ({
        ...prev,
        catalog,
        selectedPlanId: catalog[0]?.plan.id ?? null,
      }));
      const invites = await services.listInvitations(orgId);
      setInvitationState((prev) => ({ ...prev, entries: invites }));
    } catch (err) {
      setPlanState((prev) => ({
        ...prev,
        error:
          err instanceof Error
            ? err.message
            : 'Failed to load plans for onboarding.',
      }));
    }
  }, [services]);

  const handleAccountSubmit = async (event: FormEvent) => {
    event.preventDefault();
    if (!canAdvance || accountSaving) {
      return;
    }
    if (!accountForm.email.includes('@')) {
      setAccountError('Please provide a valid email address.');
      return;
    }
    setAccountSaving(true);
    setAccountError(null);
    try {
      await services.registerUser({
        email: accountForm.email.trim(),
        password: accountForm.password,
      });
      await services.loginUser({
        email: accountForm.email.trim(),
        password: accountForm.password,
      });
      setStep('organization');
    } catch (err) {
      setAccountError(
        err instanceof Error
          ? err.message
          : 'We were unable to create your account.',
      );
    } finally {
      setAccountSaving(false);
    }
  };

  const handleOrganizationSubmit = async (event: FormEvent) => {
    event.preventDefault();
    if (!canAdvance || organizationSaving) {
      return;
    }
    setOrganizationSaving(true);
    setOrganizationError(null);
    try {
      const record = await services.createOrganization(
        organizationForm.name.trim(),
      );
      setOrganization(record);
      await updatePlanCatalog(record.id);
      setStep('plan');
    } catch (err) {
      setOrganizationError(
        err instanceof Error
          ? err.message
          : 'Failed to create organization. Please try again.',
      );
    } finally {
      setOrganizationSaving(false);
    }
  };

  const handlePlanSubmit = async (event: FormEvent) => {
    event.preventDefault();
    if (!organization || !planState.selectedPlanId || planState.saving) {
      return;
    }
    setPlanState((prev) => ({ ...prev, saving: true, error: null }));
    try {
      const payload: { plan_id: string; status?: string; trial_ends_at?: string } = {
        plan_id: planState.selectedPlanId,
      };
      if (planState.enableTrial) {
        const now = new Date();
        const ms = Math.max(1, planState.trialDays) * 24 * 60 * 60 * 1000;
        const trialEnds = new Date(now.getTime() + ms).toISOString();
        payload.status = 'trialing';
        payload.trial_ends_at = trialEnds;
      }
      await services.upsertSubscription(organization.id, payload);
      const invites = await services.listInvitations(organization.id);
      setInvitationState((prev) => ({ ...prev, entries: invites }));
      setCompletionMessage(
        'Plan assigned! You can invite teammates now or finish onboarding.',
      );
      setStep('invites');
    } catch (err) {
      setPlanState((prev) => ({
        ...prev,
        error:
          err instanceof Error
            ? err.message
            : 'We were unable to assign a plan.',
      }));
    } finally {
      setPlanState((prev) => ({ ...prev, saving: false }));
    }
  };

  const handleInvitationSubmit = async (event: FormEvent) => {
    event.preventDefault();
    if (!organization || invitationState.saving || !invitationState.email.trim()) {
      return;
    }
    setInvitationState((prev) => ({
      ...prev,
      saving: true,
      error: null,
      successMessage: null,
    }));
    try {
      const record = await services.createInvitation(
        organization.id,
        invitationState.email.trim(),
      );
      setInvitationState((prev) => ({
        ...prev,
        entries: [record, ...prev.entries],
        email: '',
        saving: false,
        successMessage: `Invitation created for ${record.email}. Share the token ${record.token} so they can join.`,
      }));
    } catch (err) {
      setInvitationState((prev) => ({
        ...prev,
        saving: false,
        error:
          err instanceof Error
            ? err.message
            : 'Failed to create invitation.',
      }));
    }
  };

  const finishOnboarding = () => {
    setStep('complete');
    setCompletionMessage(
      'All set! Your organization is ready. Teammates can join using the invitations above.',
    );
  };

  const planOptions = planState.catalog.map((entry) => ({
    id: entry.plan.id,
    label: `${entry.plan.name} — ${Intl.NumberFormat('en-US', {
      style: 'currency',
      currency: entry.plan.currency,
    }).format(entry.plan.amount_cents / 100)} / ${entry.plan.billing_period}`,
    description: entry.plan.description,
  }));

  const renderStepIndicator = () => (
    <ol className="flex flex-wrap gap-2 text-sm font-medium" aria-label="Onboarding steps">
      {STEP_ORDER.map((value) => {
        const index = STEP_ORDER.indexOf(value) + 1;
        const active = value === step;
        const complete = STEP_ORDER.indexOf(value) < STEP_ORDER.indexOf(step);
        return (
          <li
            key={value}
            className={`rounded px-2 py-1 ${
              active
                ? 'bg-blue-600 text-white'
                : complete
                ? 'bg-green-600 text-white'
                : 'bg-gray-200 text-gray-700'
            }`}
          >
            <span className="mr-1">{index}.</span>
            {value.charAt(0).toUpperCase() + value.slice(1)}
          </li>
        );
      })}
    </ol>
  );

  const renderAccountStep = () => (
    <form onSubmit={handleAccountSubmit} className="space-y-4">
      <Input
        label="Work Email"
        value={accountForm.email}
        onChange={(event) =>
          setAccountForm((prev) => ({ ...prev, email: event.target.value }))
        }
        type="email"
        required
      />
      <Input
        label="Password"
        value={accountForm.password}
        onChange={(event) =>
          setAccountForm((prev) => ({ ...prev, password: event.target.value }))
        }
        type="password"
        minLength={8}
        required
      />
      <Input
        label="Confirm Password"
        value={accountForm.confirm}
        onChange={(event) =>
          setAccountForm((prev) => ({ ...prev, confirm: event.target.value }))
        }
        type="password"
        minLength={8}
        required
      />
      {accountError ? <Alert tone="danger">{accountError}</Alert> : null}
      <Button type="submit" disabled={!canAdvance || accountSaving}>
        {accountSaving ? <Spinner size="sm" /> : 'Create account'}
      </Button>
    </form>
  );

  const renderOrganizationStep = () => (
    <form onSubmit={handleOrganizationSubmit} className="space-y-4">
      <Input
        label="Organization name"
        value={organizationForm.name}
        onChange={(event) =>
          setOrganizationForm({ name: event.target.value })
        }
        required
      />
      {organizationError ? <Alert tone="danger">{organizationError}</Alert> : null}
      <div className="flex items-center gap-3">
        <Button type="submit" disabled={!canAdvance || organizationSaving}>
          {organizationSaving ? <Spinner size="sm" /> : 'Create organization'}
        </Button>
      </div>
    </form>
  );

  const renderPlanStep = () => (
    <form onSubmit={handlePlanSubmit} className="space-y-4">
      <div className="space-y-3">
        {planOptions.map((option) => (
          <label
            key={option.id}
            className={`flex flex-col gap-1 rounded border p-3 ${
              planState.selectedPlanId === option.id
                ? 'border-blue-600 bg-blue-50'
                : 'border-gray-300'
            }`}
          >
            <span className="font-semibold">{option.label}</span>
            {option.description ? (
              <span className="text-sm text-gray-600">{option.description}</span>
            ) : null}
            <input
              type="radio"
              name="plan"
              value={option.id}
              checked={planState.selectedPlanId === option.id}
              onChange={() =>
                setPlanState((prev) => ({ ...prev, selectedPlanId: option.id }))
              }
              className="sr-only"
            />
          </label>
        ))}
        {planOptions.length === 0 ? (
          <Alert tone="warning">
            No active plans are available yet. Please contact support.
          </Alert>
        ) : null}
      </div>
      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={planState.enableTrial}
          onChange={(event) =>
            setPlanState((prev) => ({ ...prev, enableTrial: event.target.checked }))
          }
        />
        Offer a {planState.trialDays}-day trial period
      </label>
      {planState.enableTrial ? (
        <Input
          label="Trial length (days)"
          type="number"
          min={1}
          value={planState.trialDays.toString()}
          onChange={(event) =>
            setPlanState((prev) => ({
              ...prev,
              trialDays: Number.parseInt(event.target.value, 10) || DEFAULT_PLAN_DAYS,
            }))
          }
        />
      ) : null}
      {planState.error ? <Alert tone="danger">{planState.error}</Alert> : null}
      <Button type="submit" disabled={!canAdvance || planState.saving}>
        {planState.saving ? <Spinner size="sm" /> : 'Assign plan'}
      </Button>
    </form>
  );

  const renderInvitesStep = () => (
    <div className="space-y-6">
      {completionMessage ? <Alert tone="success">{completionMessage}</Alert> : null}
      <form onSubmit={handleInvitationSubmit} className="space-y-3">
        <Input
          label="Invite teammate"
          placeholder="teammate@example.com"
          value={invitationState.email}
          onChange={(event) =>
            setInvitationState((prev) => ({ ...prev, email: event.target.value }))
          }
          type="email"
        />
        {invitationState.error ? (
          <Alert tone="danger">{invitationState.error}</Alert>
        ) : null}
        {invitationState.successMessage ? (
          <Alert tone="info">{invitationState.successMessage}</Alert>
        ) : null}
        <Button type="submit" disabled={invitationState.saving}>
          {invitationState.saving ? <Spinner size="sm" /> : 'Send invitation'}
        </Button>
      </form>
      <div>
        <h3 className="mb-2 text-sm font-semibold uppercase tracking-wide text-gray-600">
          Pending invitations
        </h3>
        {invitationState.entries.length === 0 ? (
          <p className="text-sm text-gray-600">
            No invitations yet. Add teammates above to share access.
          </p>
        ) : (
          <ul className="space-y-2">
            {invitationState.entries.map((invite) => (
              <li key={invite.id} className="rounded border border-gray-200 p-3">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div>
                    <p className="font-medium">{invite.email}</p>
                    <p className="text-xs text-gray-600">
                      Status: {invite.status}{' '}
                      {invite.accepted_at
                        ? `at ${new Date(invite.accepted_at).toLocaleString()}`
                        : `— expires ${new Date(invite.expires_at).toLocaleDateString()}`}
                    </p>
                  </div>
                  <code className="rounded bg-gray-100 px-2 py-1 text-xs text-gray-700">
                    {invite.token}
                  </code>
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
      <div className="flex items-center gap-3">
        <Button onClick={finishOnboarding}>Finish onboarding</Button>
      </div>
    </div>
  );

  const renderCompleteStep = () => (
    <div className="space-y-4">
      <Alert tone="success">
        {completionMessage || 'You have completed the onboarding checklist.'}
      </Alert>
      <p className="text-sm text-gray-700">
        Keep your invitation tokens handy and remind teammates to register using the same
        email before redeeming their invite.
      </p>
    </div>
  );

  return (
    <Card className="mx-auto max-w-3xl space-y-6">
      <header className="space-y-2">
        <h1 className="text-2xl font-semibold">Get started with MCP SaaS</h1>
        <p className="text-sm text-gray-600">
          Create your workspace, pick a plan, and invite teammates without waiting on
          operator intervention.
        </p>
        {renderStepIndicator()}
      </header>
      <section>
        {step === 'account'
          ? renderAccountStep()
          : step === 'organization'
          ? renderOrganizationStep()
          : step === 'plan'
          ? renderPlanStep()
          : step === 'invites'
          ? renderInvitesStep()
          : renderCompleteStep()}
      </section>
    </Card>
  );
}
