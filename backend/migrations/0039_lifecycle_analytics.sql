-- key: migration -> lifecycle-analytics
CREATE INDEX IF NOT EXISTS idx_build_artifact_runs_manifest_digest_completed
    ON build_artifact_runs(manifest_digest, completed_at DESC);

CREATE INDEX IF NOT EXISTS idx_build_artifact_runs_server_completed
    ON build_artifact_runs(server_id, completed_at DESC);
