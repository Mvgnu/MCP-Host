-- key: migration -> remediation-orchestrator
CREATE TABLE IF NOT EXISTS runtime_vm_remediation_runs (
    id BIGSERIAL PRIMARY KEY,
    runtime_vm_instance_id BIGINT NOT NULL REFERENCES runtime_vm_instances(id) ON DELETE CASCADE,
    playbook TEXT NOT NULL,
    status TEXT NOT NULL,
    automation_payload JSONB,
    approval_required BOOLEAN NOT NULL DEFAULT FALSE,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    last_error TEXT
);

CREATE INDEX IF NOT EXISTS idx_runtime_vm_remediation_runs_instance
    ON runtime_vm_remediation_runs(runtime_vm_instance_id);
CREATE INDEX IF NOT EXISTS idx_runtime_vm_remediation_runs_status
    ON runtime_vm_remediation_runs(status);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'chk_runtime_vm_remediation_status'
            AND conrelid = 'runtime_vm_remediation_runs'::regclass
    ) THEN
        ALTER TABLE runtime_vm_remediation_runs
            ADD CONSTRAINT chk_runtime_vm_remediation_status
            CHECK (status IN ('running', 'completed', 'failed'));
    END IF;
END;
$$;

-- enforce a single running run per VM instance
CREATE UNIQUE INDEX IF NOT EXISTS uq_runtime_vm_remediation_running
    ON runtime_vm_remediation_runs(runtime_vm_instance_id)
    WHERE status = 'running';
