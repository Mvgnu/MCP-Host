-- Provider BYOK runtime policy enforcement scaffolding
CREATE TABLE IF NOT EXISTS provider_tiers (
    tier TEXT PRIMARY KEY,
    provider_id UUID NOT NULL,
    byok_required BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_provider_tiers_provider ON provider_tiers(provider_id);

ALTER TABLE runtime_policy_decisions
    ADD COLUMN IF NOT EXISTS key_posture JSONB;

-- downgrade
ALTER TABLE runtime_policy_decisions
    DROP COLUMN IF EXISTS key_posture;

DROP TABLE IF EXISTS provider_tiers;
