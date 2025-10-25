-- Prebuilt service integrations table
CREATE TABLE service_integrations (
    id SERIAL PRIMARY KEY,
    server_id INTEGER NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    service_type TEXT NOT NULL,
    config JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
