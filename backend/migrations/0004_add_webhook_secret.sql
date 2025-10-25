-- Add webhook secret column to store deployment triggers
ALTER TABLE mcp_servers ADD COLUMN webhook_secret TEXT;
