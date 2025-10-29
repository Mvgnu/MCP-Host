-- BYOK provider key fabric scaffolding
-- key: migration-provider-keys

BEGIN;

CREATE TABLE IF NOT EXISTS provider_keys (
    id UUID PRIMARY KEY,
    provider_id UUID NOT NULL,
    alias TEXT,
    state TEXT NOT NULL,
    rotation_due_at TIMESTAMPTZ,
    attestation_digest TEXT,
    activated_at TIMESTAMPTZ,
    retired_at TIMESTAMPTZ,
    compromised_at TIMESTAMPTZ,
    version BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_provider_keys_provider ON provider_keys(provider_id);
CREATE INDEX IF NOT EXISTS idx_provider_keys_state ON provider_keys(state);

CREATE TABLE IF NOT EXISTS provider_key_rotations (
    id UUID PRIMARY KEY,
    provider_key_id UUID NOT NULL REFERENCES provider_keys(id) ON DELETE CASCADE,
    requested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    approved_at TIMESTAMPTZ,
    status TEXT NOT NULL,
    evidence_uri TEXT,
    request_actor_ref TEXT,
    approval_actor_ref TEXT,
    failure_reason TEXT,
    metadata JSONB NOT NULL DEFAULT '{}'::JSONB
);

CREATE INDEX IF NOT EXISTS idx_provider_key_rotations_key ON provider_key_rotations(provider_key_id);
CREATE INDEX IF NOT EXISTS idx_provider_key_rotations_status ON provider_key_rotations(status);

CREATE TABLE IF NOT EXISTS provider_key_bindings (
    id UUID PRIMARY KEY,
    provider_key_id UUID NOT NULL REFERENCES provider_keys(id) ON DELETE CASCADE,
    binding_type TEXT NOT NULL,
    binding_target_id UUID NOT NULL,
    binding_scope JSONB NOT NULL DEFAULT '{}'::JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ,
    revoked_reason TEXT,
    version BIGINT NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_provider_key_bindings_key ON provider_key_bindings(provider_key_id);
CREATE INDEX IF NOT EXISTS idx_provider_key_bindings_target ON provider_key_bindings(binding_target_id);

CREATE TABLE IF NOT EXISTS provider_key_audit_events (
    id UUID PRIMARY KEY,
    provider_id UUID NOT NULL,
    provider_key_id UUID,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::JSONB,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_provider_key_audit_provider ON provider_key_audit_events(provider_id);
CREATE INDEX IF NOT EXISTS idx_provider_key_audit_key ON provider_key_audit_events(provider_key_id);
CREATE INDEX IF NOT EXISTS idx_provider_key_audit_type ON provider_key_audit_events(event_type);

COMMIT;

-- Down

BEGIN;

DROP TABLE IF EXISTS provider_key_audit_events;
DROP TABLE IF EXISTS provider_key_bindings;
DROP TABLE IF EXISTS provider_key_rotations;
DROP TABLE IF EXISTS provider_keys;

COMMIT;
