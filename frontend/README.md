# MCP Host Frontend

This Next.js application powers the operator console, provider marketplace, and customer-facing onboarding surfaces for the MCP Host platform.

## Getting Started

Install dependencies:

```bash
npm install
```

Run the development server:

```bash
npm run dev
```

Open [http://localhost:3000](http://localhost:3000) with your browser to see the result.

The project uses the Next.js App Router. Pages live under `app/` and load client components as needed.

## Feature surfaces

### Billing onboarding wizard

- Navigate to `/console/billing/<organizationId>` to launch the operator subscription wizard. The page consumes the billing plan catalog (`/api/billing/catalog`), active subscription envelope, and quota evaluation endpoint so operators can compare plans, toggle trials, and surface `BillingQuotaOutcome` notes without recording usage.
- Plan comparisons reuse the `BillingPlanComparison` component to highlight entitlement limits and metadata sourced from `billing_plan_entitlements.metadata` JSON.
- Trial activations default to 14 days but can be tuned per organization; the wizard computes ISO timestamps for `BillingService::upsert_subscription` so operators can stage or extend trials before converting to an active subscription.
- Entitlement selectors automatically invoke `/api/billing/organizations/:id/quotas/check` with `record_usage=false` to render billing notes, helping operators validate downgrade, suspension, or overage scenarios prior to runtime enforcement changes.

### Self-service onboarding portal

- Navigate to `/onboarding` to create an account, bootstrap an organization, assign a billing plan, and invite teammates without relying on operator actions. The route renders the `SelfServiceOnboarding` multi-step component.
- Registration posts to `/api/register` then immediately authenticates against `/api/login`, wiring cookies for subsequent organization and billing calls.
- Organization creation persists through `/api/orgs`, fetches the billing plan catalog, and advances to plan selection with optional trial windows. Successful plan assignments call `/api/billing/organizations/:id/subscription`.
- Teammate invitations persist through `/api/orgs/:id/invitations`, listing pending invites with shareable tokens so invitees can accept via `/api/orgs/invitations/:token/accept` after registering with the matching email.
- Invitees can redeem their token at `/onboarding/invite/[token]`, which renders `AcceptInvitation` to guide registration or sign-in before calling the acceptance API.

### Provider marketplace dashboard

- Navigate to `/console/marketplace/providerdashboard` to review provider submissions, posture badges, evaluation timelines, and promotion gates. The page consumes `/api/marketplace/providers/:provider_id/submissions` for bootstrapping and listens to the SSE stream at `/api/marketplace/providers/:provider_id/events/stream` to refresh cards without manual reloads.
- Artifact uploads reuse the console form controls and invoke `/api/marketplace/providers/:provider_id/submissions`, surfacing submission errors inline so operators can correct posture issues before retrying.

### Federated vector DB governance

- Navigate to `/console/vector-dbs/governance` to administer residency policies, attachments, and incident logs for managed vector databases. The console bootstraps data from `/api/vector-dbs` and scoped residency/attachment/incident endpoints, reloading sections after each control action.
- The governance page reuses `VectorDbResidencyCard`, `VectorDbAttachmentList`, and `VectorDbIncidentTimeline` components to capture residency posture, credential rotation prompts, and incident remediation state.
- Attachment creation validates JSON metadata locally before calling `/api/vector-dbs/:id/attachments`, while incident logging persists remediation context through `/api/vector-dbs/:id/incidents` and resolves incidents via the new PATCH handler.

### Provider BYOK staging

- Shared helper stubs in `frontend/lib/byok.ts` define the provider key contract (`ProviderKeyRecord`, posture helpers) and currently throw a descriptive error until the backend endpoints ship. Console and provider portal features should consume these helpers so the contract remains centralized.
- `ProviderKeyDecisionPosture` mirrors the backend `key_posture` payload persisted with runtime policy decisions so SSE consumers can render BYOK state (active/rotating, rotation deadlines, signature verification posture, veto notes) without duplicating contract mapping.
- Jest coverage in `frontend/lib/byok.test.ts` asserts rotation posture helpers behave deterministically before UI wiring lands.

### Testing & Coverage

- Install dependencies with `npm install`. The suite pins `@testing-library/react@^16.3.0` so React 19 console components and analytics fixtures compile without peer dependency warnings.
- Run unit tests with coverage via `npm test -- --coverage`. The suite is configured through `tsconfig.jest.json` so Jest can compile the lifecycle console components and their JSX fixtures.
- Lifecycle analytics regression coverage exercises retry ledgers, manual overrides, promotion verdict chips, and the Zustand-powered server store. These tests mirror the backend lifecycle snapshots so console renderers stay aligned with the analytics payload contract.

This project uses [`next/font`](https://nextjs.org/docs/app/building-your-application/optimizing/fonts) to automatically optimize and load [Geist](https://vercel.com/font), a new font family for Vercel.

## Learn More

To learn more about Next.js, take a look at the following resources:

- [Next.js Documentation](https://nextjs.org/docs) - learn about Next.js features and API.
- [Learn Next.js](https://nextjs.org/learn) - an interactive Next.js tutorial.

You can check out [the Next.js GitHub repository](https://github.com/vercel/next.js) - your feedback and contributions are welcome!

## Deploy on Vercel

The easiest way to deploy your Next.js app is to use the [Vercel Platform](https://vercel.com/new?utm_medium=default-template&filter=next.js&utm_source=create-next-app&utm_campaign=create-next-app-readme) from the creators of Next.js.

Check out our [Next.js deployment documentation](https://nextjs.org/docs/app/building-your-application/deploying) for more details.
