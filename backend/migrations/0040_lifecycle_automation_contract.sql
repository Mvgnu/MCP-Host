-- key: migration -> lifecycle-automation-contract
ALTER TABLE runtime_vm_remediation_runs
    ADD COLUMN IF NOT EXISTS analytics_duration_ms BIGINT,
    ADD COLUMN IF NOT EXISTS analytics_execution_started_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS analytics_execution_completed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS analytics_retry_count INTEGER,
    ADD COLUMN IF NOT EXISTS analytics_retry_ledger JSONB,
    ADD COLUMN IF NOT EXISTS analytics_override_actor_id INTEGER REFERENCES users(id),
    ADD COLUMN IF NOT EXISTS analytics_artifact_hash TEXT,
    ADD COLUMN IF NOT EXISTS analytics_promotion_verdict_id BIGINT REFERENCES artifact_promotions(id);

CREATE INDEX IF NOT EXISTS idx_remediation_runs_analytics_promotion_verdict
    ON runtime_vm_remediation_runs(analytics_promotion_verdict_id);
