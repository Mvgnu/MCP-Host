# Billing Module

`key: billing-module -> subscriptions`

The billing module provides SaaS commercialization scaffolding for MCP Host. Key responsibilities:

- Manage normalized billing tables introduced in migration `0044_billing_foundations.sql` (`billing_plans`, `billing_plan_entitlements`, `organization_subscriptions`, and `subscription_usage_ledger`).
- Offer a `BillingService` facade for subscription lifecycle management, entitlement quota enforcement, and usage ledger upserts.
- Expose REST handlers (`api.rs`) for plan discovery, subscription bootstrap/update, and quota evaluation.
- Define provider adapter traits (`adapters.rs`) so external billing systems (Stripe-like) can reconcile customer and subscription state.

`BillingService::enforce_quota` returns structured `BillingQuotaOutcome` payloads that runtime policy and operator tooling use to annotate decisions with `billing:*` notes. The service deduplicates usage by entitlement window (daily/weekly/monthly) and emits compliance-friendly audit notes without expanding telemetry beyond subscription health.
