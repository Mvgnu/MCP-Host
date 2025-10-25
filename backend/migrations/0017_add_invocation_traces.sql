-- Track request/response pairs for evaluation
CREATE TABLE invocation_traces (
    id SERIAL PRIMARY KEY,
    server_id INTEGER NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    input_json JSONB NOT NULL,
    output_text TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_invocation_server ON invocation_traces(server_id);
