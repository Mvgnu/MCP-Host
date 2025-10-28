-- key: migration -> remediation-workspace-lifecycle
CREATE TABLE IF NOT EXISTS runtime_vm_remediation_workspaces (
    id BIGSERIAL PRIMARY KEY,
    workspace_key TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    description TEXT,
    owner_id INTEGER NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    lifecycle_state TEXT NOT NULL DEFAULT 'draft',
    active_revision_id BIGINT,
    metadata JSONB NOT NULL DEFAULT '{}'::JSONB,
    lineage_tags TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    version BIGINT NOT NULL DEFAULT 0
);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'chk_runtime_vm_remediation_workspace_state'
            AND conrelid = 'runtime_vm_remediation_workspaces'::regclass
    ) THEN
        ALTER TABLE runtime_vm_remediation_workspaces
            ADD CONSTRAINT chk_runtime_vm_remediation_workspace_state
            CHECK (lifecycle_state IN ('draft', 'validated', 'simulated', 'promoted', 'archived'));
    END IF;
END;
$$;

CREATE TABLE IF NOT EXISTS runtime_vm_remediation_workspace_revisions (
    id BIGSERIAL PRIMARY KEY,
    workspace_id BIGINT NOT NULL REFERENCES runtime_vm_remediation_workspaces(id) ON DELETE CASCADE,
    revision_number BIGINT NOT NULL,
    previous_revision_id BIGINT REFERENCES runtime_vm_remediation_workspace_revisions(id) ON DELETE SET NULL,
    created_by INTEGER NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    plan JSONB NOT NULL,
    schema_status TEXT NOT NULL DEFAULT 'pending',
    schema_errors TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    policy_status TEXT NOT NULL DEFAULT 'pending',
    policy_veto_reasons TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    simulation_status TEXT NOT NULL DEFAULT 'not_requested',
    promotion_status TEXT NOT NULL DEFAULT 'not_requested',
    metadata JSONB NOT NULL DEFAULT '{}'::JSONB,
    lineage_labels TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    schema_validated_at TIMESTAMPTZ,
    policy_evaluated_at TIMESTAMPTZ,
    simulated_at TIMESTAMPTZ,
    promoted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    version BIGINT NOT NULL DEFAULT 0,
    UNIQUE (workspace_id, revision_number)
);

ALTER TABLE runtime_vm_remediation_workspaces
    ADD CONSTRAINT fk_runtime_vm_remediation_workspace_active_revision
    FOREIGN KEY (active_revision_id)
    REFERENCES runtime_vm_remediation_workspace_revisions(id)
    ON DELETE SET NULL;

DO $$
DECLARE
    constraint_name TEXT := 'chk_runtime_vm_remediation_workspace_revision_schema_status';
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = constraint_name
            AND conrelid = 'runtime_vm_remediation_workspace_revisions'::regclass
    ) THEN
        ALTER TABLE runtime_vm_remediation_workspace_revisions
            ADD CONSTRAINT chk_runtime_vm_remediation_workspace_revision_schema_status
            CHECK (schema_status IN ('pending', 'validating', 'succeeded', 'failed'));
    END IF;
END;
$$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'chk_runtime_vm_remediation_workspace_revision_policy_status'
            AND conrelid = 'runtime_vm_remediation_workspace_revisions'::regclass
    ) THEN
        ALTER TABLE runtime_vm_remediation_workspace_revisions
            ADD CONSTRAINT chk_runtime_vm_remediation_workspace_revision_policy_status
            CHECK (policy_status IN ('pending', 'evaluating', 'approved', 'vetoed'));
    END IF;
END;
$$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'chk_runtime_vm_remediation_workspace_revision_simulation_status'
            AND conrelid = 'runtime_vm_remediation_workspace_revisions'::regclass
    ) THEN
        ALTER TABLE runtime_vm_remediation_workspace_revisions
            ADD CONSTRAINT chk_runtime_vm_remediation_workspace_revision_simulation_status
            CHECK (simulation_status IN ('not_requested', 'pending', 'running', 'succeeded', 'failed'));
    END IF;
END;
$$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'chk_runtime_vm_remediation_workspace_revision_promotion_status'
            AND conrelid = 'runtime_vm_remediation_workspace_revisions'::regclass
    ) THEN
        ALTER TABLE runtime_vm_remediation_workspace_revisions
            ADD CONSTRAINT chk_runtime_vm_remediation_workspace_revision_promotion_status
            CHECK (promotion_status IN ('not_requested', 'pending', 'approved', 'rejected', 'completed'));
    END IF;
