-- Store capabilities declared in an MCP manifest
CREATE TABLE server_capabilities (
    id SERIAL PRIMARY KEY,
    server_id INTEGER NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT
);
