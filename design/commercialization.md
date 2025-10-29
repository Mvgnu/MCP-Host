# Commercialization Foundations

`key: commercialization-doc -> billing,subscriptions`

## Objectives

- Provide a minimal-yet-complete SaaS onboarding loop: define plans, attach subscriptions to organizations, and expose entitlement checks to runtime policy.
- Keep telemetry surface area narrow by recording only entitlement usage windows required for compliance.
- Support pluggable billing providers while defaulting to a stub adapter for offline development.

## Data Model

Migration `0044_billing_foundations.sql` introduces four core tables:

| Table | Purpose |
| --- | --- |
| `billing_plans` | Catalog of sellable plans (code, price, billing period, active flag). |
| `billing_plan_entitlements` | Per-plan entitlements with optional quantity limits and reset cadence. |
| `organization_subscriptions` | Active subscription metadata for each organization, including trial windows and current period boundaries. |
| `subscription_usage_ledger` | Aggregated entitlement consumption per subscription + window (daily/weekly/monthly) to support quota checks and audit exports. |

## Service Layer

`BillingService` centralizes subscription lifecycle, quota enforcement, and usage ledger upserts. Highlights:

- `active_subscription` fetches the latest subscription + plan for runtime policy, returning `None` when status is not `active`/`trialing` or the period expired.
- `enforce_quota` evaluates requested usage against plan entitlements, returning a structured `BillingQuotaOutcome` with `billing:*` notes for downstream UX. When `record_usage` is true, the ledger increments atomically within the entitlement window.
- `upsert_subscription` seeds bootstrap flows while preserving optimistic locking via the subscription ID.

## API Surface

Handlers in `backend/src/billing/api.rs` expose:

- `GET /api/billing/plans` for plan discovery.
- `GET/POST /api/billing/organizations/:id/subscription` for onboarding and plan changes.
- `POST /api/billing/organizations/:id/quotas/check` to evaluate entitlements (optionally recording usage).

CLI parity arrives via `mcpctl billing ...` commands so operators can bootstrap tenants, inspect plan posture, and sanity-check quotas without touching SQL.

## Policy Integration

Runtime policy now calls `BillingService::enforce_quota` when computing placement decisions. Failures produce `billing:subscription-missing`, `billing:quota-exceeded:*`, or `billing:error:*` notes and flip `evaluation_required`/`governance_required` flags to block launches until entitlements are restored.

## Provider Integration

`StripeLikeAdapter` demonstrates how billing providers can map webhook payloads into `BillingService::upsert_subscription`. A future job worker will reconcile usage deltas by invoking `record_usage` with provider-reported metering data.