END;
$$;

CREATE TABLE IF NOT EXISTS runtime_vm_remediation_workspace_sandbox_executions (
    id BIGSERIAL PRIMARY KEY,
    workspace_revision_id BIGINT NOT NULL REFERENCES runtime_vm_remediation_workspace_revisions(id) ON DELETE CASCADE,
    simulator_kind TEXT NOT NULL,
    execution_state TEXT NOT NULL DEFAULT 'pending',
    requested_by INTEGER NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    gate_context JSONB NOT NULL DEFAULT '{}'::JSONB,
    diff_snapshot JSONB,
    metadata JSONB NOT NULL DEFAULT '{}'::JSONB,
    requested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    failure_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    version BIGINT NOT NULL DEFAULT 0
);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'chk_runtime_vm_remediation_workspace_sandbox_state'
            AND conrelid = 'runtime_vm_remediation_workspace_sandbox_executions'::regclass
    ) THEN
        ALTER TABLE runtime_vm_remediation_workspace_sandbox_executions
            ADD CONSTRAINT chk_runtime_vm_remediation_workspace_sandbox_state
            CHECK (execution_state IN ('pending', 'running', 'succeeded', 'failed', 'cancelled'));
    END IF;
END;
$$;

CREATE TABLE IF NOT EXISTS runtime_vm_remediation_workspace_validation_snapshots (
    id BIGSERIAL PRIMARY KEY,
    workspace_revision_id BIGINT NOT NULL REFERENCES runtime_vm_remediation_workspace_revisions(id) ON DELETE CASCADE,
    snapshot_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    gate_context JSONB NOT NULL DEFAULT '{}'::JSONB,
    notes TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata JSONB NOT NULL DEFAULT '{}'::JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    version BIGINT NOT NULL DEFAULT 0
);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'chk_runtime_vm_remediation_workspace_validation_status'
            AND conrelid = 'runtime_vm_remediation_workspace_validation_snapshots'::regclass
    ) THEN
        ALTER TABLE runtime_vm_remediation_workspace_validation_snapshots
            ADD CONSTRAINT chk_runtime_vm_remediation_workspace_validation_status
            CHECK (status IN ('pending', 'succeeded', 'failed'));
    END IF;
END;
$$;

CREATE INDEX IF NOT EXISTS idx_runtime_vm_remediation_workspace_revisions_workspace
    ON runtime_vm_remediation_workspace_revisions(workspace_id);

CREATE INDEX IF NOT EXISTS idx_runtime_vm_remediation_workspace_sandbox_revision
    ON runtime_vm_remediation_workspace_sandbox_executions(workspace_revision_id);

CREATE INDEX IF NOT EXISTS idx_runtime_vm_remediation_workspace_validation_revision
    ON runtime_vm_remediation_workspace_validation_snapshots(workspace_revision_id);

DROP TRIGGER IF EXISTS trg_runtime_vm_remediation_workspaces_updated_at ON runtime_vm_remediation_workspaces;
CREATE TRIGGER trg_runtime_vm_remediation_workspaces_updated_at
BEFORE UPDATE ON runtime_vm_remediation_workspaces
FOR EACH ROW
EXECUTE PROCEDURE set_updated_at();

DROP TRIGGER IF EXISTS trg_runtime_vm_remediation_workspace_revisions_updated_at ON runtime_vm_remediation_workspace_revisions;
CREATE TRIGGER trg_runtime_vm_remediation_workspace_revisions_updated_at
BEFORE UPDATE ON runtime_vm_remediation_workspace_revisions
FOR EACH ROW
EXECUTE PROCEDURE set_updated_at();

DROP TRIGGER IF EXISTS trg_runtime_vm_remediation_workspace_sandbox_updated_at ON runtime_vm_remediation_workspace_sandbox_executions;
CREATE TRIGGER trg_runtime_vm_remediation_workspace_sandbox_updated_at
BEFORE UPDATE ON runtime_vm_remediation_workspace_sandbox_executions
FOR EACH ROW
EXECUTE PROCEDURE set_updated_at();

DROP TRIGGER IF EXISTS trg_runtime_vm_remediation_workspace_validation_updated_at ON runtime_vm_remediation_workspace_validation_snapshots;
CREATE TRIGGER trg_runtime_vm_remediation_workspace_validation_updated_at
BEFORE UPDATE ON runtime_vm_remediation_workspace_validation_snapshots
FOR EACH ROW
EXECUTE PROCEDURE set_updated_at();
