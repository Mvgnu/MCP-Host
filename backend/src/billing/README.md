# Billing Module

`key: billing-module -> subscriptions`

The billing module provides SaaS commercialization scaffolding for MCP Host. Key responsibilities:

- Manage normalized billing tables introduced in migration `0044_billing_foundations.sql` (`billing_plans`, `billing_plan_entitlements`, `organization_subscriptions`, and `subscription_usage_ledger`).
- Offer a `BillingService` facade for subscription lifecycle management, entitlement quota enforcement, and usage ledger upserts.
- Expose REST handlers (`api.rs`) for plan discovery, subscription bootstrap/update, and quota evaluation.
- Serve plan catalog metadata (plans plus entitlements) through `/api/billing/catalog` so operator tooling can render plan
  comparisons without duplicating SQL joins.
- Define provider adapter traits (`adapters.rs`) so external billing systems (Stripe-like) can reconcile customer, subscription, and usage state.
- Run an async reconciliation worker (`reconciliation.rs`) that consumes provider callbacks, normalizes usage payloads, and settles them through `BillingService::settle_usage` into `subscription_usage_ledger`.
- Automate renewal posture via the scheduler (`scheduler.rs`), which marks overdue accounts, downgrades to the optional fallback plan (`BILLING_FALLBACK_PLAN_CODE`), or suspends subscriptions after `BILLING_PAST_DUE_GRACE_DAYS` using a configurable tick interval (`BILLING_RENEWAL_SCAN_INTERVAL_SECS`).

`BillingService::enforce_quota` returns structured `BillingQuotaOutcome` payloads that runtime policy and operator tooling use to annotate decisions with `billing:*` notes. The service deduplicates usage by entitlement window (daily/weekly/monthly) and emits compliance-friendly audit notes without expanding telemetry beyond subscription health. When providers flag overages, operators can call `BillingService::mark_subscription_overdue`, `BillingService::suspend_subscription`, or `BillingService::downgrade_subscription` to steer organizations back into compliant states without bypassing runtime quota gates.

Integration tests in `backend/tests/billing.rs` confirm quota ledger behavior for mixed entitlement plans and surface actionable veto messaging when subscriptions lapse. Renewal automation coverage lives in `backend/tests/billing_scheduler.rs`, ensuring overdue detection, fallback downgrades, and suspension semantics function against the full migration stack. Use `cargo test --package backend --test billing -- --ignored` and `cargo test --package backend --test billing_scheduler -- --ignored` with a provisioned Postgres database to execute the SQLx-backed scenarios end-to-end.
