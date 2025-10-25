-- Managed vector database instances
CREATE TABLE vector_dbs (
    id SERIAL PRIMARY KEY,
    owner_id INTEGER NOT NULL REFERENCES users(id),
    name TEXT NOT NULL,
    db_type TEXT NOT NULL,
    container_id TEXT,
    url TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_vector_dbs_owner_id ON vector_dbs(owner_id);

