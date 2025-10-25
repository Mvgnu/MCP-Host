-- Store MCP manifest returned by the container
ALTER TABLE mcp_servers ADD COLUMN manifest JSONB;
