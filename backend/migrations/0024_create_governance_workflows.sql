-- Governance workflows orchestrate promotions, rollbacks, and credential rotations
CREATE TYPE governance_workflow_kind AS ENUM ('promotion', 'rollback', 'credential_rotation');
CREATE TYPE governance_run_status AS ENUM ('pending', 'in_progress', 'completed', 'failed', 'cancelled');
CREATE TYPE governance_step_status AS ENUM ('pending', 'in_progress', 'completed', 'failed', 'blocked');

CREATE TABLE governance_workflows (
    id SERIAL PRIMARY KEY,
    owner_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    workflow_type governance_workflow_kind NOT NULL,
    tier TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE governance_workflow_steps (
    id SERIAL PRIMARY KEY,
    workflow_id INTEGER NOT NULL REFERENCES governance_workflows(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    action TEXT NOT NULL,
    config JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE governance_workflow_runs (
    id BIGSERIAL PRIMARY KEY,
    workflow_id INTEGER NOT NULL REFERENCES governance_workflows(id) ON DELETE CASCADE,
    initiated_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
    target_artifact_run_id INTEGER REFERENCES build_artifact_runs(id) ON DELETE SET NULL,
    target_manifest_digest TEXT,
    target_tier TEXT NOT NULL,
    status governance_run_status NOT NULL DEFAULT 'pending',
    notes TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    policy_decision_id INTEGER REFERENCES runtime_policy_decisions(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE governance_step_runs (
    id BIGSERIAL PRIMARY KEY,
    workflow_run_id BIGINT NOT NULL REFERENCES governance_workflow_runs(id) ON DELETE CASCADE,
    step_id INTEGER REFERENCES governance_workflow_steps(id) ON DELETE SET NULL,
    status governance_step_status NOT NULL DEFAULT 'pending',
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    error TEXT
);

CREATE TABLE governance_audit_logs (
    id BIGSERIAL PRIMARY KEY,
    workflow_run_id BIGINT NOT NULL REFERENCES governance_workflow_runs(id) ON DELETE CASCADE,
    actor_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    event_type TEXT NOT NULL,
    details JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_governance_workflows_owner ON governance_workflows(owner_id);
CREATE INDEX idx_governance_workflows_tier ON governance_workflows(tier);
CREATE INDEX idx_governance_workflow_runs_status ON governance_workflow_runs(status);
CREATE INDEX idx_governance_workflow_runs_target ON governance_workflow_runs(target_manifest_digest, target_tier);
CREATE INDEX idx_governance_step_runs_run ON governance_step_runs(workflow_run_id);
CREATE INDEX idx_governance_audit_logs_run ON governance_audit_logs(workflow_run_id);

ALTER TABLE runtime_policy_decisions
    ADD COLUMN governance_required BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN governance_run_id BIGINT REFERENCES governance_workflow_runs(id);
