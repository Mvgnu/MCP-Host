-- Provider marketplace submission and evaluation scaffolding
-- key: migration-provider-marketplace

BEGIN;

CREATE TABLE IF NOT EXISTS provider_marketplace_submissions (
    id UUID PRIMARY KEY,
    provider_id UUID NOT NULL,
    submitted_by INTEGER,
    tier TEXT NOT NULL,
    manifest_uri TEXT NOT NULL,
    artifact_digest TEXT,
    release_notes TEXT,
    posture_state JSONB NOT NULL DEFAULT '{}'::JSONB,
    posture_vetoed BOOLEAN NOT NULL DEFAULT FALSE,
    posture_notes TEXT[] NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'pending',
    metadata JSONB NOT NULL DEFAULT '{}'::JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_provider_marketplace_submissions_provider
    ON provider_marketplace_submissions(provider_id);

CREATE INDEX IF NOT EXISTS idx_provider_marketplace_submissions_status
    ON provider_marketplace_submissions(status);

CREATE TABLE IF NOT EXISTS provider_marketplace_evaluations (
    id UUID PRIMARY KEY,
    submission_id UUID NOT NULL REFERENCES provider_marketplace_submissions(id) ON DELETE CASCADE,
    evaluation_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    evaluator_ref TEXT,
    result JSONB NOT NULL DEFAULT '{}'::JSONB,
    posture_state JSONB NOT NULL DEFAULT '{}'::JSONB,
    posture_vetoed BOOLEAN NOT NULL DEFAULT FALSE,
    posture_notes TEXT[] NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_provider_marketplace_evaluations_submission
    ON provider_marketplace_evaluations(submission_id);

CREATE INDEX IF NOT EXISTS idx_provider_marketplace_evaluations_status
    ON provider_marketplace_evaluations(status);

CREATE TABLE IF NOT EXISTS provider_marketplace_promotions (
    id UUID PRIMARY KEY,
    evaluation_id UUID NOT NULL REFERENCES provider_marketplace_evaluations(id) ON DELETE CASCADE,
    gate TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    opened_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    closed_at TIMESTAMPTZ,
    notes TEXT[] NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_provider_marketplace_promotions_evaluation
    ON provider_marketplace_promotions(evaluation_id);

CREATE INDEX IF NOT EXISTS idx_provider_marketplace_promotions_status
    ON provider_marketplace_promotions(status);

CREATE TABLE IF NOT EXISTS provider_marketplace_events (
    id UUID PRIMARY KEY,
    submission_id UUID,
    evaluation_id UUID,
    promotion_id UUID,
    actor_ref TEXT,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::JSONB,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_provider_marketplace_events_submission
    ON provider_marketplace_events(submission_id);

CREATE INDEX IF NOT EXISTS idx_provider_marketplace_events_evaluation
    ON provider_marketplace_events(evaluation_id);

CREATE INDEX IF NOT EXISTS idx_provider_marketplace_events_promotion
    ON provider_marketplace_events(promotion_id);

COMMIT;

-- Down

BEGIN;

DROP TABLE IF EXISTS provider_marketplace_events;
DROP TABLE IF EXISTS provider_marketplace_promotions;
DROP TABLE IF EXISTS provider_marketplace_evaluations;
DROP TABLE IF EXISTS provider_marketplace_submissions;

COMMIT;
