-- Add indexes for foreign key columns to improve query performance
CREATE INDEX IF NOT EXISTS idx_mcp_servers_owner_id ON mcp_servers(owner_id);
CREATE INDEX IF NOT EXISTS idx_context_sessions_server_id ON context_sessions(server_id);
CREATE INDEX IF NOT EXISTS idx_context_sessions_user_id ON context_sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_usage_metrics_server_id ON usage_metrics(server_id);
CREATE INDEX IF NOT EXISTS idx_service_integrations_server_id ON service_integrations(server_id);
CREATE INDEX IF NOT EXISTS idx_server_logs_server_id ON server_logs(server_id);
CREATE INDEX IF NOT EXISTS idx_custom_domains_server_id ON custom_domains(server_id);
CREATE INDEX IF NOT EXISTS idx_server_secrets_server_id ON server_secrets(server_id);
CREATE INDEX IF NOT EXISTS idx_server_files_server_id ON server_files(server_id);
CREATE INDEX IF NOT EXISTS idx_server_capabilities_server_id ON server_capabilities(server_id);
