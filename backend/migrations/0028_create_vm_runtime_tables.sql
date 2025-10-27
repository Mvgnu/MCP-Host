-- key: migration -> runtime-vm-instances
CREATE TABLE IF NOT EXISTS runtime_vm_instances (
    id SERIAL PRIMARY KEY,
    server_id INTEGER NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    instance_id TEXT NOT NULL,
    isolation_tier TEXT,
    attestation_status TEXT NOT NULL DEFAULT 'pending',
    attestation_evidence JSONB,
    policy_version TEXT,
    capability_notes TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    terminated_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_runtime_vm_instances_server_id
    ON runtime_vm_instances(server_id);

CREATE TABLE IF NOT EXISTS runtime_vm_events (
    id BIGSERIAL PRIMARY KEY,
    vm_instance_id INTEGER NOT NULL REFERENCES runtime_vm_instances(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    event_payload JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_runtime_vm_events_instance_id
    ON runtime_vm_events(vm_instance_id);

-- Update trigger to maintain updated_at
CREATE OR REPLACE FUNCTION set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_runtime_vm_instances_updated_at ON runtime_vm_instances;
CREATE TRIGGER trg_runtime_vm_instances_updated_at
BEFORE UPDATE ON runtime_vm_instances
FOR EACH ROW
EXECUTE PROCEDURE set_updated_at();
