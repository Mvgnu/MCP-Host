CREATE TABLE capability_intelligence_scores (
    id SERIAL PRIMARY KEY,
    server_id INTEGER NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    capability TEXT NOT NULL,
    backend TEXT,
    tier TEXT,
    score NUMERIC(5,2) NOT NULL,
    status TEXT NOT NULL,
    confidence NUMERIC(4,3) NOT NULL,
    last_observed_at TIMESTAMPTZ NOT NULL,
    notes JSONB NOT NULL DEFAULT '[]'::jsonb,
    evidence JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (server_id, capability, backend, tier)
);

CREATE INDEX idx_capability_intelligence_scores_server
    ON capability_intelligence_scores(server_id);

CREATE INDEX idx_capability_intelligence_scores_updated
    ON capability_intelligence_scores(updated_at DESC);

CREATE OR REPLACE FUNCTION update_capability_intelligence_scores_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_capability_intelligence_scores_updated
BEFORE UPDATE ON capability_intelligence_scores
FOR EACH ROW
EXECUTE FUNCTION update_capability_intelligence_scores_updated_at();
