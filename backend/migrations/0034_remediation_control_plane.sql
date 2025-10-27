-- key: migration -> remediation-control-plane
CREATE TABLE IF NOT EXISTS runtime_vm_remediation_playbooks (
    id BIGSERIAL PRIMARY KEY,
    playbook_key TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    description TEXT,
    executor_type TEXT NOT NULL,
    owner_id INTEGER NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    approval_required BOOLEAN NOT NULL DEFAULT FALSE,
    sla_duration_seconds INTEGER,
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
        WHERE conname = 'chk_runtime_vm_remediation_executor_type'
            AND conrelid = 'runtime_vm_remediation_playbooks'::regclass
    ) THEN
        ALTER TABLE runtime_vm_remediation_playbooks
            ADD CONSTRAINT chk_runtime_vm_remediation_executor_type
            CHECK (executor_type IN ('shell', 'ansible', 'cloud_api'));
    END IF;
END;
$$;

DROP TRIGGER IF EXISTS trg_runtime_vm_remediation_playbooks_updated_at ON runtime_vm_remediation_playbooks;
CREATE TRIGGER trg_runtime_vm_remediation_playbooks_updated_at
BEFORE UPDATE ON runtime_vm_remediation_playbooks
FOR EACH ROW
EXECUTE PROCEDURE set_updated_at();

ALTER TABLE runtime_vm_remediation_runs
    ADD COLUMN IF NOT EXISTS playbook_id BIGINT REFERENCES runtime_vm_remediation_playbooks(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS assigned_owner_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS sla_deadline TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS approval_state TEXT NOT NULL DEFAULT 'pending',
    ADD COLUMN IF NOT EXISTS approval_decided_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS approval_notes TEXT,
    ADD COLUMN IF NOT EXISTS metadata JSONB NOT NULL DEFAULT '{}'::JSONB,
    ADD COLUMN IF NOT EXISTS version BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ADD COLUMN IF NOT EXISTS cancelled_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS cancellation_reason TEXT,
    ADD COLUMN IF NOT EXISTS failure_reason TEXT;

UPDATE runtime_vm_remediation_runs
SET approval_state = CASE
        WHEN approval_required THEN 'pending'
        ELSE 'auto-approved'
    END
WHERE approval_state = 'pending';

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'chk_runtime_vm_remediation_status'
            AND conrelid = 'runtime_vm_remediation_runs'::regclass
    ) THEN
        ALTER TABLE runtime_vm_remediation_runs
            DROP CONSTRAINT chk_runtime_vm_remediation_status;
    END IF;
END;
$$;

ALTER TABLE runtime_vm_remediation_runs
    ADD CONSTRAINT chk_runtime_vm_remediation_status
    CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled'));

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'chk_runtime_vm_remediation_approval_state'
            AND conrelid = 'runtime_vm_remediation_runs'::regclass
    ) THEN
        ALTER TABLE runtime_vm_remediation_runs
            ADD CONSTRAINT chk_runtime_vm_remediation_approval_state
            CHECK (approval_state IN ('pending', 'approved', 'rejected', 'auto-approved'));
    END IF;
END;
$$;

DROP INDEX IF EXISTS idx_runtime_vm_remediation_runs_status;
CREATE INDEX idx_runtime_vm_remediation_runs_status
    ON runtime_vm_remediation_runs(status, approval_state);

CREATE INDEX IF NOT EXISTS idx_runtime_vm_remediation_runs_playbook
    ON runtime_vm_remediation_runs(playbook_id);

CREATE INDEX IF NOT EXISTS idx_runtime_vm_remediation_runs_owner
    ON runtime_vm_remediation_runs(assigned_owner_id);

DROP TRIGGER IF EXISTS trg_runtime_vm_remediation_runs_updated_at ON runtime_vm_remediation_runs;
CREATE TRIGGER trg_runtime_vm_remediation_runs_updated_at
BEFORE UPDATE ON runtime_vm_remediation_runs
FOR EACH ROW
EXECUTE PROCEDURE set_updated_at();

CREATE TABLE IF NOT EXISTS runtime_vm_remediation_artifacts (
    id BIGSERIAL PRIMARY KEY,
    remediation_run_id BIGINT NOT NULL REFERENCES runtime_vm_remediation_runs(id) ON DELETE CASCADE,
    artifact_type TEXT NOT NULL,
    uri TEXT,
    metadata JSONB NOT NULL DEFAULT '{}'::JSONB,
    recorded_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_runtime_vm_remediation_artifacts_run
    ON runtime_vm_remediation_artifacts(remediation_run_id);
