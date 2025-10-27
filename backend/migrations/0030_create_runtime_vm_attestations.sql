-- key: migration -> runtime-vm-attestations
CREATE TABLE IF NOT EXISTS runtime_vm_attestations (
    id BIGSERIAL PRIMARY KEY,
    runtime_vm_instance_id BIGINT NOT NULL REFERENCES runtime_vm_instances(id) ON DELETE CASCADE,
    attestation_kind TEXT NOT NULL,
    verification_status TEXT NOT NULL,
    raw_quote BYTEA,
    parsed_claims JSONB,
    signer_metadata JSONB,
    freshness_expires_at TIMESTAMPTZ,
    verified_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    verification_notes TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    remediation_notes TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_runtime_vm_attestations_instance_id
    ON runtime_vm_attestations(runtime_vm_instance_id DESC, verified_at DESC);

CREATE INDEX IF NOT EXISTS idx_runtime_vm_attestations_status
    ON runtime_vm_attestations(verification_status);

DROP TRIGGER IF EXISTS trg_runtime_vm_attestations_updated_at ON runtime_vm_attestations;
CREATE TRIGGER trg_runtime_vm_attestations_updated_at
BEFORE UPDATE ON runtime_vm_attestations
FOR EACH ROW
EXECUTE PROCEDURE set_updated_at();
