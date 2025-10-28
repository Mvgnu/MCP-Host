-- key: migration -> remediation-accelerator-posture
CREATE TABLE IF NOT EXISTS runtime_vm_accelerator_posture (
    id BIGSERIAL PRIMARY KEY,
    runtime_vm_instance_id BIGINT NOT NULL REFERENCES runtime_vm_instances(id) ON DELETE CASCADE,
    accelerator_id TEXT NOT NULL,
    accelerator_type TEXT NOT NULL,
    posture TEXT NOT NULL,
    policy_feedback TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    metadata JSONB NOT NULL DEFAULT '{}'::JSONB,
    collected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (runtime_vm_instance_id, accelerator_id)
);

CREATE INDEX IF NOT EXISTS idx_runtime_vm_accelerator_posture_instance
    ON runtime_vm_accelerator_posture(runtime_vm_instance_id);

DROP TRIGGER IF EXISTS trg_runtime_vm_accelerator_posture_updated_at
    ON runtime_vm_accelerator_posture;
CREATE TRIGGER trg_runtime_vm_accelerator_posture_updated_at
BEFORE UPDATE ON runtime_vm_accelerator_posture
FOR EACH ROW
EXECUTE PROCEDURE set_updated_at();
