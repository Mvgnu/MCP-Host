-- Promotion release trains linking governance workflows and runtime gating
CREATE TYPE promotion_status AS ENUM ('scheduled', 'in_progress', 'approved', 'active', 'rolled_back');

CREATE TABLE promotion_tracks (
    id SERIAL PRIMARY KEY,
    owner_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    tier TEXT NOT NULL,
    stages TEXT[] NOT NULL DEFAULT ARRAY['candidate','staging','production']::TEXT[],
    description TEXT,
    workflow_id INTEGER REFERENCES governance_workflows(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (owner_id, name)
);

CREATE TABLE artifact_promotions (
    id BIGSERIAL PRIMARY KEY,
    promotion_track_id INTEGER NOT NULL REFERENCES promotion_tracks(id) ON DELETE CASCADE,
    manifest_digest TEXT NOT NULL,
    artifact_run_id INTEGER REFERENCES build_artifact_runs(id) ON DELETE SET NULL,
    stage TEXT NOT NULL,
    status promotion_status NOT NULL DEFAULT 'scheduled',
    scheduled_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
    approved_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
    workflow_run_id BIGINT REFERENCES governance_workflow_runs(id) ON DELETE SET NULL,
    notes TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    scheduled_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    approved_at TIMESTAMPTZ,
    activated_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (promotion_track_id, stage, manifest_digest)
);

CREATE INDEX idx_promotion_tracks_owner ON promotion_tracks(owner_id);
CREATE INDEX idx_artifact_promotions_manifest ON artifact_promotions(manifest_digest);
CREATE INDEX idx_artifact_promotions_status ON artifact_promotions(status);

ALTER TABLE governance_workflow_runs
    ADD COLUMN promotion_track_id INTEGER REFERENCES promotion_tracks(id) ON DELETE SET NULL,
    ADD COLUMN promotion_stage TEXT;

ALTER TABLE runtime_policy_decisions
    ADD COLUMN promotion_track_id INTEGER REFERENCES promotion_tracks(id) ON DELETE SET NULL,
    ADD COLUMN promotion_stage TEXT,
    ADD COLUMN promotion_status TEXT,
    ADD COLUMN promotion_notes TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[];
