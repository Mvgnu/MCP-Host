-- Data ingestion jobs for syncing content into vector databases
CREATE TABLE ingestion_jobs (
    id SERIAL PRIMARY KEY,
    owner_id INTEGER NOT NULL REFERENCES users(id),
    vector_db_id INTEGER NOT NULL REFERENCES vector_dbs(id),
    source_url TEXT NOT NULL,
    schedule_minutes INTEGER DEFAULT 0,
    last_run TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_ingestion_jobs_owner_id ON ingestion_jobs(owner_id);
