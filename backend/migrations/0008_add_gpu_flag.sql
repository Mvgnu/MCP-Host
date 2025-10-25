-- Allow servers to request GPU resources
ALTER TABLE mcp_servers ADD COLUMN use_gpu BOOLEAN NOT NULL DEFAULT FALSE;
