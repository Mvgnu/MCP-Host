-- SaaS subscription and billing scaffolding
-- key: migration-billing-foundations

BEGIN;

CREATE TABLE IF NOT EXISTS billing_plans (
    id UUID PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT,
    billing_period TEXT NOT NULL DEFAULT 'monthly',
    currency TEXT NOT NULL DEFAULT 'usd',
    amount_cents INTEGER NOT NULL DEFAULT 0,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS billing_plan_entitlements (
    id UUID PRIMARY KEY,
    plan_id UUID NOT NULL REFERENCES billing_plans(id) ON DELETE CASCADE,
    entitlement_key TEXT NOT NULL,
    limit_quantity BIGINT,
    reset_interval TEXT NOT NULL DEFAULT 'monthly',
    metadata JSONB NOT NULL DEFAULT '{}'::JSONB
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_billing_plan_entitlements_plan_key
    ON billing_plan_entitlements(plan_id, entitlement_key);

CREATE TABLE IF NOT EXISTS organization_subscriptions (
    id UUID PRIMARY KEY,
    organization_id INTEGER NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    plan_id UUID NOT NULL REFERENCES billing_plans(id),
    status TEXT NOT NULL DEFAULT 'active',
    trial_ends_at TIMESTAMPTZ,
    current_period_start TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    current_period_end TIMESTAMPTZ,
    canceled_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_org_subscriptions_org
    ON organization_subscriptions(organization_id);

CREATE INDEX IF NOT EXISTS idx_org_subscriptions_status
    ON organization_subscriptions(status);

CREATE TABLE IF NOT EXISTS subscription_usage_ledger (
    id UUID PRIMARY KEY,
    subscription_id UUID NOT NULL REFERENCES organization_subscriptions(id) ON DELETE CASCADE,
    entitlement_key TEXT NOT NULL,
    window_start TIMESTAMPTZ NOT NULL,
    window_end TIMESTAMPTZ NOT NULL,
    used_quantity BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(subscription_id, entitlement_key, window_start, window_end)
);

CREATE INDEX IF NOT EXISTS idx_usage_subscription_key
    ON subscription_usage_ledger(subscription_id, entitlement_key);

COMMIT;

-- Down

BEGIN;

DROP TABLE IF EXISTS subscription_usage_ledger;
DROP TABLE IF EXISTS organization_subscriptions;
DROP TABLE IF EXISTS billing_plan_entitlements;
DROP TABLE IF EXISTS billing_plans;

COMMIT;
