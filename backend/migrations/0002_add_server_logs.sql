-- Store container logs for historical viewing
CREATE TABLE server_logs (
    id SERIAL PRIMARY KEY,
    server_id INTEGER NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    collected_at TIMESTAMPTZ DEFAULT NOW(),
    log_text TEXT NOT NULL
);
