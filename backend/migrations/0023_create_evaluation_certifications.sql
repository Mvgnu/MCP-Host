-- Bind evaluation certifications to build artifacts and policy requirements
CREATE TABLE evaluation_certifications (
    id SERIAL PRIMARY KEY,
    build_artifact_run_id INTEGER NOT NULL REFERENCES build_artifact_runs(id) ON DELETE CASCADE,
    manifest_digest TEXT NOT NULL,
    tier TEXT NOT NULL,
    policy_requirement TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'pass', 'fail')),
    evidence JSONB,
    valid_from TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    valid_until TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX evaluation_certifications_digest_requirement_idx
    ON evaluation_certifications (manifest_digest, tier, policy_requirement);

CREATE INDEX evaluation_certifications_run_idx
    ON evaluation_certifications (build_artifact_run_id);

CREATE INDEX evaluation_certifications_status_idx
    ON evaluation_certifications (status);
