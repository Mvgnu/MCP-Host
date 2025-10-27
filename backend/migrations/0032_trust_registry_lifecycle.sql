-- key: migration -> trust-registry-lifecycle
ALTER TABLE runtime_vm_trust_history
    ADD COLUMN IF NOT EXISTS previous_lifecycle_state TEXT,
    ADD COLUMN IF NOT EXISTS current_lifecycle_state TEXT,
    ADD COLUMN IF NOT EXISTS freshness_deadline TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS remediation_attempts INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS provenance_ref TEXT,
    ADD COLUMN IF NOT EXISTS provenance JSONB;

UPDATE runtime_vm_trust_history
SET current_lifecycle_state = CASE current_status
    WHEN 'trusted' THEN 'restored'
    WHEN 'untrusted' THEN 'quarantined'
    ELSE 'suspect'
END
WHERE current_lifecycle_state IS NULL;

ALTER TABLE runtime_vm_trust_history
    ALTER COLUMN current_lifecycle_state SET NOT NULL;

ALTER TABLE runtime_vm_trust_history
    ADD CONSTRAINT chk_runtime_vm_trust_history_lifecycle
    CHECK (current_lifecycle_state IN ('suspect', 'quarantined', 'remediating', 'restored'));

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
        'previous_lifecycle_state', NEW.previous_lifecycle_state,
        'current_lifecycle_state', NEW.current_lifecycle_state,
        'transition_reason', NEW.transition_reason,
        'remediation_state', NEW.remediation_state,
        'remediation_attempts', NEW.remediation_attempts,
        'freshness_deadline', NEW.freshness_deadline,
        'provenance_ref', NEW.provenance_ref,
        'provenance', NEW.provenance,
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

CREATE TABLE IF NOT EXISTS runtime_vm_trust_registry (
    runtime_vm_instance_id BIGINT PRIMARY KEY REFERENCES runtime_vm_instances(id) ON DELETE CASCADE,
    attestation_status TEXT NOT NULL,
    lifecycle_state TEXT NOT NULL,
    remediation_state TEXT,
    remediation_attempts INTEGER NOT NULL DEFAULT 0,
    freshness_deadline TIMESTAMPTZ,
    provenance_ref TEXT,
    provenance JSONB,
    version BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_runtime_vm_trust_registry_state
    ON runtime_vm_trust_registry(lifecycle_state);

CREATE OR REPLACE FUNCTION set_runtime_vm_trust_registry_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_runtime_vm_trust_registry_updated_at ON runtime_vm_trust_registry;
CREATE TRIGGER trg_runtime_vm_trust_registry_updated_at
BEFORE UPDATE ON runtime_vm_trust_registry
FOR EACH ROW
EXECUTE PROCEDURE set_runtime_vm_trust_registry_updated_at();

INSERT INTO runtime_vm_trust_registry (
    runtime_vm_instance_id,
    attestation_status,
    lifecycle_state,
    remediation_state,
    remediation_attempts,
    freshness_deadline,
    provenance_ref,
    provenance
)
SELECT DISTINCT ON (runtime_vm_instance_id)
    runtime_vm_instance_id,
    current_status,
    current_lifecycle_state,
    remediation_state,
    remediation_attempts,
    freshness_deadline,
    provenance_ref,
    provenance
FROM runtime_vm_trust_history
ORDER BY runtime_vm_instance_id, triggered_at DESC
ON CONFLICT (runtime_vm_instance_id) DO NOTHING;

ALTER TABLE runtime_vm_trust_registry
    ADD CONSTRAINT chk_runtime_vm_trust_registry_lifecycle
    CHECK (lifecycle_state IN ('suspect', 'quarantined', 'remediating', 'restored'));
