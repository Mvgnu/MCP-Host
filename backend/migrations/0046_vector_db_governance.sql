-- Federated vector database governance scaffolding
-- key: migration-vector-db-governance

BEGIN;

CREATE TABLE IF NOT EXISTS vector_db_residency_policies (
    id SERIAL PRIMARY KEY,
    vector_db_id INTEGER NOT NULL REFERENCES vector_dbs(id) ON DELETE CASCADE,
    region TEXT NOT NULL,
    data_classification TEXT NOT NULL DEFAULT 'general',
    enforcement_mode TEXT NOT NULL DEFAULT 'monitor',
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_vector_db_residency_unique
    ON vector_db_residency_policies(vector_db_id, region);

CREATE INDEX IF NOT EXISTS idx_vector_db_residency_active
    ON vector_db_residency_policies(vector_db_id, active)
    WHERE active = TRUE;

CREATE TABLE IF NOT EXISTS vector_db_attachments (
    id UUID PRIMARY KEY,
    vector_db_id INTEGER NOT NULL REFERENCES vector_dbs(id) ON DELETE CASCADE,
    attachment_type TEXT NOT NULL,
    attachment_ref UUID NOT NULL,
    residency_policy_id INTEGER NOT NULL REFERENCES vector_db_residency_policies(id) ON DELETE RESTRICT,
    provider_key_binding_id UUID NOT NULL REFERENCES provider_key_bindings(id) ON DELETE RESTRICT,
    attached_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    detached_at TIMESTAMPTZ,
    detached_reason TEXT,
    metadata JSONB NOT NULL DEFAULT '{}'::JSONB
);

CREATE INDEX IF NOT EXISTS idx_vector_db_attachments_vector
    ON vector_db_attachments(vector_db_id);

CREATE INDEX IF NOT EXISTS idx_vector_db_attachments_binding
    ON vector_db_attachments(provider_key_binding_id)
    WHERE detached_at IS NULL;

CREATE TABLE IF NOT EXISTS vector_db_incident_logs (
    id UUID PRIMARY KEY,
    vector_db_id INTEGER NOT NULL REFERENCES vector_dbs(id) ON DELETE CASCADE,
    attachment_id UUID REFERENCES vector_db_attachments(id) ON DELETE SET NULL,
    incident_type TEXT NOT NULL,
    severity TEXT NOT NULL DEFAULT 'medium',
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at TIMESTAMPTZ,
    summary TEXT,
    notes JSONB NOT NULL DEFAULT '{}'::JSONB
);

CREATE INDEX IF NOT EXISTS idx_vector_db_incidents_vector
    ON vector_db_incident_logs(vector_db_id);

CREATE INDEX IF NOT EXISTS idx_vector_db_incidents_type
    ON vector_db_incident_logs(incident_type);

COMMIT;

-- Down

BEGIN;

DROP TABLE IF EXISTS vector_db_incident_logs;
DROP TABLE IF EXISTS vector_db_attachments;
DROP TABLE IF EXISTS vector_db_residency_policies;

COMMIT;
