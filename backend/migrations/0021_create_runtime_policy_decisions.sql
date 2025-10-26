CREATE TABLE runtime_policy_decisions (
    id SERIAL PRIMARY KEY,
    server_id INTEGER NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    backend TEXT NOT NULL,
    image TEXT NOT NULL,
    requires_build BOOLEAN NOT NULL,
    artifact_run_id INTEGER REFERENCES build_artifact_runs(id) ON DELETE SET NULL,
    manifest_digest TEXT,
    policy_version TEXT NOT NULL,
    evaluation_required BOOLEAN NOT NULL DEFAULT FALSE,
    tier TEXT,
    health_overall TEXT,
    notes JSONB NOT NULL DEFAULT '[]'::jsonb,
    decided_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_runtime_policy_decisions_server_id
    ON runtime_policy_decisions(server_id);

CREATE INDEX idx_runtime_policy_decisions_decided_at
    ON runtime_policy_decisions(decided_at DESC);
