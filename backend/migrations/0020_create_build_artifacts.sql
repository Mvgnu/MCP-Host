CREATE TABLE build_artifact_runs (
    id SERIAL PRIMARY KEY,
    server_id INTEGER NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    source_repo TEXT,
    source_branch TEXT,
    source_revision TEXT,
    registry TEXT,
    local_image TEXT NOT NULL,
    registry_image TEXT,
    manifest_tag TEXT,
    manifest_digest TEXT,
    started_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL,
    multi_arch BOOLEAN NOT NULL DEFAULT FALSE,
    auth_refresh_attempted BOOLEAN NOT NULL DEFAULT FALSE,
    auth_refresh_succeeded BOOLEAN NOT NULL DEFAULT FALSE,
    auth_rotation_attempted BOOLEAN NOT NULL DEFAULT FALSE,
    auth_rotation_succeeded BOOLEAN NOT NULL DEFAULT FALSE,
    credential_health_status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_build_artifact_runs_server_id ON build_artifact_runs(server_id);
CREATE INDEX idx_build_artifact_runs_status ON build_artifact_runs(status);

CREATE TABLE build_artifact_platforms (
    id SERIAL PRIMARY KEY,
    run_id INTEGER NOT NULL REFERENCES build_artifact_runs(id) ON DELETE CASCADE,
    platform TEXT NOT NULL,
    remote_image TEXT NOT NULL,
    remote_tag TEXT NOT NULL,
    digest TEXT,
    auth_refresh_attempted BOOLEAN NOT NULL DEFAULT FALSE,
    auth_refresh_succeeded BOOLEAN NOT NULL DEFAULT FALSE,
    auth_rotation_attempted BOOLEAN NOT NULL DEFAULT FALSE,
    auth_rotation_succeeded BOOLEAN NOT NULL DEFAULT FALSE,
    credential_health_status TEXT NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_build_artifact_platforms_run_id ON build_artifact_platforms(run_id);
CREATE INDEX idx_build_artifact_platforms_digest ON build_artifact_platforms(digest);
