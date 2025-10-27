-- key: migration -> runtime-vm-trust-history
CREATE TABLE IF NOT EXISTS runtime_vm_trust_history (
    id BIGSERIAL PRIMARY KEY,
    runtime_vm_instance_id BIGINT NOT NULL REFERENCES runtime_vm_instances(id) ON DELETE CASCADE,
    attestation_id BIGINT REFERENCES runtime_vm_attestations(id) ON DELETE SET NULL,
    previous_status TEXT,
    current_status TEXT NOT NULL,
    transition_reason TEXT,
    remediation_state TEXT,
    triggered_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_runtime_vm_trust_history_instance
    ON runtime_vm_trust_history(runtime_vm_instance_id DESC, triggered_at DESC);

CREATE INDEX IF NOT EXISTS idx_runtime_vm_trust_history_status
    ON runtime_vm_trust_history(current_status);

DROP FUNCTION IF EXISTS notify_runtime_vm_trust_transition();
CREATE FUNCTION notify_runtime_vm_trust_transition()
RETURNS TRIGGER AS $$
DECLARE
    payload JSON;
BEGIN
    payload := json_build_object(
        'runtime_vm_instance_id', NEW.runtime_vm_instance_id,
        'attestation_id', NEW.attestation_id,
        'previous_status', NEW.previous_status,
        'current_status', NEW.current_status,
        'transition_reason', NEW.transition_reason,
        'remediation_state', NEW.remediation_state,
        'triggered_at', NEW.triggered_at
    );
    PERFORM pg_notify('runtime_vm_trust_transition', payload::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_runtime_vm_trust_history_notify ON runtime_vm_trust_history;
CREATE TRIGGER trg_runtime_vm_trust_history_notify
AFTER INSERT ON runtime_vm_trust_history
FOR EACH ROW
EXECUTE PROCEDURE notify_runtime_vm_trust_transition();

ALTER TABLE evaluation_certifications
    ADD COLUMN IF NOT EXISTS last_attestation_status TEXT,
    ADD COLUMN IF NOT EXISTS fallback_launched_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS remediation_attempts INTEGER NOT NULL DEFAULT 0;
