-- key: migration -> remediation-run-workspace-linkage
ALTER TABLE runtime_vm_remediation_runs
    ADD COLUMN IF NOT EXISTS workspace_id BIGINT REFERENCES runtime_vm_remediation_workspaces(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS workspace_revision_id BIGINT REFERENCES runtime_vm_remediation_workspace_revisions(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS promotion_gate_context JSONB NOT NULL DEFAULT '{}'::JSONB;

CREATE INDEX IF NOT EXISTS idx_runtime_vm_remediation_runs_workspace
    ON runtime_vm_remediation_runs(workspace_id);
CREATE INDEX IF NOT EXISTS idx_runtime_vm_remediation_runs_workspace_revision
    ON runtime_vm_remediation_runs(workspace_revision_id);
